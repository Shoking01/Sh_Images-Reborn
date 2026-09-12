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
