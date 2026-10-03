//! Transparency board for alpha-carrying images.
//!
//! Single appearance owner: every checkerboard in the app â€” grid cell,
//! filmstrip thumb, viewer image â€” renders through [`checkerboard_layer`], so
//! no caller can drift from another.
//!
//! ## Why this reads the page color at all
//!
//! It used to take **no** theme input, on purpose. The original spec was
//! "Fixed Non-Themeable Appearance", on the reasoning that *"a theme switch can
//! never alter the board"* â€” the board is a semantic marker for "this pixel is
//! transparent", and a marker should carry the same weight everywhere.
//!
//! That reasoning holds, and the values are still not the user's to choose.
//! But the rule was implemented as a single fixed palette of **light** grays
//! (`0xC8C8C8` / `0x969696`), and three of the four built-in themes are dark.
//! The result was a bright gray block sitting on a dark grid, which is not a
//! marker at all â€” it reads as a rendering fault.
//!
//! So the appearance owner now resolves **one** thing from the theme: whether
//! the page is light or dark, and it picks between two **fixed** neutral
//! palettes accordingly. What this deliberately does *not* do:
//!
//! - It adds no theme slot. `ThemeColors` still cannot express a checker color.
//! - It gives the user no control. The colors remain constants.
//! - It does not derive tones from the palette. The two grays are literals, so
//!   the parity rule and the contrast between cells stay exactly as specified.
//!
//! The trade is deliberate and is the whole point: a theme switch *can* now
//! alter the board, because refusing that made the board wrong on most themes.
//! Light pages keep byte-identical output to the fixed palette that shipped.
//!
//! ## Rendering shape, unchanged
//!
//! GPUI offers no tile/repeat primitive â€” `ObjectFit::{Fill, Contain, Cover,
//! ScaleDown}` only stretch or scale a source, so a baked 2-cell tile cannot be
//! tiled by the compositor without smearing into a gradient. The board is
//! therefore ONE solid underlay plus a 1px contrast outline: still exactly one
//! `#checkerboard` element per visible image, still zero per-frame allocation
//! (plain `div`, no canvas repaint). [`checker_cell`] pins the intended
//! two-gray parity rule so a future tiled implementation reuses it.

use gpui::prelude::*;
use gpui::*;

/// Checker cell size in px. Fixed; not user-configurable.
pub const CELL_PX: f32 = 16.0;

/// Neutral grays used when the page is **light**. These are the values that
/// shipped as the only palette, kept byte-identical so a light theme renders
/// exactly what it always did.
pub const LIGHT_A: u32 = 0xC8C8C8;
pub const LIGHT_B: u32 = 0x969696;
/// Solid underlay: per-channel average of [`LIGHT_A`] and [`LIGHT_B`]
/// (`(0xC8 + 0x96) / 2 = 0xAF`).
pub const LIGHT_FILL: u32 = 0xAFAFAF;

/// Neutral grays used when the page is **dark**. The 50-step separation
/// between the two cells is deliberately identical to the light palette's, and
/// that constraint is what `both_palettes_keep_the_same_cell_contrast` pins: a
/// narrower dark gap made the parity pattern nearly invisible on exactly the
/// dark themes this exists to fix, while still passing every other assertion.
pub const DARK_A: u32 = 0x4A4A4E;
pub const DARK_B: u32 = 0x18181C;
/// `(0x4A + 0x18) / 2 = 0x31` per channel.
pub const DARK_FILL: u32 = 0x313135;

/// Relative luminance below which a page counts as dark.
///
/// 0.5 is the midpoint, not a tuned threshold: the two palettes are far enough
/// apart that no realistic page sits near it, so the exact value is not
/// load-bearing. Picking something "calibrated" would be a claim the evidence
/// does not support.
const DARK_PAGE_LUMA: f32 = 0.5;

/// sRGB relative luminance of an opaque color, matching the luma ramp
/// `hover_fill` and the theme derivations already use so the grid's surfaces
/// agree with each other.
fn luma(c: Hsla) -> f32 {
    let r: Rgba = c.into();
    0.2126 * r.r + 0.7152 * r.g + 0.0722 * r.b
}

/// The three grays that make up a board, selected by page polarity.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct BoardPalette {
    pub light_cell: u32,
    pub dark_cell: u32,
    pub fill: u32,
}

/// Palette for a page painted `page`.
///
/// Takes an already-parsed color rather than a hex string so this module owns
/// no parser and cannot disagree with the caller about what a malformed theme
/// color means. The caller keeps that fallback: `App` parses the background
/// with `parse_hex(...).unwrap_or(rgb(0x0d0d0f))`, so a malformed background
/// resolves to the same dark page it already paints and this function follows.
pub fn palette_for_page(page: Hsla) -> BoardPalette {
    if luma(page) < DARK_PAGE_LUMA {
        BoardPalette {
            light_cell: DARK_A,
            dark_cell: DARK_B,
            fill: DARK_FILL,
        }
    } else {
        BoardPalette {
            light_cell: LIGHT_A,
            dark_cell: LIGHT_B,
            fill: LIGHT_FILL,
        }
    }
}

/// Intended checker color for cell (`row`, `col`): alternating grays by parity.
/// Pure; pins the pattern rule for a future tiled implementation.
pub fn checker_cell(palette: BoardPalette, row: u32, col: u32) -> u32 {
    if (row + col).is_multiple_of(2) {
        palette.light_cell
    } else {
        palette.dark_cell
    }
}

