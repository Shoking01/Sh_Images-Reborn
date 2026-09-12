//! OS clipboard image copy (thin wrapper over arboard).

use sh_core::errors::{Result, ShImagesError};

/// Copy RGBA pixels to the OS clipboard.
///
/// Thin wrapper by design: there is no logic to unit-test headless (the real
/// clipboard only verifies in manual smoke). arboard's `ImageData` is RGBA
/// channel order, matching [`sh_core::decode::DecodedImage`] — no conversion.
pub fn copy_image(img: &sh_core::decode::DecodedImage) -> Result<()> {
    let expected = img.width as usize * img.height as usize * 4;
    if img.rgba.len() != expected || expected == 0 {
        return Err(ShImagesError::Unknown(
            "clipboard buffer size mismatch".into(),
        ));
    }
    let mut clipboard =
        arboard::Clipboard::new().map_err(|e| ShImagesError::Unknown(e.to_string()))?;
    clipboard
        .set_image(arboard::ImageData {
            width: img.width as usize,
            height: img.height as usize,
            bytes: std::borrow::Cow::Borrowed(&img.rgba),
        })
        .map_err(|e| ShImagesError::Unknown(e.to_string()))?;
    Ok(())
}
