//! Crop rectangle math in viewport units plus mapping to image pixels.
//!
//! Pure logic: no GPU types. The UI owns the viewport rectangle drawn over
//! the scaled image; this module normalizes it, clamps it to the image, and
//! converts it to integer pixel bounds for the exporter.

use crate::decode::{self, DecodedImage};
use crate::errors::{Result, ShImagesError};
use crate::transform::Vec2;
use std::path::Path;

/// Crop rectangle in viewport units (pixels on screen).
///
/// The two corners may be in any order — e.g. the user can drag from
/// bottom-right to top-left. Call [`normalized`](Self::normalized) before
/// interpreting the rectangle.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct CropRect {
    /// First corner x in viewport units.
    pub x0: f32,
    /// First corner y in viewport units.
    pub y0: f32,
    /// Opposite corner x in viewport units.
    pub x1: f32,
    /// Opposite corner y in viewport units.
    pub y1: f32,
}

impl CropRect {
    /// Order the corners so `(x0, y0)` is the top-left and `(x1, y1)` the
    /// bottom-right.
    pub fn normalized(self) -> Self {
        Self {
            x0: self.x0.min(self.x1),
            y0: self.y0.min(self.y1),
            x1: self.x0.max(self.x1),
            y1: self.y0.max(self.y1),
        }
    }

    /// Clamp each coordinate into the image bounds: `x0`/`x1` into `[0, w]`,
    /// `y0`/`y1` into `[0, h]`.
    pub fn clamp_to_image(self, w: f32, h: f32) -> Self {
        Self {
            x0: self.x0.clamp(0.0, w),
            y0: self.y0.clamp(0.0, h),
            x1: self.x1.clamp(0.0, w),
            y1: self.y1.clamp(0.0, h),
        }
    }

    /// Convert the viewport rectangle to integer image-pixel bounds
    /// `(x, y, w, h)`.
    ///
    /// Steps, each documented:
    /// - `scale <= 0.0` returns `None`: the viewport→image mapping divides
    ///   by `scale`, so a non-positive scale has no valid inverse.
    /// - `normalized()` first: drag direction must not affect the result.
    /// - Both corners map through [`viewport_to_image`], inverting the
    ///   current zoom/pan.
    /// - `clamp_to_image` keeps the rect inside the image; anything outside
    ///   is not exportable pixels.
    /// - Zero/negative area after clamping returns `None` (degenerate rect).
    /// - Otherwise the origin floors and the size ceils (`.max(1.0)` guards
    ///   tiny positive areas to at least 1px so a visible sliver still
    ///   exports one pixel).
    pub fn to_pixels(
        self,
        scale: f32,
        offset: Vec2,
        img_w: f32,
        img_h: f32,
    ) -> Option<(u32, u32, u32, u32)> {
        if scale <= 0.0 {
            return None;
        }
        let n = self.normalized();
        let (ax, ay) = viewport_to_image(n.x0, n.y0, scale, offset);
        let (bx, by) = viewport_to_image(n.x1, n.y1, scale, offset);
        let c = CropRect {
            x0: ax,
            y0: ay,
            x1: bx,
            y1: by,
        }
        .clamp_to_image(img_w, img_h);
        let (w, h) = (c.x1 - c.x0, c.y1 - c.y0);
        if w <= 0.0 || h <= 0.0 {
            return None;
        }
        Some((
            c.x0.floor() as u32,
            c.y0.floor() as u32,
            w.ceil().max(1.0) as u32,
            h.ceil().max(1.0) as u32,
        ))
    }
}

/// Map a viewport point (pixels on screen) to image coordinates.
///
/// Exact inverse of the zoom/pan applied by the renderer:
/// `((x - offset.x) / scale, (y - offset.y) / scale)`.
pub fn viewport_to_image(x: f32, y: f32, scale: f32, offset: Vec2) -> (f32, f32) {
    ((x - offset.x) / scale, (y - offset.y) / scale)
}

