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

/// Uniform row height (thumb 120px + name label + padding).
pub const GRID_ROW_H_PX: f32 = 170.0;

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

/// Max scroll offset for `len` items in a viewport: content height minus
/// visible height, floored at zero (nothing to scroll).
pub fn grid_max_scroll(len: usize, viewport_w: f32, viewport_h: f32) -> f32 {
    if len == 0 {
        return 0.0;
    }
    let rows = len.div_ceil(grid_columns(viewport_w)) as f32;
    (rows * GRID_ROW_H_PX - viewport_h).max(0.0)
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
        .gap(px(8.0))
        .p(px(12.0))
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
    use super::{clamp_selection, grid_columns, grid_max_scroll};

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
        // 4 items, 800px wide → 1 row of 170px < 600px visible → nothing.
        assert_eq!(grid_max_scroll(4, 800.0, 600.0), 0.0);
        // 12 items → 3 rows = 510px; 400px visible → 110px scrollable.
        assert!((grid_max_scroll(12, 800.0, 400.0) - 110.0).abs() < 1e-4);
    }
}
