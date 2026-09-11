//! The image viewer: renders the single image at session zoom/pan.

use gpui::prelude::*;
use gpui::*;
use std::path::PathBuf;

/// Params the App hands to the viewer each frame.
#[derive(Debug, Clone)]
pub struct ViewerParams {
    /// Path to the current image file, if any.
    pub path: Option<PathBuf>,
    /// Error message, if any (rendered in the empty-state arm).
    pub error: Option<String>,
    /// Current zoom scale (1.0 = 100%).
    pub zoom_scale: f32,
    /// Current pan offset in viewport pixels (negative when centered).
    pub pan_offset: sh_core::transform::Vec2,
    /// Probed image dimensions in pixels, if known.
    pub decoded_size: Option<(f32, f32)>,
}

/// Build the viewer element tree from the current params.
///
/// Renders the error state FIRST (a failed probe of the current image shows
/// the message), else the loaded image, else the empty-state placeholder.
/// The image is displayed via GPUI's built-in `img()` element, which handles
/// file loading, decoding, and BGRA conversion internally.
///
/// Sizing is explicit layout (Task 8): the image is positioned absolutely at
/// `pan_offset` and sized to `dimensions * zoom_scale`, so zoom/pan math
/// fully controls the frame. The element's aspect always equals the image's
/// aspect (`w = iw * scale`, `h = ih * scale`), so no `object_fit` is needed.
///
/// Task 9 moved the info overlays (top name/position, bottom zoom/arrows)
/// OUT of the viewer — they render as app-level children over this layer.
pub fn render_viewer(params: &ViewerParams) -> impl IntoElement {
    // Error state FIRST: a failed probe of the CURRENT image must show the
    // message, not a 1×1 artifact — session.error is set while the path is
    // still Some. (Error clears on the next successful navigate.)
    let content: AnyElement = if let Some(error) = &params.error {
        let text_color: Hsla = crate::app::parse_hex("#e8e8ee").unwrap_or(rgb(0xe8e8ee).into());
        div()
            .id("viewer-error")
            .size_full()
            .flex()
            .items_center()
            .justify_center()
            .child(div().text_color(text_color).child(error.clone()))
            .into_any()
    } else {
        match &params.path {
            Some(path) => {
                // NOTE: while dimensions are unknown (the one tick before the
                // header probe lands), fall back to 1x1 — a tiny artifact for
                // a frame. The probe is header-only, so this is near-instant.
                let (iw, ih) = params.decoded_size.unwrap_or((1.0, 1.0));
                let image = img(path.clone())
                    .id("viewer-image")
                    .absolute()
                    .left(px(params.pan_offset.x))
                    .top(px(params.pan_offset.y))
                    .w(px(iw * params.zoom_scale))
                    .h(px(ih * params.zoom_scale));

                div()
                    .id("zoom-layer")
                    .size_full()
                    .relative()
                    .child(image)
                    .into_any()
            }
            None => {
                let text_color: Hsla =
                    crate::app::parse_hex("#e8e8ee").unwrap_or(rgb(0xe8e8ee).into());
                div()
                    .id("viewer-empty")
                    .size_full()
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        div()
                            .text_color(text_color)
                            .child("Drop an image to open it"),
                    )
                    .into_any()
            }
        }
    };

    div().id("viewer-root").size_full().child(content)
}