/// One transparency-board layer sized to the image frame.
///
/// Returns a single `div#checkerboard` (a `Stateful<Div>`, exactly one element
/// per visible image â€” spec: Single-Layer Composition) so the caller can
/// position it over the image frame, using the same absolute geometry as the
/// image.
///
/// `palette` is resolved once per frame by the caller via [`palette_for_page`],
/// not here. Threading a pre-resolved `Copy` value keeps every presenter pure
/// and resolves the theme dependency once per frame instead of once per board,
/// which matters because a populated grid builds one board per visible image.
pub fn checkerboard_layer(w_px: f32, h_px: f32, palette: BoardPalette) -> Stateful<Div> {
    div()
        .id("checkerboard")
        .w(px(w_px))
        .h(px(h_px))
        .bg(rgb(palette.fill))
        .border(px(1.0))
        .border_color(rgb(palette.light_cell))
}

#[cfg(test)]
mod tests {
    use super::{
        checker_cell, checkerboard_layer, luma, palette_for_page, BoardPalette, CELL_PX, DARK_A,
        DARK_B, DARK_FILL, LIGHT_A, LIGHT_B, LIGHT_FILL,
    };
    use gpui::{rgb, Hsla};

    /// A light page and a dark page as the caller resolves them. `App` falls
    /// back to this same dark value when the theme background is malformed, so
    /// both polarities are reachable in production.
    fn light_page() -> Hsla {
        rgb(0xFFFFFF).into()
    }
    fn dark_page() -> Hsla {
        rgb(0x101014).into()
    }

    #[test]
    fn cell_size_is_pinned() {
        assert!((CELL_PX - 16.0).abs() < f32::EPSILON);
    }

    #[test]
    fn light_palette_is_byte_identical_to_the_fixed_palette_that_shipped() {
        // The regression this change could have introduced: a light theme must
        // render exactly what it rendered before polarity existed.
        assert_eq!(LIGHT_A, 0xC8C8C8);
        assert_eq!(LIGHT_B, 0x969696);
        assert_eq!(LIGHT_FILL, 0xAFAFAF);
    }

    #[test]
    fn both_palettes_derive_fill_as_the_per_channel_average() {
        for (fill, a, b) in [(LIGHT_FILL, LIGHT_A, LIGHT_B), (DARK_FILL, DARK_A, DARK_B)] {
            for shift in [16, 8, 0] {
                let av = (a >> shift) & 0xFF;
                let bv = (b >> shift) & 0xFF;
                let fv = (fill >> shift) & 0xFF;
                assert_eq!(fv, (av + bv) / 2, "fill must average both cells");
            }
        }
    }

    #[test]
    fn a_light_page_keeps_the_shipped_palette() {
        assert_eq!(
            palette_for_page(light_page()),
            BoardPalette {
                light_cell: LIGHT_A,
                dark_cell: LIGHT_B,
                fill: LIGHT_FILL,
            }
        );
    }

    #[test]
    fn a_dark_page_gets_the_dark_palette() {
        assert_eq!(
            palette_for_page(dark_page()),
            BoardPalette {
                light_cell: DARK_A,
                dark_cell: DARK_B,
                fill: DARK_FILL,
            }
        );
    }

    #[test]
    fn the_two_palettes_actually_differ_in_lightness() {
        // Without this the two palettes could drift to the same values and every
        // other test would still pass while the bug this change fixes returns.
        assert!(luma(dark_page()) < 0.2);
        assert!(luma(light_page()) > 0.9);
        assert!(
            palette_for_page(dark_page()).fill < palette_for_page(light_page()).fill,
            "the dark board must be darker than the light board"
        );
    }

    #[test]
    fn both_palettes_keep_the_same_parity_rule() {
        for palette in [
            palette_for_page(light_page()),
            palette_for_page(dark_page()),
        ] {
            assert_eq!(checker_cell(palette, 0, 0), palette.light_cell);
            assert_eq!(checker_cell(palette, 0, 1), palette.dark_cell);
            assert_eq!(checker_cell(palette, 1, 0), palette.dark_cell);
            assert_eq!(checker_cell(palette, 1, 1), palette.light_cell);
            assert_eq!(checker_cell(palette, 7, 7), palette.light_cell);
        }
    }

    #[test]
    fn both_palettes_keep_the_same_cell_contrast() {
        // If the dark palette sat much closer together than the light one, the
        // parity pattern would stop reading as a checker on dark pages, which is
        // the entire signal the board carries.
        let light = palette_for_page(light_page());
        let dark = palette_for_page(dark_page());
        let light_gap = (light.light_cell >> 8).abs_diff(light.dark_cell >> 8);
        let dark_gap = (dark.light_cell >> 8).abs_diff(dark.dark_cell >> 8);
        assert!(light_gap >= 0x20, "light cells need visible separation");
        assert_eq!(
            light_gap, dark_gap,
            "dark cells must separate by the same amount"
        );
    }

    #[test]
    fn layer_builds_for_either_polarity() {
        let _light = checkerboard_layer(320.0, 240.0, palette_for_page(light_page()));
        let _dark = checkerboard_layer(320.0, 240.0, palette_for_page(dark_page()));
        let _tiny = checkerboard_layer(1.0, 1.0, palette_for_page(dark_page()));
    }
}
