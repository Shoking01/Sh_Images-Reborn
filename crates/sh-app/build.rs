//! Build script: embeds the Sh Images application icon into the `sh-app`
//! executable on Windows.
//!
//! Why this exists instead of a workspace-level step: GPUI's Windows backend
//! resolves the window-class icon itself. In `gpui-0.2.2`,
//! `platform::windows::platform::load_icon()` runs
//! `LoadImageW(GetModuleHandleW(None), MAKEINTRESOURCEW(1), IMAGE_ICON, 0, 0,
//! LR_DEFAULTSIZE | LR_SHARED)` and hands the handle to
//! `register_window_class`. `GetModuleHandleW(None)` is the *executable*
//! module, so resource ID 1 must be a member of `sh-app.exe` itself — a
//! resource linked into `libsh_app` would never be found, and the window would
//! fall back to the blank default executable icon.
//!
//! `embed_resource::compile` is used for exactly that reason: when the calling
//! crate has a binary target it emits `cargo:rustc-link-arg-bins`, attaching
//! the compiled `.lib` straight to `sh-app.exe`.
//!
//! Platform behavior: the gate in `main` reads `CARGO_CFG_TARGET_OS` — the
//! *selected target* — and never `cfg(target_os)`, which inside a build script
//! describes the build *host* and would silently drop the icon for a Windows
//! target cross-built from a non-Windows machine. `embed_icon` is therefore
//! ungated as well. When the target is Windows the resource is compiled and a
//! failure is fatal (`.manifest_required()`). The icon is cosmetic, so a
//! missing `rc.exe` or a malformed `.ico` would otherwise be discovered by
//! users as a blank taskbar icon; failing the build keeps the regression in
//! CI. For any other target this script does nothing beyond its rerun
//! annotations.

/// Resource script compiled on Windows. The branding assets live at the
/// repository root so that other crates (and later installer/docs work units)
/// can reach them without depending on `sh-app`; this package is two levels
/// down, hence the `../..` prefix.
const RESOURCE_SCRIPT: &str = "../../assets/branding/sh-images.rc";

/// Icon referenced by `RESOURCE_SCRIPT`. Annotated separately because
/// `embed-resource` emits no rerun annotation of its own for files referenced
/// from a resource script, so without this line editing the artwork would not
/// trigger a rebuild.
const ICON_FILE: &str = "../../assets/branding/sh-images.ico";

fn main() {
    // Emitted on every platform: the annotations are cheap, and they keep the
    // two files from being silently ignored by Cargo's build fingerprint.
    println!("cargo:rerun-if-changed={RESOURCE_SCRIPT}");
    println!("cargo:rerun-if-changed={ICON_FILE}");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows") {
        embed_icon();
    }
}

/// Compiles and links the resource script into this crate's binaries.
///
/// # Panics
///
/// Panics if the resource cannot be compiled. That is the intended contract:
/// `manifest_required()` turns a missing resource compiler or an unreadable
/// `.ico` into a build failure instead of a silently iconless executable.
fn embed_icon() {
    use std::{env, path::PathBuf};

    // Anchored on CARGO_MANIFEST_DIR rather than the working directory: the
    // resource script path is handed to `RC.EXE` verbatim, so it has to be
    // right no matter where the build script is launched from.
    let manifest_dir = PathBuf::from(
        env::var("CARGO_MANIFEST_DIR").expect("build scripts always receive CARGO_MANIFEST_DIR"),
    );
    let script = manifest_dir.join(RESOURCE_SCRIPT);

    // `embed_resource::compile` decides between `rustc-link-arg-bins` (attach
    // the resource to the executable) and `rustc-link-lib` (attach it to the
    // library) by probing for `Cargo.toml` / `src/main.rs` *relative to the
    // working directory*. Anchoring that probe on the package root is what
    // keeps the icon in `sh-app.exe` instead of quietly landing in
    // `libsh_app`, where `GetModuleHandleW(None)` would never find it.
    let previous_dir = env::current_dir().ok();
    env::set_current_dir(&manifest_dir).expect("package root must be a usable working directory");

    let result = embed_resource::compile(&script, embed_resource::NONE).manifest_required();

    if let Some(previous_dir) = previous_dir {
        let _ = env::set_current_dir(previous_dir);
    }

    result.unwrap_or_else(|result| {
        panic!(
            "could not embed the Sh Images icon: compiling {RESOURCE_SCRIPT} \
             (which references {ICON_FILE}) failed with {result:?}"
        )
    });
}
