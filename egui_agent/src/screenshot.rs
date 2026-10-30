//! Screenshot fallback: ask the backend for a frame capture via
//! `Context::request_screenshot` (egui 0.37's callback form of
//! `ViewportCommand::Screenshot`), then poll for the delivered image and encode it as PNG.
//!
//! Requires a backend that honors the command (eframe's glow/wgpu backends
//! do; headless kittest without a renderer does not — the command then
//! resolves to `NotSupported` after a few frames).

use std::sync::{Arc, Mutex};

use base64::Engine as _;

use crate::protocol::{AgentError, Response, ResponseBody};

/// Captured screenshots seen in any viewport's input, keyed by the viewport
/// they were taken of.
type Stash = Arc<Mutex<Vec<(egui::ViewportId, Arc<egui::ColorImage>)>>>;

fn stash(ctx: &egui::Context) -> Stash {
    let key = egui::Id::unique("egui_agent::screenshot_stash");
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Stash>(key).clone())
}

/// Ask the backend to capture the current viewport. egui 0.37 delivers the capture to a
/// callback (no more `Event::Screenshot`), so the callback files it under the viewport that
/// asked, whichever viewport runs the next frame.
pub(crate) fn request(ctx: &egui::Context) {
    let viewport = ctx.viewport_id();
    let stash = stash(ctx);
    stash.lock().unwrap().retain(|(v, _)| *v != viewport);
    let sink = stash.clone();
    ctx.request_screenshot(move |image| {
        sink.lock().unwrap().push((viewport, image));
    });
}

/// Check for the delivered screenshot of the current viewport, wherever the
/// backend routed it.
pub(crate) fn poll(ctx: &egui::Context) -> Option<Response> {
    let viewport = ctx.viewport_id();
    let image = {
        let stash = stash(ctx);
        let mut st = stash.lock().unwrap();
        let pos = st.iter().position(|(v, _)| *v == viewport)?;
        st.remove(pos).1
    };
    Some(encode_png(&image))
}

fn encode_png(image: &egui::ColorImage) -> Response {
    let [w, h] = image.size;
    let mut rgba = Vec::with_capacity(w * h * 4);
    for px in &image.pixels {
        rgba.extend_from_slice(&px.to_array());
    }
    let mut png = Vec::new();
    let encoder = image::codecs::png::PngEncoder::new(&mut png);
    let result = image::ImageEncoder::write_image(
        encoder,
        &rgba,
        w as u32,
        h as u32,
        image::ExtendedColorType::Rgba8,
    );
    match result {
        Ok(()) => Ok(ResponseBody::Image {
            format: "png".into(),
            width: w as u32,
            height: h as u32,
            data: base64::engine::general_purpose::STANDARD.encode(&png),
        }),
        Err(e) => Err(AgentError::Internal(format!("png encode failed: {e}"))),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The backend answers a request through the callback; `poll` hands the image out once,
    /// for the viewport that asked, wherever the backend's frame ran.
    #[test]
    fn delivered_screenshot_is_polled_once() {
        let ctx = egui::Context::default();
        let viewport = ctx.viewport_id();
        ctx.run_ui(egui::RawInput::default(), |ui| request(ui.ctx())).textures_delta.clear();
        let image = Arc::new(egui::ColorImage::filled([2, 2], egui::Color32::RED));
        stash(&ctx).lock().unwrap().push((viewport, image));
        let mut got = None;
        ctx.run_ui(egui::RawInput::default(), |ui| got = poll(ui.ctx())).textures_delta.clear();
        assert!(got.is_some(), "the delivered screenshot is found");
        let mut again = None;
        ctx.run_ui(egui::RawInput::default(), |ui| again = poll(ui.ctx())).textures_delta.clear();
        assert!(again.is_none(), "consumed once");
    }

    #[test]
    fn a_request_clears_a_stale_capture() {
        let ctx = egui::Context::default();
        let viewport = ctx.viewport_id();
        stash(&ctx).lock().unwrap().push((viewport, Arc::new(egui::ColorImage::filled([1, 1], egui::Color32::BLUE))));
        ctx.run_ui(egui::RawInput::default(), |ui| request(ui.ctx())).textures_delta.clear();
        let mut got = None;
        ctx.run_ui(egui::RawInput::default(), |ui| got = poll(ui.ctx())).textures_delta.clear();
        assert!(got.is_none(), "the stale capture went with the new request");
    }
}
