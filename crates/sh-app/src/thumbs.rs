//! Folder-grid thumbnails: small BGRA images for GPUI rendering.
//!
//! Root cause of the 984MB grid (55 images): grid cells used `img(path)`,
//! so GPUI decoded every image at FULL resolution (~17MB each) to show a
//! 160px cell. Thumbs decode with a 256px longest-side cap and are handed to
//! GPUI as pre-built `RenderImage`s — ~170KB CPU + ~170KB GPU each instead.

use gpui::RenderImage;
use std::sync::Arc;

/// Longest side of a thumbnail decode. 256px covers the 160×120 cell with
/// headroom for HiDPI; larger only burns RAM for no visible gain.
pub const THUMB_MAX_DIM: u32 = 256;

/// Build a GPUI-renderable thumbnail from a decoded image.
///
/// Returns `None` when the pixel buffer doesn't match the dimensions
/// (corrupt decoder output) — the caller renders a placeholder instead.
/// Conversion is RGBA→BGRA: [`RenderImage`] is documented BGRA, while
/// sh-core stays format-neutral RGBA (see `decode.rs`).
pub fn render_thumb(decoded: &sh_core::decode::DecodedImage) -> Option<Arc<RenderImage>> {
    let expected = decoded.width as usize * decoded.height as usize * 4;
    if decoded.rgba.len() != expected || expected == 0 {
        return None;
    }
    let mut bgra = decoded.rgba.clone();
    for px in bgra.as_chunks_mut::<4>().0 {
        px.swap(0, 2);
    }
    let buffer = image::RgbaImage::from_raw(decoded.width, decoded.height, bgra)?;
    let frame = image::Frame::new(buffer);
    Some(Arc::new(RenderImage::new(smallvec::smallvec![frame])))
}

#[cfg(test)]
mod tests {
    use super::{render_thumb, THUMB_MAX_DIM};

    fn solid_rgba(w: u32, h: u32, r: u8, g: u8, b: u8, a: u8) -> sh_core::decode::DecodedImage {
        sh_core::decode::DecodedImage {
            width: w,
            height: h,
            rgba: vec![r, g, b, a]
                .into_iter()
                .cycle()
                .take(w as usize * h as usize * 4)
                .collect(),
        }
    }

    #[test]
    fn thumb_cap_is_256px() {
        assert_eq!(THUMB_MAX_DIM, 256);
    }

    #[test]
    fn render_thumb_swaps_to_bgra_and_sizes() {
        // Pure red pixel → BGRA must read blue in slot 0.
        let decoded = solid_rgba(4, 2, 255, 0, 0, 255);
        let thumb = render_thumb(&decoded).expect("valid buffer converts");
        let size = thumb.size(0);
        assert_eq!((size.width.0, size.height.0), (4, 2));
        let bytes = thumb.as_bytes(0).expect("frame has bytes");
        assert_eq!(bytes.len(), 4 * 2 * 4);
        assert_eq!(&bytes[0..4], &[0, 0, 255, 255]);
    }

    #[test]
    fn thumb_from_real_1080p_fixture_caps_at_256() {
        // End-to-end of the grid path on real file bytes (not synthetic):
        // header decode → 256px downscale → BGRA RenderImage.
        let fixture = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../sh-core/tests/fixtures/bench_1080p.png");
        let decoded = sh_core::decode::load_with_limit(&fixture, THUMB_MAX_DIM)
            .expect("bench fixture must decode");
        assert!(
            decoded.width.max(decoded.height) <= THUMB_MAX_DIM,
            "longest side capped: {}x{}",
            decoded.width,
            decoded.height
        );
        let thumb = render_thumb(&decoded).expect("real decode converts");
        let size = thumb.size(0);
        assert_eq!(size.width.0 as u32, decoded.width);
        assert_eq!(size.height.0 as u32, decoded.height);
        assert_eq!(
            thumb.as_bytes(0).expect("bytes").len(),
            decoded.width as usize * decoded.height as usize * 4
        );
    }

    #[test]
    fn render_thumb_rejects_mismatched_buffer() {
        let mut decoded = solid_rgba(2, 2, 1, 2, 3, 4);
        decoded.rgba.pop();
        assert!(render_thumb(&decoded).is_none());
        let empty = sh_core::decode::DecodedImage {
            width: 0,
            height: 0,
            rgba: Vec::new(),
        };
        assert!(render_thumb(&empty).is_none());
    }
}
