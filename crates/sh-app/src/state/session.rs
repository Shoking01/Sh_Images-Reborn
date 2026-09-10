//! Mutable session state for the viewer UI.

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
    /// Pixel dimensions from the file header, if probed.
    ///
    /// Dimensions only — full pixel decodes are never stored here. Rendering
    /// goes through GPUI's `img()` asset pipeline, and fit/zoom math needs
    /// nothing more than `(width, height)`, so retaining RGBA buffers would
    /// burn ~26MB per 4K image for data nobody reads.
    pub dimensions: Option<(u32, u32)>,
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

    /// Pixel dimensions of the current image, if probed.
    pub fn current_dimensions(&self) -> Option<(u32, u32)> {
        self.current_item().and_then(|i| i.dimensions)
    }

    /// Position label like `"4/23"`.
    pub fn position_label(&self) -> String {
        if self.images.is_empty() {
            String::new()
        } else {
            format!("{}/{}", self.current + 1, self.images.len())
        }
    }

    /// Current image size as a transform [`Vec2`], if dimensions are known.
    pub fn current_image_size(&self) -> Option<Vec2> {
        self.current_dimensions().map(|(w, h)| Vec2 {
            x: w as f32,
            y: h as f32,
        })
    }

    /// Zoom anchored at a cursor position in viewport pixels.
    ///
    /// Delegates to [`transform::zoom_at`], whose contract keeps the image
    /// point under the cursor fixed; the scale multiplier is applied and
    /// clamped there. Switches to `Percent100` mode (manual zoom overrides fit).
    pub fn zoom_at(&mut self, cursor: Vec2, delta: f32) {
        self.zoom = transform::zoom_at(self.zoom, cursor, delta);
        self.fit_mode = FitMode::Percent100;
    }

    /// Pan by a viewport delta.
    pub fn pan(&mut self, delta: Vec2) {
        self.zoom = transform::pan(self.zoom, delta);
    }

    /// Toggle between 100% and fit (double-click).
    ///
    /// With no known dimensions the toggle is a no-op.
    pub fn toggle_fit_100(&mut self, viewport: Vec2) {
        let Some(img_size) = self.current_image_size() else {
            return;
        };
        self.zoom = match self.fit_mode {
            FitMode::Fit => ZoomState {
                scale: 1.0,
                offset: Vec2 {
                    x: (viewport.x - img_size.x) / 2.0,
                    y: (viewport.y - img_size.y) / 2.0,
                },
            },
            FitMode::Percent100 => transform::fit(img_size, viewport),
        };
        self.fit_mode = match self.fit_mode {
            FitMode::Fit => FitMode::Percent100,
            FitMode::Percent100 => FitMode::Fit,
        };
    }

    /// Clamp zoom into allowed scale bounds.
    pub fn clamp_zoom(&mut self, image_size: Vec2, viewport: Vec2) {
        self.zoom = transform::clamp_scale(self.zoom, image_size, viewport);
    }
}

/// Returns the circular index `delta` steps from `current` within `len` items.
///
/// Returns `None` when `len == 0`.
pub fn next_index(current: usize, delta: isize, len: usize) -> Option<usize> {
    if len == 0 {
        return None;
    }
    Some((current as isize + delta).rem_euclid(len as isize) as usize)
}

