//! Folder grid: thumbnail cells with selection + manual wheel scroll.
//!
//! Scroll note: gpui 0.2.2 exposes no scrollable plain `div` (only
//! `overflow_hidden` and the virtualized `uniform_list`, whose
//! `Fn(Range, &mut Window, &mut App)` row callback receives no entity
//! `Context` — cell clicks could not reach `App` through it). The grid is
//! therefore a `flex_wrap` row whose inner offset (`-scroll_px`, driven by
//! the root wheel handler in Grid view) is clamped by [`grid_max_scroll`].
//! All geometry here is pure and unit-tested; rendering stays dumb.

use gpui::prelude::*;
use gpui::*;

/// Cell footprint width; columns = floor(viewport_w / this).
pub const GRID_CELL_PX: f32 = 180.0;

/// Uniform row height (thumb 120px + single-line label + padding).
/// Labels MUST stay single-line (ellipsis) at the call site — a wrapped
/// label grows the row and silently breaks [`grid_max_scroll`].
pub const GRID_ROW_H_PX: f32 = 170.0;

/// Row/column gap — must match the `gap()` in [`grid`].
pub const GRID_GAP_PX: f32 = 8.0;

/// Container padding (each side) — must match the `p()` in [`grid`].
pub const GRID_PAD_PX: f32 = 12.0;

/// Column count for a viewport width: fixed 180px cells, at least one.
pub fn grid_columns(viewport_w: f32) -> usize {
    ((viewport_w / GRID_CELL_PX).floor() as usize).max(1)
}

/// Clamp a selection index into a list length (sticky at ends, no wrap).
pub fn clamp_selection(sel: usize, len: usize) -> usize {
    if len == 0 {
        0
    } else {
        sel.min(len - 1)
    }
}

/// Inclusive index range between anchor and target, ascending — the union
/// operand for Shift+click / Shift+arrows. Direction-agnostic by design.
pub fn selection_range(a: usize, b: usize) -> Vec<usize> {
    (a.min(b)..=a.max(b)).collect()
}

/// Max scroll offset for `len` items in a viewport: content height minus
/// visible height, floored at zero (nothing to scroll). Content accounts
/// rows + inter-row gaps + vertical padding — forgetting either strands the
/// last rows out of reach in windowed sizes.
pub fn grid_max_scroll(len: usize, viewport_w: f32, viewport_h: f32) -> f32 {
    if len == 0 {
        return 0.0;
    }
    let rows = len.div_ceil(grid_columns(viewport_w)) as f32;
    let content = rows * GRID_ROW_H_PX + (rows - 1.0).max(0.0) * GRID_GAP_PX + 2.0 * GRID_PAD_PX;
    (content - viewport_h).max(0.0)
}

/// Render the grid from pre-built cells (call site builds each cell with its
/// `img()`, label, selection border, and `cx.listener` click — same pattern
/// as the overlay arrows). `scroll_px` shifts content up inside the clipped
/// area; the caller clamps it with [`grid_max_scroll`].
pub fn grid(items: Vec<AnyElement>, scroll_px: f32) -> impl IntoElement {
    let mut rows = div()
        .id("grid-rows")
        .flex()
        .flex_wrap()
        .gap(px(GRID_GAP_PX))
        .p(px(GRID_PAD_PX))
        .relative()
        .top(px(-scroll_px));
    for item in items {
        rows = rows.child(item);
    }
    div()
        .id("grid-scroll")
        .flex_1()
        .overflow_hidden()
        .child(rows)
}

#[cfg(test)]
mod tests {
    // NOTE: explicit imports instead of `use super::*` — gpui's glob re-exports
    // the `test` proc macro, which blows the recursion limit under `use super::*`.
    use super::{clamp_selection, grid_columns, grid_max_scroll, selection_range, GridSize};

    #[test]
    fn columns_follow_viewport_width() {
        assert_eq!(grid_columns(179.0), 1);
        assert_eq!(grid_columns(180.0), 1);
        assert_eq!(grid_columns(800.0), 4);
        assert_eq!(grid_columns(0.0), 1);
    }

