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

/// Reads image dimensions from the file header without decoding pixels.
///
/// Uses content-based format detection (no extension trust), consistent with
/// [`load`]. Only the header is read, so this is near-instant even for huge
/// files — the UI uses it for fit/zoom math without paying a full decode.
///
/// # Errors
///
/// Returns [`ShImagesError::Io`] for missing/unreadable files,
/// [`ShImagesError::UnsupportedFormat`] when the content is not a recognized
/// image format, and [`ShImagesError::Decode`] for corrupt headers.
pub fn probe_dimensions(path: &Path) -> Result<(u32, u32)> {
    // Same detection pattern as `load`: guess by content, then fail on
    // unknown formats BEFORE header parsing so callers can distinguish
    // not-an-image from a corrupt image (matching `load`'s error classes).
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    let _ = reader
        .format()
        .ok_or_else(|| ShImagesError::UnsupportedFormat(path.display().to_string()))?;
    let dims = reader.into_dimensions()?;
    Ok((dims.0, dims.1))
}

/// Probes whether the image at `path` carries usable transparency.
///
/// Format fast path: formats that cannot carry an alpha channel (JPEG)
/// answer `Ok(false)` from the header alone — no pixel data is inspected.
/// Alpha-capable formats decode through the existing [`load`] pipeline
/// (including its downscale cap) and delegate to [`has_alpha_rgba`].
///
/// # Errors
///
/// Same classes as [`probe_dimensions`]: [`ShImagesError::Io`] for
/// missing/unreadable files, [`ShImagesError::UnsupportedFormat`] for
/// unrecognized content, [`ShImagesError::Decode`] for corrupt data.
/// Never returns a transparency verdict on error.
pub fn probe_has_alpha(path: &Path) -> Result<bool> {
    // Same detection pattern as `load`/`probe_dimensions` so error classes
    // stay indistinguishable across the probe family.
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    let format = reader
        .format()
        .ok_or_else(|| ShImagesError::UnsupportedFormat(path.display().to_string()))?;
    if !format_can_have_alpha(format) {
        // By construction: the format has no alpha channel, so no pixel
        // can be below fully opaque — answer from the header, no decode.
        return Ok(false);
    }
    let decoded = load(path)?;
    Ok(has_alpha_rgba(&decoded))
}

/// Whether a decoded frame of this format can carry an alpha channel at
/// all. JPEG is the notable no-alpha case (gray/YCbCr only); every other
/// format the app supports (PNG, GIF, WebP, TIFF, BMP, ICO, …) may carry
/// alpha, so those take the decode-and-scan path — an honest verdict over
/// a verdict guessed from the format name.
fn format_can_have_alpha(format: image::ImageFormat) -> bool {
    !matches!(format, image::ImageFormat::Jpeg)
}

/// Reports whether decoded RGBA8 bytes carry usable transparency.
///
/// Returns `true` iff at least one pixel has an alpha value below fully
/// opaque (`255`). Pure in-memory scan with early exit and no I/O, so the
/// grid path can run it on already-decoded thumbnail bytes for free.
pub fn has_alpha_rgba(decoded: &DecodedImage) -> bool {
    decoded.rgba.chunks_exact(4).any(|px| px[3] < 255)
}

/// File facts for the viewer info popover (no-EXIF slice).
///
/// Dimensions come from the header-only [`probe_dimensions`] (no pixel
/// allocation); `size_bytes` from a single `metadata` stat; `format` is the
/// canonical uppercase display name (`"PNG"`, `"JPEG"`, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileInfo {
    /// Pixel width from the image header.
    pub width: u32,
    /// Pixel height from the image header.
    pub height: u32,
    /// File size in bytes (`metadata.len()`).
    pub size_bytes: u64,
    /// Canonical uppercase format name (e.g. `"PNG"`).
    pub format: String,
}

/// Canonical uppercase display name for a detected image format.
///
/// Known formats map explicitly; anything else falls back to the
/// debug-name uppercased so future formats still render sensibly.
fn canonical_format_name(fmt: image::ImageFormat) -> String {
    match fmt {
        image::ImageFormat::Png => "PNG".into(),
        image::ImageFormat::Jpeg => "JPEG".into(),
        image::ImageFormat::Gif => "GIF".into(),
        image::ImageFormat::WebP => "WEBP".into(),
        image::ImageFormat::Bmp => "BMP".into(),
        image::ImageFormat::Ico => "ICO".into(),
        image::ImageFormat::Tiff => "TIFF".into(),
        other => format!("{other:?}").to_uppercase(),
    }
}

