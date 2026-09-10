//! Image decoding abstraction. Returns raw RGBA8 plus dimensions.
//!
//! GPU note: GPUI expects BGRA8 on upload, so sh-app swaps channels after
//! calling into decode. Core stays format-neutral (RGBA8).

use crate::errors::{Result, ShImagesError};
use image::{DynamicImage, ImageReader};
use std::path::Path;

/// A decoded image in RGBA8.
#[derive(Debug, Clone, PartialEq)]
pub struct DecodedImage {
    /// Decoded width in pixels.
    pub width: u32,
    /// Decoded height in pixels.
    pub height: u32,
    /// Raw RGBA8 pixel data (`width * height * 4` bytes).
    pub rgba: Vec<u8>,
}

/// Default cap on the longest side before downscaling (protects RAM/VRAM).
pub const DEFAULT_MAX_DIMENSION: u32 = 8192;

/// Decode an image, downscaling to keep the longest side <= `max_dimension`.
pub fn load(path: &Path) -> Result<DecodedImage> {
    load_with_limit(path, DEFAULT_MAX_DIMENSION)
}

/// Decode with an explicit longest-side cap (0 disables the cap).
pub fn load_with_limit(path: &Path, max_dimension: u32) -> Result<DecodedImage> {
    // `ImageReader::open` deduces the format from the extension alone; guess by
    // content too, so files with unusual extensions still decode. Unknown
    // content with a known extension falls back to the extension-picked format.
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    // Detect the format by content; unknown formats fail here, before pixel decode.
    let _ = reader
        .format()
        .ok_or_else(|| ShImagesError::UnsupportedFormat(path.display().to_string()))?;
    let img = reader.decode()?;
    let source = to_rgba8(img);
    let longest = source.width().max(source.height());
    let source = if max_dimension > 0 && longest > max_dimension {
        let scale = max_dimension as f32 / longest as f32;
        image::imageops::resize(
            &source,
            (source.width() as f32 * scale).max(1.0) as u32,
            (source.height() as f32 * scale).max(1.0) as u32,
            image::imageops::FilterType::Lanczos3,
        )
    } else {
        source
    };
    Ok(DecodedImage {
        width: source.width(),
        height: source.height(),
        rgba: source.into_raw(),
    })
}

fn to_rgba8(img: DynamicImage) -> image::RgbaImage {
    img.to_rgba8()
}

/// Decode only the header (dimensions) without full pixel decode.
pub fn dimensions(path: &Path) -> Result<(u32, u32)> {
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    let dims = reader.into_dimensions()?;
    Ok((dims.0, dims.1))
}

#[cfg(test)]
mod tests {
    use super::*;
    use image::{Rgba, RgbaImage};

    fn write_png(path: &Path, img: &RgbaImage) {
        img.save(path).unwrap();
    }

    fn fixture(width: u32, height: u32) -> RgbaImage {
        RgbaImage::from_fn(width, height, |x, y| {
            Rgba([(x % 255) as u8, (y % 255) as u8, ((x + y) % 255) as u8, 255])
        })
    }

    #[test]
    fn decodes_png_dimensions_and_rgba() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.png");
        let img = fixture(4, 3);
        write_png(&p, &img);
        let decoded = load(&p).unwrap();
        assert_eq!((decoded.width, decoded.height), (4, 3));
        assert_eq!(decoded.rgba.len(), 4 * 3 * 4);
        assert_eq!(&decoded.rgba[0..4], &[0, 0, 0, 255]);
    }

    #[test]
    fn decodes_jpeg() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jpg");
        // The image 0.25 JPEG encoder accepts only L8/Rgb8, so the RGBA8
        // fixture must be converted to RGB8 before saving.
        let img = image::DynamicImage::from(fixture(4, 3)).to_rgb8();
        img.save(&p).unwrap();
        let decoded = load(&p).unwrap();
        assert_eq!((decoded.width, decoded.height), (4, 3));
    }

    #[test]
    fn corrupt_file_returns_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("corrupt.png");
        std::fs::write(&p, b"not really a png").unwrap();
        assert!(load(&p).is_err());
    }

    #[test]
    fn missing_file_returns_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = load(&dir.path().join("nope.png")).unwrap_err();
        assert!(matches!(err, ShImagesError::Io(_)));
    }

    #[test]
    fn unsupported_extension_still_decodes_by_content() {
        // `with_guessed_format` sniffs the magic bytes, so the extension is irrelevant.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("weird.dat");
        let img = fixture(2, 2);
        img.save_with_format(&p, image::ImageFormat::Png).unwrap(); // PNG bytes with .dat extension
        let decoded = load(&p).unwrap();
        assert_eq!((decoded.width, decoded.height), (2, 2));
    }

    #[test]
    fn downscales_huge_images() {
        let max = 64;
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big.png");
        let img = fixture(200, 100);
        write_png(&p, &img);
        let d = load_with_limit(&p, max).unwrap();
        assert!(d.width.max(d.height) <= max);
    }

    #[test]
    fn downscale_at_cap_is_exact() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big64.png");
        let img = fixture(200, 100);
        write_png(&p, &img);
        let d = load_with_limit(&p, 64).unwrap();
        assert_eq!((d.width, d.height), (64, 32));
    }

    #[test]
    fn zero_limit_disables_downscale() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("big0.png");
        let img = fixture(200, 100);
        write_png(&p, &img);
        let d = load_with_limit(&p, 0).unwrap();
        assert_eq!((d.width, d.height), (200, 100));
    }

    #[test]
    fn unknown_content_and_extension_returns_unsupported_format() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("data.xyz");
        std::fs::write(&p, b"\x00\x01\x02\x03 not an image").unwrap();
        let err = load(&p).unwrap_err();
        assert!(matches!(err, ShImagesError::UnsupportedFormat(_)));
    }
}