    #[test]
    fn selection_sticks_at_ends() {
        assert_eq!(clamp_selection(0, 0), 0);
        assert_eq!(clamp_selection(5, 3), 2);
        assert_eq!(clamp_selection(1, 3), 1);
        assert_eq!(clamp_selection(0, 3), 0);
    }

    #[test]
    fn max_scroll_is_content_minus_visible() {
        assert_eq!(grid_max_scroll(0, 800.0, 600.0), 0.0);
        // 4 items, 800px wide → 1 row: 170 + 0 gaps + 24 pad = 194 < 600.
        assert_eq!(grid_max_scroll(4, 800.0, 600.0), 0.0);
        // 12 items → 3 rows: 3*170 + 2*8 + 24 = 550; 400 visible → 150.
        assert!((grid_max_scroll(12, 800.0, 400.0) - 150.0).abs() < 1e-4);
    }

    #[test]
    fn selection_range_covers_both_directions() {
        assert_eq!(selection_range(2, 5), vec![2, 3, 4, 5]);
        assert_eq!(selection_range(5, 2), vec![2, 3, 4, 5]);
        assert_eq!(selection_range(3, 3), vec![3]);
        assert_eq!(selection_range(0, 0), vec![0]);
    }

    // ── Zoomable grid: per-preset geometry + parameterized math ──

    #[test]
    fn geometry_table_matches_design_integers() {
        // Every value is an integer constant — no float derivation between
        // presets. M is today's geometry verbatim.
        let s = GridSize::S.geometry();
        assert_eq!(
            (
                s.cell_w, s.row_h, s.thumb_w, s.thumb_h, s.label_w, s.bar_w,
                s.bar_h, s.bar_left
            ),
            (120, 125, 100, 75, 100, 80, 3, 10)
        );
        let m = GridSize::M.geometry();
        assert_eq!(
            (
                m.cell_w, m.row_h, m.thumb_w, m.thumb_h, m.label_w, m.bar_w,
                m.bar_h, m.bar_left
            ),
            (180, 170, 160, 120, 160, 140, 3, 10)
        );
        let l = GridSize::L.geometry();
        assert_eq!(
            (
                l.cell_w, l.row_h, l.thumb_w, l.thumb_h, l.label_w, l.bar_w,
                l.bar_h, l.bar_left
            ),
            (240, 215, 220, 165, 220, 200, 3, 10)
        );
    }

    #[test]
    fn columns_follow_preset_cell_width() {
        // 800px viewport: S → 6 > M → 4 > L → 3.
        assert_eq!(grid_columns(800.0, &GridSize::S.geometry()), 6);
        assert_eq!(grid_columns(800.0, &GridSize::M.geometry()), 4);
        assert_eq!(grid_columns(800.0, &GridSize::L.geometry()), 3);
    }

    #[test]
    fn max_scroll_follows_preset_row_height() {
        // 12 items in an 800x200 viewport:
        // S: 2 rows → 2*125 + 1*8 + 24 = 282 → 82.
        // M: 3 rows → 3*170 + 2*8 + 24 = 550 → 350.
        // L: 4 rows → 4*215 + 3*8 + 24 = 908 → 708.
        assert!((grid_max_scroll(12, 800.0, 200.0, &GridSize::S.geometry()) - 82.0).abs() < 1e-4);
        assert!(
            (grid_max_scroll(12, 800.0, 200.0, &GridSize::M.geometry()) - 350.0).abs() < 1e-4
        );
        assert!(
            (grid_max_scroll(12, 800.0, 200.0, &GridSize::L.geometry()) - 708.0).abs() < 1e-4
        );
    }

    #[test]
    fn empty_grid_never_scrolls_at_any_preset() {
        for size in [GridSize::S, GridSize::M, GridSize::L] {
            assert_eq!(grid_max_scroll(0, 800.0, 600.0, &size.geometry()), 0.0);
        }
    }
}
