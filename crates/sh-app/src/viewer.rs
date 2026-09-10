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
}

/// Build the viewer element tree from the current params.
///
/// Returns a [`div`] containing either the loaded image or an empty-state
/// placeholder. The image is displayed via GPUI's built-in `img()` element,
/// which handles file loading, decoding, and BGRA conversion internally.
///
/// When `nav_arrows` is `Some`, the provided element is rendered as an
/// overlay at the bottom (prev/next buttons wired by the caller).
pub fn render_viewer(params: &ViewerParams, nav_arrows: Option<AnyElement>) -> impl IntoElement {
    let content: AnyElement = match &params.path {
        Some(path) => {
            let image = img(path.clone())
                .id("viewer-image")
                .size_full()
                .object_fit(ObjectFit::Contain);

            if let Some(arrows) = nav_arrows {
                // Overlay nav bar at the bottom of the image.
                div()
                    .id("viewer-image-wrap")
                    .size_full()
                    .relative()
                    .child(image)
                    .child(arrows)
                    .into_any()
            } else {
                image.into_any()
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
