//! The image viewer: renders the single image at session zoom/pan.

use crate::app::parse_hex;
use gpui::prelude::*;
use gpui::*;
use std::path::PathBuf;

/// Params the App hands to the viewer each frame.
#[derive(Debug, Clone)]
pub struct ViewerParams {
    /// Path to the current image file, if any.
    pub path: Option<PathBuf>,
    /// Position label like "3/12".
    pub position: String,
    /// Error message, if any.
    pub error: Option<String>,
    /// Zoom level text like "100%".
    pub zoom_text: String,
    /// Whether the top info overlay is visible.
    pub show_overlay_top: bool,
    /// Whether the bottom info overlay is visible.
    pub show_overlay_bottom: bool,
    /// Current zoom scale (1.0 = 100%).
    pub zoom_scale: f32,
    /// Current pan offset in viewport pixels (negative when centered).
    pub pan_offset: sh_core::transform::Vec2,
    /// Probed image dimensions in pixels, if known.
    pub decoded_size: Option<(f32, f32)>,
}

/// Build the viewer element tree from the current params.
///
/// Returns a [`div`] containing either the loaded image or an empty-state
/// placeholder. The image is displayed via GPUI's built-in `img()` element,
/// which handles file loading, decoding, and BGRA conversion internally.
///
/// Sizing is explicit layout (Task 8): the image is positioned absolutely at
/// `pan_offset` and sized to `dimensions * zoom_scale`, replacing the earlier
/// `object_fit(Contain)` approach so zoom/pan math fully controls the frame.
/// The element's aspect always equals the image's aspect
/// (`w = iw * scale`, `h = ih * scale`), so no `object_fit` is needed.
///
/// When `nav_arrows` is `Some`, the provided element is rendered as an
/// overlay at the bottom (prev/next buttons wired by the caller).
pub fn render_viewer(params: &ViewerParams, nav_arrows: Option<AnyElement>) -> impl IntoElement {
    let content: AnyElement = match &params.path {
        Some(path) => {
            // NOTE: while dimensions are unknown (the one tick before the
            // header probe lands), fall back to 1x1 — a tiny artifact for a
            // frame. The probe is header-only, so this is near-instant.
            let (iw, ih) = params.decoded_size.unwrap_or((1.0, 1.0));
            let image = img(path.clone())
                .id("viewer-image")
                .absolute()
                .left(px(params.pan_offset.x))
                .top(px(params.pan_offset.y))
                .w(px(iw * params.zoom_scale))
                .h(px(ih * params.zoom_scale));

            let layer = div().id("zoom-layer").size_full().relative().child(image);
            if let Some(arrows) = nav_arrows {
                layer.child(arrows).into_any()
            } else {
                layer.into_any()
            }
        }
        None => {
            let text_color: Hsla = parse_hex("#e8e8ee").unwrap_or(rgb(0xe8e8ee).into());
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
    };

    div().id("viewer-root").size_full().child(content)
}
