//! Mutable session state for the viewer UI.

use sh_core::decode::DecodedImage;
use sh_core::transform::{self, Vec2, ZoomState};
use std::path::PathBuf;

/// How zoom/pan currently behaves.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FitMode {
    /// Fit the entire image within the viewport (default).
    #[default]
    Fit,
    /// Display at native 100% resolution.
    Percent100,
}

/// One image in the session.
#[derive(Debug, Clone)]
pub struct ImageItem {
    /// Absolute filesystem path to the image.
    pub path: PathBuf,
    /// Decoded pixel data, if loaded.
    pub decoded: Option<DecodedImage>,
    /// Display name (file name).
    pub name: String,
}

/// Session state for the open set of images.
#[derive(Debug, Default)]
pub struct Session {
    /// All images in the current folder.
    pub images: Vec<ImageItem>,
    /// Index of the currently displayed image.
    pub current: usize,
    /// Current zoom/pan state.
    pub zoom: ZoomState,
    /// How the image fits the viewport.
    pub fit_mode: FitMode,
    /// Last error message, if any.
    pub error: Option<String>,
    /// Whether the top info overlay is visible.
    pub show_overlay_top: bool,
    /// Whether the bottom info overlay is visible.
    pub show_overlay_bottom: bool,
}

impl Session {
    /// Current image item, if any.
    pub fn current_item(&self) -> Option<&ImageItem> {
        self.images.get(self.current)
    }

    /// Current decoded image, if any.
    pub fn current_decoded(&self) -> Option<&DecodedImage> {
        self.current_item().and_then(|i| i.decoded.as_ref())
    }

    /// Position label like `"4/23"`.
    pub fn position_label(&self) -> String {
        if self.images.is_empty() {
            String::new()
        } else {
            format!("{}/{}", self.current + 1, self.images.len())
        }
    }

    /// Compute a fit zoom for the current image in the given viewport.
    pub fn fit_zoom(&self, viewport: Vec2) -> ZoomState {
        match self.current_decoded() {
            Some(img) => transform::fit(
                Vec2 {
                    x: img.width as f32,
                    y: img.height as f32,
                },
                viewport,
            ),
            None => ZoomState {
                scale: 1.0,
                offset: Vec2 { x: 0.0, y: 0.0 },
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn position_label_empty() {
        let s = Session::default();
        assert_eq!(s.position_label(), "");
    }

    #[test]
    fn position_label_with_images() {
        let s = Session {
            images: vec![
                ImageItem {
                    path: PathBuf::from("a.png"),
                    decoded: None,
                    name: "a.png".into(),
                },
                ImageItem {
                    path: PathBuf::from("b.png"),
                    decoded: None,
                    name: "b.png".into(),
                },
                ImageItem {
                    path: PathBuf::from("c.png"),
                    decoded: None,
                    name: "c.png".into(),
                },
            ],
            current: 1,
            ..Default::default()
        };
        assert_eq!(s.position_label(), "2/3");
    }

    #[test]
    fn fit_mode_default_is_fit() {
        assert_eq!(FitMode::default(), FitMode::Fit);
    }

    #[test]
    fn session_default_has_zero_images() {
        let s = Session::default();
        assert!(s.images.is_empty());
        assert_eq!(s.current, 0);
        assert_eq!(s.zoom.scale, 0.0);
        assert_eq!(s.fit_mode, FitMode::Fit);
        assert!(s.error.is_none());
    }
}
