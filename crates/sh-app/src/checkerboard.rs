//! Fixed-gray transparency board for alpha-carrying images.
//!
//! Slice-B appearance owner (viewer-checkerboard change): every checkerboard
//! in the app uses these constants — no theme input anywhere, so a theme
//! switch can never alter the board (spec: Fixed Non-Themeable Appearance).
//!
//! Slice-B fallback (recorded): GPUI 0.2.2 offers no tile/repeat primitive —
//! `ObjectFit::{Fill, Contain, Cover, ScaleDown}` only stretch or scale a
//! source, so a baked 2-cell tile cannot be tiled by the compositor
//! (stretching it would smear the cells into a gradient). The board therefore
//! ships as ONE solid mid-gray underlay + 1px contrast outline: still exactly
//! one `#checkerboard` element per visible image, still the fixed grays, zero
//! per-frame allocation (plain `div`, no canvas repaint). [`checker_cell`]
//! pins the intended 2-gray checker rule so a future tiled implementation
//! reuses it.

use gpui::prelude::*;
use gpui::*;

/// Checker cell size in px. Fixed; not user-configurable in this change.
pub const CELL_PX: f32 = 16.0;
/// Fixed board gray (light cell). MUST NOT follow the active theme.
pub const GRAY_A: u32 = 0xC8C8C8;
/// Fixed board gray (dark cell). MUST NOT follow the active theme.
pub const GRAY_B: u32 = 0x969696;
/// Solid underlay fill: per-channel average of [`GRAY_A`] and [`GRAY_B`]
/// (`(0xC8 + 0x96) / 2 = 0xAF` per channel).
pub const BOARD_FILL: u32 = 0xAFAFAF;

/// Intended checker color for cell (`row`, `col`): alternating grays by
/// parity. Pure; pins the pattern rule for a future tiled implementation.
pub fn checker_cell(row: u32, col: u32) -> u32 {
    if (row + col).is_multiple_of(2) {
        GRAY_A
    } else {
        GRAY_B
    }
}

/// One transparency-board layer sized to the image frame.
///
/// Returns a single `div#checkerboard` (a `Stateful<Div>`, exactly one
/// element per visible image — spec: Single-Layer Composition) so the caller
/// can position it over the image frame (same absolute geometry as the
/// image). Takes no theme input.
pub fn checkerboard_layer(w_px: f32, h_px: f32) -> Stateful<Div> {
    div()
        .id("checkerboard")
        .w(px(w_px))
        .h(px(h_px))
        .bg(rgb(BOARD_FILL))
        .border(px(1.0))
        .border_color(rgb(GRAY_A))
}

#[cfg(test)]
mod tests {
    use super::{checker_cell, checkerboard_layer, BOARD_FILL, CELL_PX, GRAY_A, GRAY_B};

    #[test]
    fn board_constants_are_pinned() {
        assert!((CELL_PX - 16.0).abs() < f32::EPSILON);
        assert_eq!(GRAY_A, 0xC8C8C8);
        assert_eq!(GRAY_B, 0x969696);
    }

    #[test]
    fn board_fill_averages_both_grays_per_channel() {
        assert_eq!(BOARD_FILL, 0xAFAFAF);
        for shift in [16, 8, 0] {
            let a = (GRAY_A >> shift) & 0xFF;
            let b = (GRAY_B >> shift) & 0xFF;
            let f = (BOARD_FILL >> shift) & 0xFF;
            assert_eq!(f, (a + b) / 2);
        }
    }

    #[test]
    fn checker_rule_alternates_grays_by_parity() {
        assert_eq!(checker_cell(0, 0), GRAY_A);
        assert_eq!(checker_cell(0, 1), GRAY_B);
        assert_eq!(checker_cell(1, 0), GRAY_B);
        assert_eq!(checker_cell(1, 1), GRAY_A);
        assert_eq!(checker_cell(7, 7), GRAY_A);
    }

    #[test]
    fn layer_builds_without_theme_input() {
        // Signature takes only the frame size: theme-independence by
        // construction (no theme parameter exists to pass).
        let _layer = checkerboard_layer(320.0, 240.0);
        let _tiny = checkerboard_layer(1.0, 1.0);
    }
}
