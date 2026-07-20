//! Screenshot fallback: ask the backend for a frame capture via
//! `ViewportCommand::Screenshot`, then poll for the resulting
//! `egui::Event::Screenshot` and encode it as PNG.
//!
//! Requires a backend that honors the command (eframe's glow/wgpu backends
//! do; headless kittest without a renderer does not — the command then
//! resolves to `NotSupported` after a few frames).

use base64::Engine as _;

use crate::protocol::{AgentError, Response, ResponseBody};

/// Ask the backend to capture the current viewport.
pub(crate) fn request(ctx: &egui::Context) {
    ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot(egui::UserData::default()));
}

/// Check this frame's input for the delivered screenshot event.
pub(crate) fn poll(ctx: &egui::Context) -> Option<Response> {
    let image = ctx.input(|i| {
        i.raw.events.iter().find_map(|e| match e {
            egui::Event::Screenshot { image, .. } => Some(image.clone()),
            _ => None,
        })
    })?;
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
