//! Platform-specific integration: native file dialogs.

use std::path::PathBuf;

/// Open a native file dialog and return the selected image path, if any.
///
/// V1 uses rfd's SYNC modal dialog on the main thread (plan-approved
/// deviation): rfd's Windows backend pumps its own message loop while the
/// dialog is up, so the OS keeps painting our window, and a modal picker is
/// the expected UX for an explicit Ctrl+O. The app has no background work
/// that could stall during the pick. An async variant is a V2 concern once
/// background decoding lands.
pub fn pick_image() -> Option<PathBuf> {
    let extensions: Vec<&str> = sh_core::theme::supported_extensions().to_vec();
    rfd::FileDialog::new()
        .add_filter("Images", &extensions)
        .pick_file()
}
