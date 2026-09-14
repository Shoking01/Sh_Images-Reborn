//! Recent-folders list logic: pure, total, no I/O.
//!
//! The caller owns filesystem checks (`is_dir`, "has images"); this module
//! only manipulates the ordered path list and derives chip labels.

use std::path::{Path, PathBuf};

/// Maximum remembered folders (product decision: 5 — see the V3 spec).
pub const RECENT_DIRS_MAX: usize = 5;

/// Prepend `path`, deduping (an existing copy anywhere in the list moves to
/// the front), trimmed to [`RECENT_DIRS_MAX`]. Total: works for any input
/// list, including hand-edited settings with duplicates.
pub fn push_recent(dirs: &[PathBuf], path: PathBuf) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::with_capacity(RECENT_DIRS_MAX);
    out.push(path);
    for d in dirs {
        if !out.contains(d) {
            out.push(d.clone());
        }
    }
    out.truncate(RECENT_DIRS_MAX);
    out
}

/// Chip label: `parent_name\name` (e.g. `Trabajo\Portfolio`). Falls back to
/// the path's own display when the parent is a filesystem root (a direct
/// child of `C:\` is short and unambiguous already), when there is no
/// parent, or when either component lacks a `file_name` (trailing separators).
pub fn display_name(path: &Path) -> String {
    let Some(parent) = path.parent() else {
        return path.display().to_string();
    };
    if parent.parent().is_none() {
        return path.display().to_string();
    }
    match (parent.file_name(), path.file_name()) {
        (Some(p), Some(n)) => format!("{}\\{}", p.to_string_lossy(), n.to_string_lossy()),
        _ => path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn p(s: &str) -> PathBuf {
        PathBuf::from(s)
    }

    // ── push_recent ──

    #[test]
    fn push_on_empty_seeds_single() {
        let out = push_recent(&[], p("C:\\a"));
        assert_eq!(out, vec![p("C:\\a")]);
    }

    #[test]
    fn push_new_path_prepends() {
        let out = push_recent(&[p("C:\\a")], p("C:\\b"));
        assert_eq!(out, vec![p("C:\\b"), p("C:\\a")]);
    }

    #[test]
    fn push_duplicate_at_front_is_stable() {
        let out = push_recent(&[p("C:\\a"), p("C:\\b")], p("C:\\a"));
        assert_eq!(out, vec![p("C:\\a"), p("C:\\b")]);
    }

    #[test]
    fn push_duplicate_at_back_moves_to_front() {
        let out = push_recent(&[p("C:\\a"), p("C:\\b")], p("C:\\b"));
        assert_eq!(out, vec![p("C:\\b"), p("C:\\a")]);
    }

    #[test]
    fn push_duplicate_in_middle_moves_to_front() {
        let out = push_recent(&[p("C:\\a"), p("C:\\b"), p("C:\\c")], p("C:\\b"));
        assert_eq!(out, vec![p("C:\\b"), p("C:\\a"), p("C:\\c")]);
    }

    #[test]
    fn push_trims_to_max_evicting_oldest() {
        // List as it exists after opening d1..d5 in order (most-recent-first):
        // [d5, d4, d3, d2, d1] — d1 is the OLDEST (opened first).
        let dirs: Vec<PathBuf> = (1..=5).rev().map(|i| p(&format!("C:\\d{i}"))).collect();
        // Pushing a 6th evicts the oldest (d1), keeps the newest 5.
        let out = push_recent(&dirs, p("C:\\new"));
        assert_eq!(out.len(), RECENT_DIRS_MAX);
        assert_eq!(out[0], p("C:\\new"));
        assert_eq!(out[4], p("C:\\d2"));
        assert!(!out.contains(&p("C:\\d1")));
    }

    #[test]
    fn push_dedupes_hand_edited_duplicates() {
        let out = push_recent(&[p("C:\\a"), p("C:\\a"), p("C:\\b")], p("C:\\a"));
        assert_eq!(out, vec![p("C:\\a"), p("C:\\b")]);
    }

    // ── display_name ──

    #[test]
    fn display_name_joins_parent_and_name() {
        assert_eq!(display_name(Path::new("C:\\a\\b")), "a\\b");
    }

    #[test]
    fn display_name_drive_root_child_falls_back_to_full_display() {
        // `C:\Fotos`: parent is `C:\` (a root) — the full display is short
        // and loses nothing.
        assert_eq!(display_name(Path::new("C:\\Fotos")), "C:\\Fotos");
    }

    #[test]
    fn display_name_root_uses_full_display() {
        assert_eq!(display_name(Path::new("C:\\")), "C:\\");
    }

    #[test]
    fn display_name_no_parent_uses_full_display() {
        assert_eq!(display_name(Path::new("some-dir")), "some-dir");
    }
}
