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
pub fn render_viewer(params: &ViewerParams) -> impl IntoElement {
    let content: AnyElement = match &params.path {
        Some(path) => img(path.clone())
            .id("viewer-image")
            .size_full()
            .object_fit(ObjectFit::Contain)
            .into_any(),
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
