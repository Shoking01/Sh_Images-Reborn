//! Viewer filmstrip: bottom-docked thumbnail navigator.
//!
//! The strip is layout chrome, never overlay chrome: a fixed-height
//! in-flow row below the image area showing a windowed range of cached
//! thumbnails with click-to-navigate. Slice A established the height
//! constant consumed by the viewport carve
//! ([`crate::app::stable_filmstrip_viewport`]); Slice B adds the window
//! helper, geometry consts, mount predicate, borrowed params, and the
//! cell builder with click wiring.

use gpui::prelude::*;
use gpui::*;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::state::session::ImageItem;

/// Fixed strip height in px: layout chrome, never content-sized.
///
/// Subtracted by [`crate::app::stable_filmstrip_viewport`] and enforced in
/// layout by `#filmstrip.h(px(STRIP_H_PX))`. Same discipline as
/// `TOPBAR_H_PX` / `BOTTOM_BAR_H_PX`: fixed constants keep carve
/// arithmetic exact instead of depending on measured content height.
pub const STRIP_H_PX: f32 = 104.0;

/// Half-window radius: cells are built for `current ± FILMSTRIP_WINDOW`,
/// clamped at folder boundaries — at most 49 cells per frame. 49 cells of
/// ~72px cover a 1920px strip (~25 visible) plus prefetch buffer, so
/// arrow-key navigation never shows an unbuilt cell.
pub const FILMSTRIP_WINDOW: usize = 24;

/// Strip cell width in px. Fixed (never content-sized): the carve math in
/// [`crate::app::stable_filmstrip_viewport`] subtracts [`STRIP_H_PX`] only,
/// so cell geometry must fit inside the strip by construction — see
/// `strip_geometry_fits_strip_height`. Grid `GridSize` S/M/L geometry
/// (120–240px cells) is not reused and stays untouched.
pub const STRIP_CELL_PX: f32 = 72.0;

/// Strip thumbnail square in px. 64px at 2x HiDPI = 128 device px, under
/// [`crate::thumbs::THUMB_MAX_DIM`] (256): the strip reuses resident
/// thumbnails with zero decode changes.
pub const STRIP_THUMB_PX: f32 = 64.0;

/// Gap between strip cells in px (also the strip's horizontal padding).
pub const STRIP_GAP_PX: f32 = 8.0;

/// Pure window: the index range rendered around `current`, clamped at
/// folder boundaries. `lo = current.saturating_sub(FILMSTRIP_WINDOW)`,
/// `hi = (current + FILMSTRIP_WINDOW + 1).min(len)`.
///
/// Empty folder yields an empty range. Never wraps, never pads past the
/// folder. There is no strip scroll state: the window is recomputed from
/// `session.current` every frame, so follow-navigate re-centering is the
/// absence of state, not an update step. `current` is always inside the
/// returned range (for `len > 0`).
pub fn filmstrip_window(current: usize, len: usize) -> std::ops::Range<usize> {
    if len == 0 {
        return 0..0;
    }
    let current = current.min(len - 1);
    let lo = current.saturating_sub(FILMSTRIP_WINDOW);
    let hi = current
        .saturating_add(FILMSTRIP_WINDOW)
        .saturating_add(1)
        .min(len);
    lo..hi
}

/// Width of the positioned cell row, including the gap on both sides.
///
/// The row owns the horizontal padding so its absolute left position can be
/// derived from the same geometry used by the cells.
pub fn filmstrip_row_width(cell_count: usize) -> f32 {
    if cell_count == 0 {
        return 0.0;
    }
    cell_count as f32 * STRIP_CELL_PX + (cell_count as f32 + 1.0) * STRIP_GAP_PX
}

/// Left position for the positioned row that centers the active cell.
///
/// The window remains `current ± FILMSTRIP_WINDOW`; this only chooses where
/// that already-built row is placed inside the viewport. Centering the active
/// cell rather than the whole row keeps it visible at both folder boundaries.
pub fn filmstrip_row_offset(viewport_width: f32, current: usize, len: usize) -> f32 {
    if len == 0 {
        return 0.0;
    }
    let current = current.min(len - 1);
    let window = filmstrip_window(current, len);
    let local = (current - window.start) as f32;
    viewport_width.max(0.0) / 2.0
        - STRIP_GAP_PX
        - local * (STRIP_CELL_PX + STRIP_GAP_PX)
        - STRIP_CELL_PX / 2.0
}