/// Decode full + cut `(x, y, w, h)` in image px, at most `max_dimension`
/// on the longest side.
///
/// Errors (never panics) on empty rects, out-of-bounds rects, or decode
/// failures. Bounds are pre-checked with a header-only probe so a bad rect
/// fails fast without paying the full decode. (`imageops::crop_imm` alone
/// would silently clamp — here OOB is a caller bug worth surfacing.)
///
/// # Coordinate spaces
///
/// `rect` is addressed in HEADER space — the uncapped dimensions
/// [`decode::probe_dimensions`] reports, which is what the viewer's zoom
/// math and drag handling compute against. The decode, however, may be
/// CAPPED, and a capped frame has a different size than the header. So the
/// rect is projected into the decoded frame's space before cutting; cutting
/// the uncapped rect against a capped buffer would either panic inside
/// `crop_imm` or, worse, silently return the wrong pixels when the cap only
/// shifted the origin. The projection uses the same floor-origin /
/// ceil-size rule as [`CropRect::to_pixels`], so a drag and a programmatic
/// crop agree pixel for pixel.
///
/// # Consequence of a cap
///
/// The exported crop is a crop OF THE CAPPED FRAME, so its resolution
/// follows the cap: a 100x100 region taken from a 6000px image with
/// `max_dimension = 512` exports at roughly 9x9, not 100x100. That is the
/// setting doing what it says ("never decode more than this on the longest
/// side") rather than a silent surprise, and it is the deal an image viewer
/// makes when the user caps its own decode budget. With the default
/// [`decode::DEFAULT_MAX_DIMENSION`] and any image at or under it, the
/// projection is the identity and this function is byte-identical to the
/// uncapped path.
pub fn crop_image_with_limit(
    path: &Path,
    rect: (u32, u32, u32, u32),
    max_dimension: u32,
) -> Result<DecodedImage> {
    let (x, y, w, h) = rect;
    if w == 0 || h == 0 {
        return Err(ShImagesError::Unknown("crop rectangle has no area".into()));
    }
    let (img_w, img_h) = decode::probe_dimensions(path)?;
    if x.saturating_add(w) > img_w || y.saturating_add(h) > img_h {
        return Err(ShImagesError::Unknown(format!(
            "crop rectangle ({x},{y} {w}x{h}) outside image ({img_w}x{img_h}): {}",
            path.display()
        )));
    }
    let full = decode::load_with_limit(path, max_dimension)?;
    // `full` IS the capped frame, so derive the mapping from what actually
    // decoded rather than from the header: a future resize path that
    // ignored the cap would then surface as a bounds error instead of a
    // wrong-pixel crop.
    let (fx_w, fx_h) = (full.width, full.height);
    let (fx, fy, fw, fh) = if (fx_w, fx_h) == (img_w, img_h) {
        (x, y, w, h)
    } else {
        let (sx, sy) = (fx_w as f32 / img_w as f32, fx_h as f32 / img_h as f32);
        // Floor the origin, ceil the size (same rule as `to_pixels`): a
        // ceil on both would bias every crop one pixel down-right, and a
        // floor on both could drop a 1px-wide selection entirely.
        let fx = (x as f32 * sx).floor().max(0.0) as u32;
        let fy = (y as f32 * sy).floor().max(0.0) as u32;
        let fw = ((w as f32 * sx).ceil().max(1.0) as u32).min(fx_w.saturating_sub(fx));
        let fh = ((h as f32 * sy).ceil().max(1.0) as u32).min(fx_h.saturating_sub(fy));
        (fx, fy, fw, fh)
    };
    if fw == 0 || fh == 0 {
        return Err(ShImagesError::Unknown(format!(
            "crop rectangle ({x},{y} {w}x{h}) collapses to no area under a {max_dimension}px decode cap (frame {fx_w}x{fx_h}): {}",
            path.display()
        )));
    }
    // Bounds are re-checked against the DECODED frame here, which is the
    // only check that can keep `crop_imm`'s documented panic unreachable.
    if fx.saturating_add(fw) > fx_w || fy.saturating_add(fh) > fx_h {
        return Err(ShImagesError::Unknown(format!(
            "crop rectangle ({fx},{fy} {fw}x{fh}) outside decoded frame ({fx_w}x{fx_h}): {}",
            path.display()
        )));
    }
    let buf = image::RgbaImage::from_raw(full.width, full.height, full.rgba)
        .ok_or_else(|| ShImagesError::Unknown("decoded buffer size mismatch".into()))?;
    let sub = image::imageops::crop_imm(&buf, fx, fy, fw, fh).to_image();
    Ok(DecodedImage {
        width: sub.width(),
        height: sub.height(),
        rgba: sub.into_raw(),
    })
}

