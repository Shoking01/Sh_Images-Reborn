//! App view state: Welcome → Grid → Viewer.

use std::path::{Path, PathBuf};

/// The three app views.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum View {
    /// Startup screen (always first unless a CLI path was given).
    #[default]
    Welcome,
    /// Folder thumbnail grid.
    Grid,
    /// Single-image viewer.
    Viewer,
}

/// Where startup should land.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StartupTarget {
    Welcome,
    Grid,
    Viewer,
}

/// How the CLI arg was classified by the caller (file vs dir, via fs check).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum CliKind {
    File,
    Dir,
}

/// Pure startup routing: CLI file → Viewer, CLI dir → Grid, else Welcome.
/// The caller (main.rs) classifies the CLI path with `is_dir`/`is_file`
/// BEFORE calling, so this stays pure and headless-testable.
pub fn startup_view(cli: Option<CliKind>) -> StartupTarget {
    match cli {
        Some(CliKind::File) => StartupTarget::Viewer,
        Some(CliKind::Dir) => StartupTarget::Grid,
        None => StartupTarget::Welcome,
    }
}

/// Welcome screen variant. `Continue` only when the dir still exists —
/// a stale `last_dir` (deleted/unplugged drive) shows Fresh instead of a
/// dead button. Callers refresh this after every folder open.
#[derive(Debug, Clone, PartialEq)]
pub enum WelcomeVariant {
    Continue(PathBuf),
    Fresh,
}

pub fn welcome_variant(last_dir: Option<&Path>) -> WelcomeVariant {
    match last_dir {
        Some(dir) if dir.is_dir() => WelcomeVariant::Continue(dir.to_path_buf()),
        _ => WelcomeVariant::Fresh,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn startup_with_cli_file_goes_viewer() {
        assert_eq!(startup_view(Some(CliKind::File)), StartupTarget::Viewer);
    }

    #[test]
    fn startup_with_cli_dir_goes_grid() {
        assert_eq!(startup_view(Some(CliKind::Dir)), StartupTarget::Grid);
    }

    #[test]
    fn startup_without_cli_goes_welcome() {
        assert_eq!(startup_view(None), StartupTarget::Welcome);
    }

    #[test]
    fn welcome_variant_needs_existing_dir() {
        assert_eq!(welcome_variant(None), WelcomeVariant::Fresh);
        assert_eq!(
            welcome_variant(Some(&PathBuf::from("Z:\\no\\existe"))),
            WelcomeVariant::Fresh
        );
    }
}
