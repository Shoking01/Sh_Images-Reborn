//! Mutable session state for the viewer UI.

use sh_core::navigation::{ImageEntry, MetaView, SortBy, SortDir};
use sh_core::transform::{self, Vec2, ZoomState};
use std::path::PathBuf;
use std::time::SystemTime;

/// How zoom/pan currently behaves.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum FitMode {
    /// Fit the entire image within the viewport (default).
    #[default]
    Fit,
    /// Display at native 100% resolution.
    Percent100,
}

/// Relative snap band above the fit floor inside which the session snaps
/// back to fit.
///
/// Pinned from the proven wheel-path literal it replaces: post-clamp
/// `scale >= floor` always holds, so the band only ever catches the floor
/// itself plus float dust from wheel multipliers (a float-exact floor
/// comparison would never fire). The value governs EVERY zoom entry point
/// through [`Session::clamp_zoom`] — wheel and presets alike — so there is
/// exactly one floor rule.
pub const FIT_SNAP_REL_EPS: f32 = 1e-4;

/// Exactly the four viewer zoom presets. 100% = actual pixels.
///
/// Click-only UI surface: activation goes through
/// [`Session::set_zoom_preset`], never through direct scale assignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ZoomPreset {
    /// Fit the image to the viewport (existing fit semantics).
    Fit,
    /// Half size (scale 0.5).
    Scale50,
    /// Actual pixels (scale 1.0).
    Scale100,
    /// Double size (scale 2.0).
    Scale200,
}

impl ZoomPreset {
    /// Target scale, or `None` for [`ZoomPreset::Fit`].
    pub fn scale(self) -> Option<f32> {
        match self {
            ZoomPreset::Fit => None,
            ZoomPreset::Scale50 => Some(0.5),
            ZoomPreset::Scale100 => Some(1.0),
            ZoomPreset::Scale200 => Some(2.0),
        }
    }
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
    /// File size in bytes (from the scan entry).
    pub size: u64,
    /// Modified time (from the scan entry).
    pub modified: SystemTime,
    /// Creation time where the OS reports it; `None` on Linux.
    pub created: Option<SystemTime>,
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
    /// Whether the bottom info overlay is visible.
    pub show_overlay_bottom: bool,
    /// Slideshow auto-advance active (V3). Transient, NEVER persisted:
    /// a folder swap or entering crop resets it.
    pub slideshow_active: bool,
    /// Active sort criterion — the session is the runtime source of truth
    /// for order (settings only persist it; see the V3 sort-engine spec).
    pub sort_by: SortBy,
    /// Active sort direction.
    pub sort_dir: SortDir,
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

    /// Re-sort `images` in memory under the active `sort_by`/`sort_dir`,
    /// keeping the selection on the same image (re-anchored by path).
    ///
    /// Zero I/O: items are sorted in place through [`MetaView`] borrowed
    /// views of their metadata, so probed `dimensions` and `name` survive
    /// per path (an entry round-trip would drop them).
    pub fn resort(&mut self) {
        let anchor = self.images.get(self.current).map(|i| i.path.clone());
        self.images.sort_by(|a, b| {
            let a = MetaView {
                path: &a.path,
                size: a.size,
                modified: a.modified,
                created: a.created,
            };
            let b = MetaView {
                path: &b.path,
                size: b.size,
                modified: b.modified,
                created: b.created,
            };
            sh_core::navigation::compare_meta(&a, &b, self.sort_by, self.sort_dir)
        });
        self.current = anchor
            .and_then(|a| self.images.iter().position(|i| i.path == a))
            .unwrap_or(0);
    }

