//! Screenshot fallback: ask the backend for a frame capture via
//! `ViewportCommand::Screenshot`, then poll for the resulting
//! `egui::Event::Screenshot` and encode it as PNG.
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
    let key = egui::Id::new("egui_agent::screenshot_stash");
    ctx.data_mut(|d| d.get_temp_mut_or_default::<Stash>(key).clone())
}

/// Backends deliver every captured screenshot into the raw input of whichever
/// viewport runs next (eframe's `handle_screenshots`), which with several
/// native windows is often not the viewport that asked. This plugin sees every
/// viewport's input and keeps the screenshots for [`poll`].
struct ScreenshotCatcher {
    stash: Stash,
}

impl egui::Plugin for ScreenshotCatcher {
    fn debug_name(&self) -> &'static str {
        "egui_agent::screenshot"
    }

    fn input_hook(&mut self, _ctx: &egui::Context, input: &mut egui::RawInput) {
        let mut found = Vec::new();
        for e in &input.events {
            if let egui::Event::Screenshot {
                viewport_id, image, ..
            } = e
            {
                found.push((*viewport_id, image.clone()));
            }
        }
        if !found.is_empty() {
            self.stash.lock().unwrap().extend(found);
        }
    }
}

fn ensure_installed(ctx: &egui::Context) {
    let key = egui::Id::new("egui_agent::screenshot_installed");
    if ctx.data(|d| d.get_temp::<bool>(key).unwrap_or(false)) {
        return;
    }
    ctx.add_plugin(ScreenshotCatcher { stash: stash(ctx) });
    ctx.data_mut(|d| d.insert_temp(key, true));
}

/// Ask the backend to capture the current viewport.
pub(crate) fn request(ctx: &egui::Context) {
    ensure_installed(ctx);
    let viewport = ctx.viewport_id();
    stash(ctx).lock().unwrap().retain(|(v, _)| *v != viewport);
    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
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

    #[test]
    fn root_screenshot_delivered_to_another_viewport_is_found() {
        let ctx = egui::Context::default();
        let root = egui::ViewportId::ROOT;
        let other = egui::ViewportId::from_hash_of("second window");
        ctx.run_ui(egui::RawInput::default(), |ui| request(ui.ctx()))
            .textures_delta
            .clear();
        // The backend routes the capture into the other window's next input.
        let image = Arc::new(egui::ColorImage::new([2, 2], vec![egui::Color32::RED; 4]));
        let mut input = egui::RawInput {
            viewport_id: other,
            events: vec![egui::Event::Screenshot {
                viewport_id: root,
                user_data: egui::UserData::default(),
                image,
            }],
            ..Default::default()
        };
        input.viewports.insert(other, egui::ViewportInfo::default());
        ctx.run_ui(input, |_| {}).textures_delta.clear();
        let mut got = None;
        ctx.run_ui(egui::RawInput::default(), |ui| got = poll(ui.ctx()))
            .textures_delta
            .clear();
        assert!(
            got.is_some(),
            "screenshot found despite landing in another viewport"
        );
        // Consumed once.
        let mut again = None;
        ctx.run_ui(egui::RawInput::default(), |ui| again = poll(ui.ctx()))
            .textures_delta
            .clear();
        assert!(again.is_none());
    }
}