/// Resolve the three info-panel facts without decoding pixel data.
///
/// Dimensions delegate to the header-only [`probe_dimensions`]; size comes
/// from one `metadata` stat; format reuses the
/// `ImageReader::open → with_guessed_format → format()` path `load` uses.
///
/// # Errors
///
/// `ShImagesError::Io` for missing/unreadable files (including the metadata
/// call), `UnsupportedFormat` for unrecognized content, `Decode` for corrupt
/// headers — same classes as [`probe_dimensions`]; never panics.
pub fn probe_file_info(path: &Path) -> Result<FileInfo> {
    let (width, height) = probe_dimensions(path)?;
    let size_bytes = std::fs::metadata(path).map_err(ShImagesError::Io)?.len();
    let reader = ImageReader::open(path)?.with_guessed_format()?;
    let format = reader
        .format()
        .ok_or_else(|| ShImagesError::UnsupportedFormat(path.display().to_string()))?;
    Ok(FileInfo {
        width,
        height,
        size_bytes,
        format: canonical_format_name(format),
    })
}

/// Format a byte count with decimal SI units, one fractional digit
/// (`"512 B"`, `"1.0 KB"`, `"2.4 MB"`). Pure; output is locale-neutral and
/// stays untranslated per the proper-nouns rule.
pub fn format_file_size(bytes: u64) -> String {
    const UNITS: &[&str] = &["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64 / 1000.0;
    let mut unit = 1;
    while value >= 1000.0 && unit + 1 < UNITS.len() {
        value /= 1000.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
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

    #[test]
    fn probe_dimensions_reads_fixture_png() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("probe.png");
        let img = fixture(17, 9);
        write_png(&p, &img);
        assert_eq!(probe_dimensions(&p).unwrap(), (17, 9));
    }

    #[test]
    fn probe_dimensions_corrupt_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("corrupt.png");
        std::fs::write(&p, b"definitely not a png header").unwrap();
        assert!(probe_dimensions(&p).is_err());
    }

    #[test]
    fn probe_dimensions_missing_file_errors() {
        let dir = tempfile::tempdir().unwrap();
        let err = probe_dimensions(&dir.path().join("nope.png")).unwrap_err();
        assert!(matches!(err, ShImagesError::Io(_)));
    }

    #[test]
    fn probe_dimensions_unknown_content_is_unsupported_format() {
        // Unknown content AND unknown extension → `UnsupportedFormat`, matching
        // `load`'s error classes. (Unknown content with a known image
        // extension falls back to that format and fails as `Decode` — the
        // corrupt-header case covered above.)
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("data.xyz");
        std::fs::write(&p, b"\x00\x01\x02\x03 not an image").unwrap();
        let err = probe_dimensions(&p).unwrap_err();
        assert!(matches!(err, ShImagesError::UnsupportedFormat(_)));
    }

    #[test]
    fn probe_file_info_resolves_png_facts() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("facts.png");
        let img = fixture(17, 9);
        write_png(&p, &img);
        let info = probe_file_info(&p).unwrap();
        assert_eq!((info.width, info.height), (17, 9));
        assert_eq!(info.format, "PNG");
        assert_eq!(info.size_bytes, std::fs::metadata(&p).unwrap().len());
    }

    #[test]
    fn probe_file_info_resolves_jpeg_facts() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("facts.jpg");
        let img = image::DynamicImage::from(fixture(4, 3)).to_rgb8();
        img.save(&p).unwrap();
        let info = probe_file_info(&p).unwrap();
        assert_eq!((info.width, info.height), (4, 3));
        assert_eq!(info.format, "JPEG");
        assert_eq!(info.size_bytes, std::fs::metadata(&p).unwrap().len());
    }

    #[test]
    fn probe_file_info_matches_header_probe_on_tiny_image() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("tiny.png");
        let img = fixture(1, 1);
        write_png(&p, &img);
        let info = probe_file_info(&p).unwrap();
        assert_eq!(probe_dimensions(&p).unwrap(), (info.width, info.height));
        assert_eq!((info.width, info.height), (1, 1));
    }

    #[test]
    fn probe_file_info_corrupt_file_returns_typed_error() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("corrupt.png");
        std::fs::write(&p, b"not really a png").unwrap();
        let err = probe_file_info(&p).unwrap_err();
        assert!(
            matches!(
                err,
                ShImagesError::Decode(_) | ShImagesError::UnsupportedFormat(_)
            ),
            "corrupt file must map to Decode/UnsupportedFormat, got {err:?}"
        );
    }

    #[test]
    fn probe_file_info_missing_file_returns_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = probe_file_info(&dir.path().join("nope.png")).unwrap_err();
        assert!(matches!(err, ShImagesError::Io(_)));
    }

    #[test]
    fn format_file_size_table() {
        assert_eq!(format_file_size(0), "0 B");
        assert_eq!(format_file_size(512), "512 B");
        assert_eq!(format_file_size(1023), "1023 B");
        assert_eq!(format_file_size(1024), "1.0 KB");
        assert_eq!(format_file_size(1536), "1.5 KB");
        assert_eq!(format_file_size(2_400_000), "2.4 MB");
        assert_eq!(format_file_size(3_100_000_000), "3.1 GB");
    }

    // ── Transparency detection (viewer-checkerboard, Slice A) ──

    fn transparent_png(width: u32, height: u32) -> RgbaImage {
        // One semitransparent pixel at (0,0); everything else opaque.
        let mut img = RgbaImage::from_pixel(width, height, Rgba([200, 30, 40, 255]));
        img.put_pixel(0, 0, Rgba([255, 0, 0, 128]));
        img
    }

    #[test]
    fn has_alpha_rgba_true_for_single_semitransparent_pixel() {
        let decoded = DecodedImage {
            width: 1,
            height: 1,
            rgba: vec![255, 0, 0, 254],
        };
        assert!(has_alpha_rgba(&decoded));
    }

    #[test]
    fn has_alpha_rgba_false_for_all_opaque_buffer() {
        let decoded = DecodedImage {
            width: 1,
            height: 2,
            rgba: vec![10, 20, 30, 255, 40, 50, 60, 255],
        };
        assert!(!has_alpha_rgba(&decoded));
    }

    #[test]
    fn has_alpha_rgba_false_for_empty_buffer() {
        let decoded = DecodedImage {
            width: 0,
            height: 0,
            rgba: Vec::new(),
        };
        assert!(!has_alpha_rgba(&decoded));
    }

    #[test]
    fn has_alpha_rgba_true_for_decoded_transparent_png() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.png");
        write_png(&p, &transparent_png(2, 2));
        let decoded = load(&p).unwrap();
        assert!(has_alpha_rgba(&decoded));
    }

    #[test]
    fn has_alpha_rgba_false_for_decoded_opaque_png() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("o.png");
        write_png(&p, &fixture(4, 3)); // fixture() paints every pixel alpha 255
        let decoded = load(&p).unwrap();
        assert!(!has_alpha_rgba(&decoded));
    }

    #[test]
    fn probe_has_alpha_transparent_png_returns_true() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.png");
        write_png(&p, &transparent_png(1, 1));
        assert_eq!(probe_has_alpha(&p).unwrap(), true);
    }

    #[test]
    fn probe_has_alpha_opaque_png_returns_false() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("o.png");
        write_png(&p, &fixture(4, 3));
        assert_eq!(probe_has_alpha(&p).unwrap(), false);
    }

    #[test]
    fn probe_has_alpha_jpeg_returns_false() {
        // JPEG has no alpha channel, so the verdict comes from the header
        // alone — by construction, no pixel data is inspected.
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("t.jpg");
        let img = image::DynamicImage::from(fixture(4, 3)).to_rgb8();
        img.save(&p).unwrap();
        assert_eq!(probe_has_alpha(&p).unwrap(), false);
    }

    #[test]
    fn probe_has_alpha_corrupt_file_errors_never_a_verdict() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("corrupt.png");
        std::fs::write(&p, b"not really a png").unwrap();
        let err = probe_has_alpha(&p).unwrap_err();
        assert!(
            matches!(
                err,
                ShImagesError::Decode(_) | ShImagesError::UnsupportedFormat(_)
            ),
            "corrupt file must map to Decode/UnsupportedFormat, got {err:?}"
        );
    }

    #[test]
    fn probe_has_alpha_missing_file_is_io_error() {
        let dir = tempfile::tempdir().unwrap();
        let err = probe_has_alpha(&dir.path().join("nope.png")).unwrap_err();
        assert!(matches!(err, ShImagesError::Io(_)));
    }

    #[test]
    fn probe_has_alpha_unknown_content_is_unsupported_format() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("data.xyz");
        std::fs::write(&p, b"\x00\x01\x02\x03 not an image").unwrap();
        let err = probe_has_alpha(&p).unwrap_err();
        assert!(matches!(err, ShImagesError::UnsupportedFormat(_)));
    }
}