/// [`crop_image_with_limit`] at [`decode::DEFAULT_MAX_DIMENSION`].
///
/// Same sibling-not-signature rule as [`decode::load`] /
/// [`decode::load_with_limit`]: the default is a real behavior, so a caller
/// with no cap to offer says so explicitly instead of passing a magic
/// number. Production passes the user's `Settings::max_decode_dimension`.
pub fn crop_image(path: &Path, rect: (u32, u32, u32, u32)) -> Result<DecodedImage> {
    crop_image_with_limit(path, rect, decode::DEFAULT_MAX_DIMENSION)
}

/// Write RGBA pixels as PNG (lossless), always PNG regardless of extension.
/// Creates parent dirs like `settings::save`.
pub fn save_png(path: &Path, img: &DecodedImage) -> Result<()> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)?;
        }
    }
    let buf = image::RgbaImage::from_raw(img.width, img.height, img.rgba.clone())
        .ok_or_else(|| ShImagesError::Unknown("crop buffer size mismatch".into()))?;
    buf.save_with_format(path, image::ImageFormat::Png)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture() -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../sh-core/tests/fixtures/bench_1080p.png")
    }

    #[test]
    fn crop_image_cuts_center_of_fixture() {
        let cut = crop_image(&fixture(), (910, 490, 100, 100)).unwrap();
        assert_eq!((cut.width, cut.height), (100, 100));
        assert_eq!(cut.rgba.len(), 100 * 100 * 4);
    }

    #[test]
    fn crop_out_of_bounds_errors() {
        assert!(crop_image(&fixture(), (5000, 5000, 10, 10)).is_err());
    }

    #[test]
    fn save_png_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let cut = crop_image(&fixture(), (0, 0, 50, 40)).unwrap();
        let out = dir.path().join("cut.png");
        save_png(&out, &cut).unwrap();
        let back = crate::decode::load(&out).unwrap();
        assert_eq!((back.width, back.height), (50, 40));
    }

    // ── Capped decode (Settings::max_decode_dimension reaches the crop) ──

    /// The load-bearing test: the OUTPUT of a whole-image crop IS the
    /// decoded frame, so its dimensions prove the cap reached the decode
    /// rather than merely being stored in a settings struct. 1920x1080
    /// under a 640 cap is exactly one third on each axis.
    #[test]
    fn crop_image_with_limit_caps_the_decoded_frame() {
        let cut = crop_image_with_limit(&fixture(), (0, 0, 1920, 1080), 640).unwrap();

        assert_eq!((cut.width, cut.height), (640, 360));
        assert_eq!(cut.rgba.len(), 640 * 360 * 4);
    }

    /// The cap must not change the default path. The fixture is 1920x1080,
    /// well under `DEFAULT_MAX_DIMENSION` (8192), so the header-space rect
    /// projects onto itself and the result is bit-identical in dimensions.
    #[test]
    fn default_limit_leaves_the_decoded_frame_uncapped() {
        let (w, h) = crate::decode::probe_dimensions(&fixture()).unwrap();
        let cut = crop_image(&fixture(), (0, 0, w, h)).unwrap();

        assert_eq!((cut.width, cut.height), (w, h));
    }

    /// A sub-rect is projected into the capped frame: rect (100,100,200,100)
    /// on a 1920x1080 source under a 640 cap scales by exactly 1/3, giving
    /// origin floor(100/3)=33 and size ceil(200/3)=67 x ceil(100/3)=34.
    /// Hardcoded on purpose — this pins the floor/ceil rule, so switching
    /// to a different rule (or dropping the projection) fails here instead
    /// of silently shifting every capped crop.
    #[test]
    fn crop_image_with_limit_projects_a_sub_rect_into_the_capped_frame() {
        let cut = crop_image_with_limit(&fixture(), (100, 100, 200, 100), 640).unwrap();

        assert_eq!((cut.width, cut.height), (67, 34));
    }

    /// An uncapped 1-cap decode collapses the frame to 1x1. The crop must
    /// still return a valid, non-empty buffer rather than panicking inside
    /// `crop_imm` on a rect that no longer fits.
    #[test]
    fn extreme_cap_yields_a_valid_single_pixel_crop() {
        let cut = crop_image_with_limit(&fixture(), (0, 0, 1920, 1080), 1).unwrap();

        assert_eq!((cut.width, cut.height), (1, 1));
        assert_eq!(cut.rgba.len(), 4);
    }

    /// `0` disables the cap, matching `load_with_limit`'s contract.
    #[test]
    fn zero_cap_disables_the_decode_limit() {
        let cut = crop_image_with_limit(&fixture(), (0, 0, 1920, 1080), 0).unwrap();

        assert_eq!((cut.width, cut.height), (1920, 1080));
    }

    /// A rect the user can legitimately draw in header space must never
    /// become a bounds error purely because a cap is in play.
    #[test]
    fn capped_crop_accepts_every_in_bounds_header_rect() {
        for cap in [1u32, 37, 640, 1920, 8192] {
            let cut = crop_image_with_limit(&fixture(), (1910, 1070, 10, 10), cap)
                .unwrap_or_else(|e| panic!("cap {cap} must accept an in-bounds rect: {e}"));
            assert!(
                cut.width >= 1 && cut.height >= 1,
                "cap {cap} cut an empty area"
            );
            assert_eq!(cut.rgba.len(), (cut.width * cut.height * 4) as usize);
        }
    }

    /// Out-of-bounds stays an error under a cap — the header (uncapped)
    /// bounds are the contract, so a bad rect fails the same way regardless
    /// of the cap.
    #[test]
    fn capped_crop_still_rejects_out_of_bounds_rects() {
        assert!(crop_image_with_limit(&fixture(), (5000, 5000, 10, 10), 640).is_err());
    }

    #[test]
    fn normalize_orders_corners() {
        let r = CropRect {
            x0: 300.0,
            y0: 200.0,
            x1: 100.0,
            y1: 50.0,
        }
        .normalized();
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (100.0, 50.0, 300.0, 200.0));
    }

    #[test]
    fn clamp_keeps_rect_inside_image() {
        let r = CropRect {
            x0: -20.0,
            y0: -10.0,
            x1: 5000.0,
            y1: 3000.0,
        }
        .clamp_to_image(1920.0, 1080.0);
        assert_eq!((r.x0, r.y0, r.x1, r.y1), (0.0, 0.0, 1920.0, 1080.0));
    }

    #[test]
    fn viewport_to_image_inverts_zoom_pan() {
        let p = viewport_to_image(300.0, 150.0, 2.0, Vec2 { x: 100.0, y: 50.0 });
        assert!((p.0 - 100.0).abs() < 1e-4 && (p.1 - 50.0).abs() < 1e-4);
    }

    #[test]
    fn degenerate_rect_is_empty() {
        assert!(CropRect {
            x0: 10.0,
            y0: 10.0,
            x1: 10.0,
            y1: 50.0
        }
        .to_pixels(1.0, Vec2 { x: 0.0, y: 0.0 }, 1920.0, 1080.0)
        .is_none());
    }

    proptest::proptest! {
        #[test]
        fn crop_invariants_hold(
            x0 in -2000.0f32..4000.0,
            y0 in -2000.0f32..4000.0,
            x1 in -2000.0f32..4000.0,
            y1 in -2000.0f32..4000.0,
            scale in 0.01f32..8.0,
            ox in -1000.0f32..1000.0,
            oy in -1000.0f32..1000.0,
        ) {
            let r = CropRect { x0, y0, x1, y1 };
            // normalize always orders corners top-left -> bottom-right.
            let n = r.normalized();
            assert!(n.x0 <= n.x1 && n.y0 <= n.y1);
            // clamp always lands inside the image bounds.
            let c = n.clamp_to_image(1920.0, 1080.0);
            assert!((0.0..=1920.0).contains(&c.x0));
            assert!((0.0..=1920.0).contains(&c.x1));
            assert!((0.0..=1080.0).contains(&c.y0));
            assert!((0.0..=1080.0).contains(&c.y1));
            // to_pixels with positive area always yields w, h >= 1.
            if let Some((_, _, w, h)) =
                r.to_pixels(scale, Vec2 { x: ox, y: oy }, 1920.0, 1080.0)
            {
                assert!(w >= 1 && h >= 1);
            }
        }
    }
}