/// Mount predicate: the strip renders iff the current view is the viewer
/// AND the persisted `filmstrip` setting is ON (Req 1). Tab and idle state
/// cannot reach this predicate — they gate overlay chrome only — so Tab
/// can never hide the strip and idling can never fade it.
pub fn should_mount_filmstrip(is_viewer: bool, setting_on: bool) -> bool {
    is_viewer && setting_on
}

/// Borrowed strip inputs, built per frame in `App::render`.
///
/// A sidecar beside [`crate::viewer::ViewerParams`] (which gains zero
/// fields): the viewer stays pure-presentational over owned values while
/// the strip borrows the resident thumb maps read-only. No pixel buffers
/// are stored or copied — cells hold `Arc` clones of already-resident
/// thumbnails, and misses render a neutral placeholder (never a decode).
pub struct FilmstripParams<'a> {
    /// Anchor: `session.current`. The active marker and window derive here.
    pub current: usize,
    /// Current viewport width in logical pixels, used to center the active cell.
    pub viewport_width: f32,
    /// Session images (paths + cached verdicts live alongside).
    pub images: &'a [ImageItem],
    /// Resident decoded thumbnails by path (populated by `spawn_thumb_batch`).
    pub thumbs: &'a HashMap<PathBuf, Arc<RenderImage>>,
    /// Cached per-thumb alpha verdicts, keyed exactly like `thumbs`.
    pub thumb_alpha: &'a HashMap<PathBuf, bool>,
    /// Persisted checkerboard visibility setting.
    pub checkerboard_on: bool,
    /// Transparency-board palette, resolved once per frame by `App::render`.
    pub checker_palette: crate::checkerboard::BoardPalette,
    /// Active-marker color (theme accent, resolved by the caller).
    pub accent: Hsla,
    /// Neutral placeholder fill (theme surface, resolved by the caller).
    pub surface: Hsla,
}

