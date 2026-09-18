//! Settings content scroll geometry (Shortcuts section).
//!
//! Scroll note: gpui 0.2.2 has a native `overflow_y_scroll`, but a scroll
//! container inside a flex layout needs `min-height: 0` to actually bound
//! itself (flex auto-minimums grow it to its content instead), and `Div`
//! exposes no `min-height` setter — so the native path can never engage
//! here (verified against gpui + taffy 0.9.0 sources; an `overflow_y_scroll`
//! attempt shipped dead scroll). This module follows the grid precedent
//! (`ui/grid.rs`): the render wraps content in a `relative` div shifted by
//! `-scroll_px` (driven by the root wheel handler in Settings view), clamped
//! by [`settings_max_scroll`]. Row heights below are EXPLICIT in render
//! (`.h()`, never content-sized) so the clamp arithmetic stays exact —
//! changing a row height without updating its constant strands rows out of
//! reach, exactly like `GRID_ROW_H_PX`.

/// Gap between section children — must match the `gap()` in the render.
pub const SETTINGS_GAP_PX: f32 = 8.0;

/// Gap between rows inside the Shortcuts column — must match its `gap()`.
/// (The section column uses a tighter gap than the outer content column.)
pub const SHORTCUT_GAP_PX: f32 = 2.0;

/// Content padding (each side) — must match the `p()` in the render.
pub const SETTINGS_PAD_PX: f32 = 16.0;

/// Reset-all button row height — must match its `.h()` in the render.
pub const SHORTCUT_RESET_H_PX: f32 = 40.0;

/// Context group header height — must match its `.h()` in the render.
pub const SHORTCUT_GROUP_H_PX: f32 = 32.0;

/// Shortcut row height — must match its `.h()` in the render. Rows stay
/// single-line; wrapping would silently break [`shortcuts_content_h`].
pub const SHORTCUT_ROW_H_PX: f32 = 40.0;

/// Shortcut actions rendered (reset button excluded) — must match the
/// `ACTIONS` table length consumed by the Shortcuts section.
pub const SHORTCUT_ROW_COUNT: usize = 18;

/// Context groups rendered — Viewer, Grid, Global, in that order.
pub const SHORTCUT_GROUP_COUNT: usize = 3;

/// Inner content height of the Shortcuts section (no outer padding):
/// reset + group headers + rows + inter-child gaps. Exact by construction
/// (all heights explicit in render).
pub fn shortcuts_content_h() -> f32 {
    let rows = SHORTCUT_ROW_COUNT as f32;
    let groups = SHORTCUT_GROUP_COUNT as f32;
    // Children: reset + group headers + rows; gaps sit between children.
    let children = 1.0 + groups + rows;
    SHORTCUT_RESET_H_PX
        + groups * SHORTCUT_GROUP_H_PX
        + rows * SHORTCUT_ROW_H_PX
        + (children - 1.0).max(0.0) * SHORTCUT_GAP_PX
}

/// Max scroll offset: content height minus visible height, floored at zero
/// (nothing to scroll). Same shape as [`crate::ui::grid::grid_max_scroll`].
pub fn settings_max_scroll(content_h: f32, visible_h: f32) -> f32 {
    (content_h - visible_h).max(0.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcuts_content_height_is_exact() {
        // 40 + 3*32 + 18*40 + 21*2 = 40 + 96 + 720 + 42 = 898.
        assert_eq!(shortcuts_content_h(), 898.0);
    }

    #[test]
    fn max_scroll_is_content_minus_visible_floored() {
        assert_eq!(settings_max_scroll(898.0, 1200.0), 0.0);
        assert_eq!(settings_max_scroll(898.0, 498.0), 400.0);
        assert_eq!(settings_max_scroll(0.0, 0.0), 0.0);
    }
}
