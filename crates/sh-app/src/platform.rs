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
