//! Platform-specific integration: native file dialogs.

/// Build the async image-open dialog, starting in `start_dir` when given.
///
/// The dialog runs on rfd's dedicated thread (`rfd::AsyncFileDialog`
/// Windows backend spawns its own `std::thread` and completes through a
/// waker), so awaiting `pick_file()` from a `cx.spawn` never pumps the main
/// thread's message loop: gpui's entity lease is released before the dialog
/// blocks, which rules out the `double_lease_panic` a sync modal dialog
/// would cause when gpui's redraw messages arrive mid-pick.
pub fn image_dialog(start_dir: Option<&std::path::Path>) -> rfd::AsyncFileDialog {
    let extensions: Vec<&str> = sh_core::theme::supported_extensions().to_vec();
    let mut dialog = rfd::AsyncFileDialog::new().add_filter("Images", &extensions);
    if let Some(dir) = start_dir {
        dialog = dialog.set_directory(dir);
    }
    dialog
}

/// Build the async folder-pick dialog, starting in `start_dir` when given.
///
/// Same threading contract as [`image_dialog`]: `rfd::AsyncFileDialog` runs
/// on its own thread, so awaiting `pick_folder()` from a `cx.spawn` never
/// pumps the main thread loop (no `double_lease_panic`).
/// VERIFY `pick_folder` + `set_directory` exist on AsyncFileDialog in the
/// locked rfd 0.15.4 source (cargo registry) before using; report file:line.
pub fn folder_dialog(start_dir: Option<&std::path::Path>) -> rfd::AsyncFileDialog {
    let mut dialog = rfd::AsyncFileDialog::new();
    if let Some(dir) = start_dir {
        dialog = dialog.set_directory(dir);
    }
    dialog
}

/// Default crop-output name: `foto.png` → `foto_crop.png`, `a.b.png` →
/// `a.b_crop.png`. Stem is preserved untouched; only the final extension is
/// the plan's responsibility (output is always PNG).
pub fn default_crop_filename(original: &std::path::Path) -> String {
    let stem = original
        .file_stem()
        .map(|s| s.to_string_lossy())
        .unwrap_or_else(|| "image".into());
    format!("{stem}_crop.png")
}

/// Build the async save dialog for a crop, starting in the ORIGINAL image's
/// folder with the `<stem>_crop.png` default name and a PNG-only filter
/// (output is always PNG — the plan's lossless decision).
///
/// Same threading contract as [`image_dialog`]: `rfd::AsyncFileDialog` runs
/// on its own thread (verified rfd 0.15.4 `save_file` at file_dialog.rs:284
/// returning `Future<Output = Option<FileHandle>>`; `FileHandle::path` at
/// file_handle/native.rs:136), so awaiting it from a `cx.spawn` never pumps
/// the main thread loop.
pub fn save_dialog(original: &std::path::Path) -> rfd::AsyncFileDialog {
    let dir = original.parent();
    let name = default_crop_filename(original);
    let mut dialog = rfd::AsyncFileDialog::new()
        .add_filter("PNG image", &["png"])
        .set_file_name(name);
    if let Some(dir) = dir {
        dialog = dialog.set_directory(dir);
    }
    dialog
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn crop_filename_appends_stem_suffix() {
        assert_eq!(
            default_crop_filename(Path::new("C:/pics/foto.png")),
            "foto_crop.png"
        );
    }

    #[test]
    fn crop_filename_handles_multi_dot_names() {
        assert_eq!(default_crop_filename(Path::new("a.b.png")), "a.b_crop.png");
    }

    #[test]
    fn crop_filename_falls_back_for_no_stem() {
        // Path with no usable stem (e.g. ".."): fallback, never panic.
        assert_eq!(default_crop_filename(Path::new("..")), "image_crop.png");
    }
}