    /// Set criterion + direction and resort in one step (the single
    /// call-site API for sort changes).
    pub fn apply_sort(&mut self, by: SortBy, dir: SortDir) {
        self.sort_by = by;
        self.sort_dir = dir;
        self.resort();
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
    ///
    /// No-op unless in [`FitMode::Percent100`]: a fitted image is fully
    /// visible, so panning would only displace it off the background.
    pub fn pan(&mut self, delta: Vec2) {
        if self.fit_mode != FitMode::Percent100 {
            return;
        }
        self.zoom = transform::pan(self.zoom, delta);
    }

    /// Recompute fit zoom for a new viewport size.
    ///
    /// No-op unless in [`FitMode::Fit`] and image dimensions are known.
    /// Called from `App::render` when the viewport changes (e.g. fullscreen
    /// toggle); `Percent100` user zoom is intentionally left alone on resize.
    pub fn refit_for_viewport(&mut self, viewport: Vec2) {
        if self.fit_mode != FitMode::Fit {
            return;
        }
        let Some(img_size) = self.current_image_size() else {
            return;
        };
        self.zoom = transform::fit(img_size, viewport);
    }

    /// Activate a zoom preset, centered on the viewport midpoint.
    ///
    /// Fit resolves through the existing fit path (`transform::fit` +
    /// [`FitMode::Fit`] — byte-equal to `toggle_fit_100`'s Percent100→Fit
    /// arm for the same inputs). Scale presets anchor [`Session::zoom_at`]
    /// at the viewport center with a multiplicative delta and then run
    /// [`Session::clamp_zoom`], so the `zoom_at` → `clamp_zoom` funnel and
    /// the [`FIT_SNAP_REL_EPS`] floor rule hold for every preset — no scale
    /// is ever assigned directly. No-op when image dimensions are unknown.
    pub fn set_zoom_preset(&mut self, preset: ZoomPreset, viewport: Vec2) {
        let Some(img_size) = self.current_image_size() else {
            return;
        };
        let Some(target) = preset.scale() else {
            self.zoom = transform::fit(img_size, viewport);
            self.fit_mode = FitMode::Fit;
            return;
        };
        // Fresh sessions carry scale 0.0 and `zoom_at` divides by the
        // current scale — seed from fit first (the same computation the Fit
        // arm uses), so the delta is always well-formed.
        if self.zoom.scale <= 0.0 {
            self.zoom = transform::fit(img_size, viewport);
        }
        let center = Vec2 {
            x: viewport.x / 2.0,
            y: viewport.y / 2.0,
        };
        let delta = target / self.zoom.scale;
        self.zoom_at(center, delta);
        self.clamp_zoom(img_size, viewport);
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
    ///
    /// Snaps back to [`FitMode::Fit`] when the clamped scale lands on the fit
    /// floor: without this, zooming out with the wheel leaves the session at
    /// fit scale but stuck in `Percent100`, which wrongly enables pan and
    /// disables viewport-change refit.
    pub fn clamp_zoom(&mut self, image_size: Vec2, viewport: Vec2) {
        self.zoom = transform::clamp_scale(self.zoom, image_size, viewport);
        let floor = transform::fit_scale(image_size.x, image_size.y, viewport.x, viewport.y);
        // Post-clamp `scale >= floor` always holds, so this only catches the
        // floor (plus float dust from the wheel multiplier) — never real zoom.
        if self.zoom.scale <= floor * (1.0 + FIT_SNAP_REL_EPS) {
            self.zoom = transform::fit(image_size, viewport);
            self.fit_mode = FitMode::Fit;
        }
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

/// Build image items from scanned entries (name extracted, metadata kept).
pub fn build_image_items(entries: impl IntoIterator<Item = ImageEntry>) -> Vec<ImageItem> {
    entries
        .into_iter()
        .map(|e| ImageItem {
            name: e
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("image")
                .to_string(),
            path: e.path,
            dimensions: None,
            size: e.size,
            modified: e.modified,
            created: e.created,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use sh_core::navigation::{SortBy, SortDir};
    use std::time::{Duration, SystemTime};

    /// Item factory for sort tests — no filesystem involved.
    fn sort_item(path: &str, size: u64, modified: SystemTime) -> ImageItem {
        ImageItem {
            path: PathBuf::from(path),
            name: path.rsplit('/').next().unwrap_or(path).to_string(),
            dimensions: None,
            size,
            modified,
            created: None,
        }
    }

    const EPOCH: SystemTime = SystemTime::UNIX_EPOCH;

    impl ImageItem {
        /// Test convenience: set probed dimensions on a factory-built item.
        fn with_dimensions(mut self, w: u32, h: u32) -> Self {
            self.dimensions = Some((w, h));
            self
        }
    }

    #[test]
    fn resort_reanchors_current_by_path() {
        let mut s = Session {
            images: vec![sort_item("b.png", 0, EPOCH), sort_item("a.png", 0, EPOCH)],
            current: 0, // b.png selected
            sort_by: SortBy::Name,
            sort_dir: SortDir::Asc,
            ..Default::default()
        };
        s.resort();
        // Name order: a, b. Selection was b.png → now index 1, same image.
        assert_eq!(s.images[0].path, PathBuf::from("a.png"));
        assert_eq!(s.images[1].path, PathBuf::from("b.png"));
        assert_eq!(s.current, 1);
        assert_eq!(s.images[s.current].path, PathBuf::from("b.png"));
    }

    #[test]
    fn apply_sort_updates_fields_and_reanchors() {
        let t1 = EPOCH;
        let t2 = EPOCH + Duration::from_secs(10);
        let mut s = Session {
            images: vec![sort_item("new.png", 0, t2), sort_item("old.png", 0, t1)],
            current: 0, // new.png
            ..Default::default()
        };
        s.apply_sort(SortBy::Modified, SortDir::Asc);
        assert_eq!(s.sort_by, SortBy::Modified);
        assert_eq!(s.sort_dir, SortDir::Asc);
        assert_eq!(s.images[0].path, PathBuf::from("old.png"));
        assert_eq!(s.current, 1);
        assert_eq!(s.images[s.current].path, PathBuf::from("new.png"));
    }

    #[test]
    fn resort_preserves_probed_dimensions_per_path() {
        // Dimensions are probed async and feed fit/zoom math — resort must
        // never drop them (the entry round-trip trap).
        let mut s = Session {
            images: vec![
                ImageItem {
                    path: PathBuf::from("b.png"),
                    name: "b.png".into(),
                    dimensions: Some((1920, 1080)),
                    size: 0,
                    modified: EPOCH,
                    created: None,
                },
                ImageItem {
                    path: PathBuf::from("a.png"),
                    name: "a.png".into(),
                    dimensions: None,
                    size: 0,
                    modified: EPOCH,
                    created: None,
                },
            ],
            current: 0,
            sort_by: SortBy::Name,
            sort_dir: SortDir::Asc,
            ..Default::default()
        };
        s.resort();
        assert_eq!(s.images[0].path, PathBuf::from("a.png"));
        assert_eq!(s.images[0].dimensions, None);
        assert_eq!(s.images[1].dimensions, Some((1920, 1080)));
    }

    #[test]
    fn resort_empty_session_is_noop() {
        let mut s = Session::default();
        s.apply_sort(SortBy::Size, SortDir::Desc);
        assert!(s.images.is_empty());
        assert_eq!(s.current, 0);
    }

    #[test]
    fn resort_missing_anchor_clamps_to_zero() {
        // Defensive: the anchor path vanished from the list mid-folder.
        let mut s = Session {
            images: vec![sort_item("a.png", 0, EPOCH)],
            current: 0,
            sort_by: SortBy::Name,
            sort_dir: SortDir::Desc,
            ..Default::default()
        };
        // Force an impossible anchor: current beyond the list.
        s.current = 5;
        s.resort();
        assert_eq!(s.current, 0);
    }

    #[test]
    fn position_label_empty() {
        let s = Session::default();
        assert_eq!(s.position_label(), "");
    }

    #[test]
    fn position_label_with_images() {
        let s = Session {
            images: vec![
                sort_item("a.png", 0, EPOCH),
                sort_item("b.png", 0, EPOCH),
                sort_item("c.png", 0, EPOCH),
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
            ImageEntry {
                path: PathBuf::from("/photos/cat.png"),
                size: 10,
                modified: EPOCH,
                created: Some(EPOCH),
            },
            ImageEntry {
                path: PathBuf::from("/photos/dog.jpg"),
                size: 20,
                modified: EPOCH,
                created: None,
            },
        ]);
        assert_eq!(items.len(), 2);
        assert_eq!(items[0].name, "cat.png");
        assert_eq!(items[1].name, "dog.jpg");
        assert!(items[0].dimensions.is_none());
        assert_eq!(items[0].size, 10);
        assert_eq!(items[1].created, None);
    }

    #[test]
    fn build_image_items_fallback_for_no_name() {
        // Path ending in separator yields no file_name → falls back to "image".
        let items = build_image_items(vec![ImageEntry {
            path: PathBuf::from("/"),
            size: 0,
            modified: EPOCH,
            created: None,
        }]);
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
            images: vec![sort_item("img.png", 0, EPOCH).with_dimensions(w, h)],
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
    fn pan_in_fit_mode_is_noop() {
        // A fitted image is fully visible — panning must not displace it.
        let fit = transform::fit(
            Vec2 {
                x: 1000.0,
                y: 500.0,
            },
            Vec2 { x: 800.0, y: 600.0 },
        );
        let mut s = zoom_session(1000, 500, fit, FitMode::Fit);
        s.pan(Vec2 { x: 50.0, y: -30.0 });
        assert_eq!(s.zoom, fit);
    }

    #[test]
    fn refit_for_viewport_recenters_for_new_viewport() {
        // Fit computed for 1280x720 must recenter when the window becomes
        // 1920x1080 (fullscreen), instead of rendering off-center.
        let old_view = Vec2 {
            x: 1280.0,
            y: 720.0,
        };
        let new_view = Vec2 {
            x: 1920.0,
            y: 1080.0,
        };
        let img = Vec2 {
            x: 1000.0,
            y: 500.0,
        };
        let mut s = zoom_session(1000, 500, transform::fit(img, old_view), FitMode::Fit);
        s.refit_for_viewport(new_view);
        let expected = transform::fit(img, new_view);
        assert!((s.zoom.scale - expected.scale).abs() < 1e-5);
        assert!((s.zoom.offset.x - expected.offset.x).abs() < 1e-4);
        assert!((s.zoom.offset.y - expected.offset.y).abs() < 1e-4);
    }

    #[test]
    fn refit_for_viewport_noop_in_percent100() {
        // User zoom is intentionally left alone on resize.
        let st = ZoomState {
            scale: 1.0,
            offset: Vec2 { x: -50.0, y: 25.0 },
        };
        let mut s = zoom_session(1000, 500, st, FitMode::Percent100);
        s.refit_for_viewport(Vec2 {
            x: 1920.0,
            y: 1080.0,
        });
        assert_eq!(s.zoom, st);
    }

    #[test]
    fn refit_for_viewport_noop_without_dimensions() {
        let mut s = Session {
            images: vec![sort_item("x.png", 0, EPOCH)],
            zoom: ZoomState {
                scale: 0.8,
                offset: Vec2 { x: 1.0, y: 2.0 },
            },
            fit_mode: FitMode::Fit,
            ..Default::default()
        };
        s.refit_for_viewport(Vec2 {
            x: 1920.0,
            y: 1080.0,
        });
        assert_eq!(s.zoom.scale, 0.8);
        assert_eq!(s.zoom.offset.x, 1.0);
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
            images: vec![sort_item("x.png", 0, EPOCH)],
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
        assert_eq!(s2.fit_mode, FitMode::Percent100);
    }

    #[test]
    fn clamp_zoom_at_floor_snaps_back_to_fit() {
        // Regression: wheel zoom-out left the session at fit scale but stuck
        // in Percent100, wrongly enabling pan and disabling resize refit.
        let img = Vec2 {
            x: 1000.0,
            y: 500.0,
        };
        let viewport = Vec2 { x: 800.0, y: 600.0 };
        // Floor for this geometry is 0.8; land just under it like a wheel-out.
        let mut s = zoom_session(
            1000,
            500,
            ZoomState {
                scale: 0.5,
                offset: Vec2 { x: -30.0, y: 12.0 },
            },
            FitMode::Percent100,
        );
        s.clamp_zoom(img, viewport);
        assert_eq!(s.fit_mode, FitMode::Fit);
        let expected = sh_core::transform::fit(img, viewport);
        assert!((s.zoom.scale - expected.scale).abs() < 1e-5);
        assert!((s.zoom.offset.x - expected.offset.x).abs() < 1e-4);
        assert!((s.zoom.offset.y - expected.offset.y).abs() < 1e-4);
    }

    #[test]
    fn clamp_zoom_above_floor_keeps_percent100() {
        let viewport = Vec2 { x: 800.0, y: 600.0 };
        let mut s = zoom_session(
            1000,
            500,
            ZoomState {
                scale: 2.0,
                offset: Vec2 { x: 10.0, y: -5.0 },
            },
            FitMode::Percent100,
        );
        s.clamp_zoom(
            Vec2 {
                x: 1000.0,
                y: 500.0,
            },
            viewport,
        );
        assert_eq!(s.fit_mode, FitMode::Percent100);
        assert_eq!(s.zoom.scale, 2.0);
    }

    // ── Zoom presets (viewer-zoom-presets, Phase 3) ──

    const VIEW: Vec2 = Vec2 { x: 800.0, y: 600.0 };
    /// 4:2 image: fit floor 0.2 in VIEW → every preset scale above floor.
    const IMG: Vec2 = Vec2 {
        x: 4000.0,
        y: 2000.0,
    };

    #[test]
    fn zoom_preset_scale_yields_the_exact_four_variants() {
        assert_eq!(ZoomPreset::Fit.scale(), None);
        assert_eq!(ZoomPreset::Scale50.scale(), Some(0.5));
        assert_eq!(ZoomPreset::Scale100.scale(), Some(1.0));
        assert_eq!(ZoomPreset::Scale200.scale(), Some(2.0));
        // Exactly four variants: a fifth changes this list, not the enum.
        assert_eq!(
            [
                ZoomPreset::Fit,
                ZoomPreset::Scale50,
                ZoomPreset::Scale100,
                ZoomPreset::Scale200,
            ]
            .len(),
            4
        );
    }

    #[test]
    fn set_zoom_preset_lands_above_floor_scales_exactly() {
        for (preset, target) in [
            (ZoomPreset::Scale50, 0.5f32),
            (ZoomPreset::Scale100, 1.0),
            (ZoomPreset::Scale200, 2.0),
        ] {
            let mut s = zoom_session(
                IMG.x as u32,
                IMG.y as u32,
                transform::fit(IMG, VIEW),
                FitMode::Fit,
            );
            s.set_zoom_preset(preset, VIEW);
            assert!(
                (s.zoom.scale - target).abs() < 1e-5,
                "preset {preset:?} landed at {}",
                s.zoom.scale
            );
            assert_eq!(s.fit_mode, FitMode::Percent100);
        }
    }

    #[test]
    fn set_zoom_preset_centers_on_the_viewport_midpoint() {
        // Content at the viewport center stays at the viewport center.
        let mut s = zoom_session(
            IMG.x as u32,
            IMG.y as u32,
            transform::fit(IMG, VIEW),
            FitMode::Fit,
        );
        s.set_zoom_preset(ZoomPreset::Scale100, VIEW);
        // Centered anchor: the image point under the midpoint before == after.
        let fit_state = transform::fit(IMG, VIEW);
        let center = Vec2 {
            x: VIEW.x / 2.0,
            y: VIEW.y / 2.0,
        };
        let before_x = (center.x - fit_state.offset.x) / fit_state.scale;
        let after_x = (center.x - s.zoom.offset.x) / s.zoom.scale;
        let before_y = (center.y - fit_state.offset.y) / fit_state.scale;
        let after_y = (center.y - s.zoom.offset.y) / s.zoom.scale;
        assert!((before_x - after_x).abs() < 1e-2);
        assert!((before_y - after_y).abs() < 1e-2);
    }

    #[test]
    fn set_zoom_preset_recenters_small_image_at_clamped_scale() {
        // 100x100 image in 800x600: fit floor = min(8, 6) = 6.0 (below the
        // MAX_SCALE cap), so a 0.5 request clamps up. `clamp_scale` lifts it
        // to the floor AND re-centers; because the result sits exactly on
        // the floor, the epsilon predicate then swaps in the fit state —
        // whose geometry IS the centered-at-clamped-scale state. Net effect:
        // the image ends centered at 6.0 in FitMode::Fit, never drifting at
        // a stale offset.
        let img = Vec2 { x: 100.0, y: 100.0 };
        let mut s = zoom_session(100, 100, transform::fit(img, VIEW), FitMode::Fit);
        s.set_zoom_preset(ZoomPreset::Scale50, VIEW);
        let expected = transform::fit(img, VIEW);
        assert!((s.zoom.scale - expected.scale).abs() < 1e-5);
        assert!((s.zoom.offset.x - expected.offset.x).abs() < 1e-4);
        assert!((s.zoom.offset.y - expected.offset.y).abs() < 1e-4);
        assert_eq!(s.fit_mode, FitMode::Fit);
    }

    #[test]
    fn set_zoom_preset_sub_floor_snaps_back_to_fit() {
        // 2160x2160 image in a 1080x1080 viewport: fit floor is 0.5 EXACTLY,
        // so the 50% request sits inside the snap band and must land on the
        // fit state in FitMode::Fit — never stuck at 0.5 in Percent100.
        let viewport = Vec2 {
            x: 1080.0,
            y: 1080.0,
        };
        let img = Vec2 {
            x: 2160.0,
            y: 2160.0,
        };
        let mut s = zoom_session(2160, 2160, transform::fit(img, viewport), FitMode::Fit);
        s.set_zoom_preset(ZoomPreset::Scale50, viewport);
        assert_eq!(s.fit_mode, FitMode::Fit);
        let expected = transform::fit(img, viewport);
        assert!((s.zoom.scale - expected.scale).abs() < 1e-5);
        assert!((s.zoom.offset.x - expected.offset.x).abs() < 1e-4);
    }

    #[test]
    fn set_zoom_preset_fit_equals_the_toggle_fit_arm() {
        // The Fit chip must land on the exact same state as the existing
        // toggle_fit_100 Percent100→Fit arm for the same inputs.
        let start = transform::fit(IMG, VIEW);
        let mut via_chip = zoom_session(IMG.x as u32, IMG.y as u32, start, FitMode::Percent100);
        via_chip.set_zoom_preset(ZoomPreset::Fit, VIEW);
        let mut via_toggle = zoom_session(IMG.x as u32, IMG.y as u32, start, FitMode::Percent100);
        via_toggle.toggle_fit_100(VIEW);
        assert_eq!(via_chip.zoom, via_toggle.zoom);
        assert_eq!(via_chip.fit_mode, FitMode::Fit);
        assert_eq!(via_toggle.fit_mode, FitMode::Fit);
    }

    #[test]
    fn set_zoom_preset_without_dimensions_is_noop() {
        let mut s = Session {
            images: vec![sort_item("x.png", 0, EPOCH)],
            zoom: ZoomState {
                scale: 0.8,
                offset: Vec2 { x: 1.0, y: 2.0 },
            },
            fit_mode: FitMode::Fit,
            ..Default::default()
        };
        s.set_zoom_preset(ZoomPreset::Scale100, VIEW);
        assert_eq!(s.zoom.scale, 0.8);
        assert_eq!(s.zoom.offset.x, 1.0);
        assert_eq!(s.fit_mode, FitMode::Fit);
    }

    #[test]
    fn set_zoom_preset_seeds_degenerate_start_scale_from_fit() {
        // A fresh Session::default() carries scale 0.0; without the seed the
        // multiplicative delta would divide by zero. After the seed the
        // preset still lands exactly.
        let mut s = Session {
            images: vec![sort_item("img.png", 0, EPOCH).with_dimensions(4000, 2000)],
            ..Default::default()
        };
        assert_eq!(s.zoom.scale, 0.0);
        s.set_zoom_preset(ZoomPreset::Scale100, VIEW);
        assert!((s.zoom.scale - 1.0).abs() < 1e-5);
        assert_eq!(s.fit_mode, FitMode::Percent100);
    }

    #[test]
    fn fit_snap_rel_eps_boundary_holds_on_both_sides() {
        // Just-above-epsilon above the floor: scale/mode kept.
        // At/below epsilon: snap to fit. Pins the named constant's contract.
        let mut above = zoom_session(
            1000,
            500,
            ZoomState {
                scale: 0.8 * (1.0 + FIT_SNAP_REL_EPS * 4.0),
                offset: Vec2 { x: 0.0, y: 0.0 },
            },
            FitMode::Percent100,
        );
        above.clamp_zoom(
            Vec2 {
                x: 1000.0,
                y: 500.0,
            },
            VIEW,
        );
        assert_eq!(above.fit_mode, FitMode::Percent100);

        let mut at = zoom_session(
            1000,
            500,
            ZoomState {
                scale: 0.8 * (1.0 + FIT_SNAP_REL_EPS),
                offset: Vec2 { x: 0.0, y: 0.0 },
            },
            FitMode::Percent100,
        );
        at.clamp_zoom(
            Vec2 {
                x: 1000.0,
                y: 500.0,
            },
            VIEW,
        );
        assert_eq!(at.fit_mode, FitMode::Fit);
    }
}