/// Build the strip row for the ±24 window around `params.current`.
///
/// Cell contract (grid-cell idiom minus labels and modifier branches):
/// `thumbs.get(path)` hit renders `img(arc.clone())` (zero new decodes);
/// miss renders a neutral `surface`-fill placeholder (never a broken-image
/// treatment); the checkerboard layer appears iff
/// [`crate::viewer::should_show_checkerboard`] on the cached verdict
/// (verdict-pending `None` renders no board for one tick, exactly like the
/// grid); the active marker renders iff `idx == current` (single source of
/// truth, computed per frame — it cannot desync and follows click
/// navigation automatically).
///
/// Clicks swallow mousedown (strip clicks never arm the root pan gesture)
/// and route through the existing seq-guarded
/// `navigate(idx − current)` path — probe, prefetch, and `grid_selected`
/// sync come for free with zero new async code. No modifier branches (Req
/// 7: click-to-navigate only, no multi-select, no focus model). Reads
/// resident maps only: no filesystem I/O, no new threads, nothing blocks
/// the frame loop.
pub fn render_filmstrip(
    params: &FilmstripParams<'_>,
    cx: &mut Context<crate::app::App>,
) -> AnyElement {
    let window = filmstrip_window(params.current, params.images.len());
    let row_offset =
        filmstrip_row_offset(params.viewport_width, params.current, params.images.len());
    let row_width = filmstrip_row_width(window.len());
    let mut cells: Vec<AnyElement> = Vec::with_capacity(window.len());
    for idx in window {
        let path = &params.images[idx].path;
        let hit = params.thumbs.contains_key(path);
        // Thumb or neutral placeholder: a missing decode (batch still
        // running, slow/corrupt file) shows the themed surface chip until
        // the background batch lands — never a broken-image treatment.
        let thumb: AnyElement = match params.thumbs.get(path) {
            Some(arc) => img(arc.clone())
                .id(("strip-thumb", idx))
                .w(px(STRIP_THUMB_PX))
                .h(px(STRIP_THUMB_PX))
                .rounded(px(8.0))
                .into_any(),
            None => div()
                .id(("strip-thumb-empty", idx))
                .w(px(STRIP_THUMB_PX))
                .h(px(STRIP_THUMB_PX))
                .bg(params.surface)
                .rounded(px(8.0))
                .into_any(),
        };
        // Checkerboard gate identical to the grid cell: setting ON plus a
        // CONFIRMED transparent verdict from the cached map.
        let show_board = crate::viewer::should_show_checkerboard(
            params.checkerboard_on,
            params.thumb_alpha.get(path).copied(),
        );
        let mut thumb_frame = div().relative();
        if show_board {
            thumb_frame = thumb_frame.child(
                crate::checkerboard::checkerboard_layer(
                    STRIP_THUMB_PX,
                    STRIP_THUMB_PX,
                    params.checker_palette,
                )
                .absolute()
                .top(px(0.0))
                .left(px(0.0))
                .debug_selector(move || format!("strip-board-{idx}")),
            );
        }
        thumb_frame = thumb_frame.child(thumb);
        // Active marker: slim accent bar on the thumb's bottom edge,
        // echoing `grid-active-bar` geometry. Exactly one cell matches
        // per frame (`idx == current`).
        if idx == params.current {
            thumb_frame = thumb_frame.child(
                div()
                    .id(("strip-active-bar", idx))
                    .absolute()
                    .bottom(px(0.0))
                    .left(px(4.0))
                    .w(px(STRIP_THUMB_PX - 8.0))
                    .h(px(3.0))
                    .rounded(px(2.0))
                    .bg(params.accent)
                    .debug_selector(move || format!("strip-active-{idx}")),
            );
        }
        let swallow_cell = cx.listener(
            |_this: &mut crate::app::App, _ev: &MouseDownEvent, _window, cx| {
                cx.stop_propagation();
            },
        );
        let kind = if hit { "thumb" } else { "empty" };
        let cell = div()
            .id(("strip-cell", idx))
            .w(px(STRIP_CELL_PX))
            .flex_none()
            .cursor_pointer()
            .flex()
            .items_center()
            .justify_center()
            .rounded(px(8.0))
            .debug_selector(move || format!("strip-{kind}-{idx}"))
            .child(thumb_frame)
            .on_mouse_down(MouseButton::Left, swallow_cell)
            .on_click(cx.listener(
                move |this: &mut crate::app::App, _ev: &ClickEvent, _window, cx| {
                    this.note_interaction(cx);
                    this.navigate(idx as isize - this.session.current as isize, cx);
                },
            ));
        cells.push(cell.into_any());
    }
    div()
        .id("filmstrip")
        .w_full()
        .h(px(STRIP_H_PX))
        .relative()
        .overflow_hidden()
        .debug_selector(|| "filmstrip".to_string())
        .child(
            div()
                .absolute()
                .left(px(row_offset))
                .top(px(0.0))
                .w(px(row_width))
                .h(px(STRIP_H_PX))
                .flex()
                .flex_row()
                .items_center()
                .gap(px(STRIP_GAP_PX))
                .px(px(STRIP_GAP_PX))
                .children(cells),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::{
        filmstrip_row_offset, filmstrip_row_width, filmstrip_window, should_mount_filmstrip,
        FILMSTRIP_WINDOW, STRIP_CELL_PX, STRIP_GAP_PX, STRIP_H_PX, STRIP_THUMB_PX,
    };

    /// R2.1: centered window — current 50 in a large folder yields
    /// 26..75 (indices 26 through 74 inclusive, 49 cells max).
    #[test]
    fn window_centers_current_in_large_folder() {
        assert_eq!(FILMSTRIP_WINDOW, 24);
        assert_eq!(filmstrip_window(50, 100), 26..75);
    }

    /// R2.2: out-of-window indices render nothing — 200 images at
    /// current 100 yields exactly 76..125 (no cell below 76/above 124).
    #[test]
    fn window_excludes_out_of_window_indices() {
        let w = filmstrip_window(100, 200);
        assert_eq!(w, 76..125);
        assert!(!w.contains(&75));
        assert!(!w.contains(&125));
    }

    /// R2.3: the window follows navigation with no strip scroll state —
    /// moving 10 → 11 re-centers to 0..36 (indices 0 through 35, clamped).
    #[test]
    fn window_follows_navigation_with_clamp() {
        assert_eq!(filmstrip_window(10, 200), 0..35);
        assert_eq!(filmstrip_window(11, 200), 0..36);
    }

    /// R2.4: folder boundaries clamp — 10 images at 0 yield 0..10
    /// (indices 0 through 9, no wraparound, no padding past the folder).
    #[test]
    fn window_clamps_at_folder_boundaries() {
        assert_eq!(filmstrip_window(0, 10), 0..10);
        assert_eq!(filmstrip_window(9, 10), 0..10);
    }

    /// R2: empty folder yields an empty range (the strip mounts no cells).
    #[test]
    fn window_is_empty_for_empty_folder() {
        assert!(filmstrip_window(0, 0).is_empty());
    }

    /// R2: `current` is always inside its own window, across starts,
    /// ends, clamps, and single-image folders.
    #[test]
    fn window_always_contains_current() {
        for len in [1usize, 2, 10, 49, 50, 100, 200] {
            for current in 0..len {
                let w = filmstrip_window(current, len);
                assert!(
                    w.contains(&current),
                    "current={current} must be inside {w:?} (len={len})"
                );
                assert!(w.len() <= 2 * FILMSTRIP_WINDOW + 1);
            }
        }
    }

    #[test]
    fn row_geometry_centers_current_in_a_large_folder() {
        assert!((filmstrip_row_width(49) - 3928.0).abs() < f32::EPSILON);
        assert!((filmstrip_row_offset(1000.0, 50, 100) + 1464.0).abs() < f32::EPSILON);
    }

    #[test]
    fn row_offset_centers_current_at_folder_boundaries() {
        assert!((filmstrip_row_offset(1000.0, 0, 100) - 456.0).abs() < f32::EPSILON);
        assert!((filmstrip_row_offset(1000.0, 99, 100) + 1464.0).abs() < f32::EPSILON);
        assert!((filmstrip_row_offset(200.0, 0, 100) - 56.0).abs() < f32::EPSILON);
    }

    /// Cell geometry fits the fixed strip height by construction: the
    /// 64px thumb plus vertical padding (one gap each side) stays under
    /// 104px, and the 72px cell clears the thumb. Pins the design
    /// contract so a future geometry edit fails here, not in layout.
    /// The fit relations are const-evaluable, so they live in `const`
    /// blocks (compile-time pins); the value pins stay runtime asserts.
    #[test]
    fn strip_geometry_fits_strip_height() {
        assert!((STRIP_H_PX - 104.0).abs() < f32::EPSILON);
        assert!((STRIP_CELL_PX - 72.0).abs() < f32::EPSILON);
        assert!((STRIP_THUMB_PX - 64.0).abs() < f32::EPSILON);
        assert!((STRIP_GAP_PX - 8.0).abs() < f32::EPSILON);
        const {
            assert!(STRIP_THUMB_PX + 2.0 * STRIP_GAP_PX <= STRIP_H_PX);
        }
        const {
            assert!(STRIP_CELL_PX >= STRIP_THUMB_PX);
        }
    }

    /// R1 mount predicate: the strip mounts iff the view is Viewer AND
    /// the persisted setting is ON. Grid/Welcome/Settings never mount,
    /// whatever the flag.
    #[test]
    fn mount_predicate_is_viewer_and_setting_only() {
        assert!(should_mount_filmstrip(true, true));
        assert!(!should_mount_filmstrip(true, false));
        assert!(!should_mount_filmstrip(false, true));
        assert!(!should_mount_filmstrip(false, false));
    }

    /// R3.3/R3.4: the strip reuses the grid's board gate verbatim
    /// (`should_show_checkerboard`): board iff the setting is ON plus a
    /// CONFIRMED transparent verdict. `None` (verdict in flight) renders
    /// no board for one tick — exactly like the grid placeholder arm.
    #[test]
    fn strip_board_gate_matches_grid() {
        use crate::viewer::should_show_checkerboard;
        assert!(should_show_checkerboard(true, Some(true)));
        assert!(!should_show_checkerboard(true, Some(false)));
        assert!(!should_show_checkerboard(true, None));
        assert!(!should_show_checkerboard(false, Some(true)));
        assert!(!should_show_checkerboard(false, Some(false)));
        assert!(!should_show_checkerboard(false, None));
    }
}