/// Build image items from a resolved path list.
pub fn build_image_items(paths: impl IntoIterator<Item = PathBuf>) -> Vec<ImageItem> {
    paths
        .into_iter()
        .map(|path| ImageItem {
            name: path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("image")
                .to_string(),
            path,
            dimensions: None,
        })
        .collect()
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
                    dimensions: None,
                    name: "a.png".into(),
                },
                ImageItem {
                    path: PathBuf::from("b.png"),
                    dimensions: None,
                    name: "b.png".into(),
                },
                ImageItem {
                    path: PathBuf::from("c.png"),
                    dimensions: None,
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

    #[test]
    fn build_image_items_extracts_names() {
        let items = build_image_items(vec![
            PathBuf::from("/photos/cat.png"),
            PathBuf::from("/photos/dog.jpg"),
        ]);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "cat.png");
        assert_eq!(items[1].name, "dog.jpg");
        assert!(items[0].dimensions.is_none());
    }

    #[test]
    fn build_image_items_fallback_for_no_name() {
        // Path ending in separator yields no file_name → falls back to "image".
        let items = build_image_items(vec![PathBuf::from("/")]);
        assert_eq!(items[0].name, "image");
    }

    #[test]
    fn next_index_wraps_forward() {
        // Last index + 1 wraps to 0.
        assert_eq!(next_index(4, 1, 5), Some(0));
        // Middle forward stays in range.
        assert_eq!(next_index(2, 1, 5), Some(3));
    }

    #[test]
    fn next_index_wraps_backward() {
        // 0 − 1 wraps to the last index.
        assert_eq!(next_index(0, -1, 5), Some(4));
        assert_eq!(next_index(3, -1, 5), Some(2));
    }

    #[test]
    fn next_index_single_item_stays_on_itself() {
        assert_eq!(next_index(0, 1, 1), Some(0));
        assert_eq!(next_index(0, -1, 1), Some(0));
    }

    #[test]
    fn next_index_empty_returns_none() {
        assert_eq!(next_index(0, 1, 0), None);
    }

    // ── zoom/pan/fit handler tests (B1) ──

    /// Session with one image of known dimensions and a given zoom state.
    fn zoom_session(w: u32, h: u32, zoom: ZoomState, fit_mode: FitMode) -> Session {
        Session {
            images: vec![ImageItem {
                path: PathBuf::from("img.png"),
                dimensions: Some((w, h)),
                name: "img.png".into(),
            }],
            zoom,
            fit_mode,
            ..Default::default()
        }
    }

    #[test]
    fn zoom_at_delegates_and_sets_percent100() {
        // 1000x500 image, viewport 800x600 → fit scale 0.8.
        let fit = transform::fit(
            Vec2 {
                x: 1000.0,
                y: 500.0,
            },
            Vec2 { x: 800.0, y: 600.0 },
        );
        let mut s = zoom_session(1000, 500, fit, FitMode::Fit);

        // Cursor in viewport pixels (transform::zoom_at's contract).
        let cursor = Vec2 { x: 400.0, y: 300.0 };
        s.zoom_at(cursor, 2.0);
        assert_eq!(s.fit_mode, FitMode::Percent100);
        assert!((s.zoom.scale - fit.scale * 2.0).abs() < 1e-5);
        // Anchored: the image point under the cursor stays fixed.
        let before = (cursor.x - fit.offset.x) / fit.scale;
        let after = (cursor.x - s.zoom.offset.x) / s.zoom.scale;
        assert!((before - after).abs() < 1e-2);
    }

    #[test]
    fn pan_moves_offset() {
        let st = ZoomState {
            scale: 2.0,
            offset: Vec2 { x: 10.0, y: -4.0 },
        };
        let mut s = zoom_session(100, 100, st, FitMode::Percent100);
        s.pan(Vec2 { x: 5.0, y: 7.0 });
        assert_eq!(s.zoom.offset.x, 15.0);
        assert_eq!(s.zoom.offset.y, 3.0);
        assert_eq!(s.zoom.scale, 2.0);
    }

    #[test]
    fn toggle_fit_100_from_fit_sets_100_centered() {
        let viewport = Vec2 { x: 800.0, y: 600.0 };
        let fit = transform::fit(
            Vec2 {
                x: 1000.0,
                y: 500.0,
            },
            viewport,
        );
        let mut s = zoom_session(1000, 500, fit, FitMode::Fit);

        s.toggle_fit_100(viewport);
        assert_eq!(s.fit_mode, FitMode::Percent100);
        assert_eq!(s.zoom.scale, 1.0);
        // Centered: offset = (viewport - image) / 2.
        assert_eq!(s.zoom.offset.x, (800.0 - 1000.0) / 2.0);
        assert_eq!(s.zoom.offset.y, (600.0 - 500.0) / 2.0);
    }

    #[test]
    fn toggle_fit_100_from_percent100_returns_to_fit() {
        let viewport = Vec2 { x: 800.0, y: 600.0 };
        // Start at 100% with an off-center offset.
        let st = ZoomState {
            scale: 1.0,
            offset: Vec2 { x: -50.0, y: 25.0 },
        };
        let mut s = zoom_session(1000, 500, st, FitMode::Percent100);

        s.toggle_fit_100(viewport);
        assert_eq!(s.fit_mode, FitMode::Fit);
        let expected = transform::fit(
            Vec2 {
                x: 1000.0,
                y: 500.0,
            },
            viewport,
        );
        assert!((s.zoom.scale - expected.scale).abs() < 1e-5);
        assert!((s.zoom.offset.x - expected.offset.x).abs() < 1e-4);
        assert!((s.zoom.offset.y - expected.offset.y).abs() < 1e-4);
    }

    #[test]
    fn toggle_fit_100_without_dimensions_is_noop() {
        let mut s = Session {
            images: vec![ImageItem {
                path: PathBuf::from("x.png"),
                dimensions: None,
                name: "x.png".into(),
            }],
            zoom: ZoomState {
                scale: 0.8,
                offset: Vec2 { x: 1.0, y: 2.0 },
            },
            fit_mode: FitMode::Fit,
            ..Default::default()
        };
        s.toggle_fit_100(Vec2 { x: 800.0, y: 600.0 });
        assert_eq!(s.zoom.scale, 0.8);
        assert_eq!(s.fit_mode, FitMode::Fit);
    }

    #[test]
    fn clamp_zoom_clamps_into_bounds() {
        let img = Vec2 {
            x: 1000.0,
            y: 500.0,
        };
        let viewport = Vec2 { x: 800.0, y: 600.0 };
        // Scale above MAX_SCALE must clamp down and re-center.
        let mut s = zoom_session(
            1000,
            500,
            ZoomState {
                scale: 100.0,
                offset: Vec2 { x: 0.0, y: 0.0 },
            },
            FitMode::Percent100,
        );
        s.clamp_zoom(img, viewport);
        assert!(s.zoom.scale <= sh_core::transform::MAX_SCALE + 1e-5);
        // No-op when already in range.
        let ok = ZoomState {
            scale: 2.0,
            offset: Vec2 { x: 10.0, y: -5.0 },
        };
        let mut s2 = zoom_session(1000, 500, ok, FitMode::Percent100);
        s2.clamp_zoom(img, viewport);
        assert_eq!(s2.zoom.scale, 2.0);
        assert_eq!(s2.zoom.offset.x, 10.0);
        assert_eq!(s2.zoom.offset.y, -5.0);
    }
}
