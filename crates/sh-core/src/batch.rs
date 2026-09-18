//! Batch file operations for multi-selection: move + recycle-bin delete.
//!
//! Per-file results (never all-or-nothing): every function returns a
//! [`BatchReport`] so callers can keep exactly the un-moved paths selected
//! and report counts. Pure tempdir-testable; the OS bin itself is smoke.

use std::path::{Path, PathBuf};

/// Per-file outcome of a batch operation.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct BatchReport {
    /// Files successfully moved (or trashed).
    pub moved: Vec<PathBuf>,
    /// Move-only: destination already had the name — untouched both sides.
    pub skipped_existing: Vec<PathBuf>,
    /// Files that errored, with the OS message.
    pub failed: Vec<(PathBuf, String)>,
}

/// Move `paths` into `dest`. Never overwrites: an existing destination name
/// skips. `rename` first; on failure (notably cross-device C:→D:) falls back
/// to copy + trash-original, preserving reversibility end-to-end. Any other
/// failure lands in `failed` with the source untouched.
pub fn move_paths(paths: &[PathBuf], dest: &Path) -> BatchReport {
    let mut report = BatchReport::default();
    for src in paths {
        let Some(name) = src.file_name() else {
            report.failed.push((src.clone(), "no file name".into()));
            continue;
        };
        let target = dest.join(name);
        if target.exists() {
            report.skipped_existing.push(src.clone());
            continue;
        }
        match std::fs::rename(src, &target) {
            Ok(()) => report.moved.push(src.clone()),
            Err(rename_err) => match copy_then_trash(src, &target) {
                Ok(()) => report.moved.push(src.clone()),
                Err(_) => report.failed.push((src.clone(), rename_err.to_string())),
            },
        }
    }
    report
}

/// Copy `src` to `dest`, then move the source to the recycle bin. The
/// cross-device fallback behind [`move_paths`]; unit-tested directly (a CI
/// tempdir pair across drives is not expressible — only the trigger differs,
/// and the trigger is a 3-line branch).
fn copy_then_trash(src: &Path, dest: &Path) -> std::io::Result<()> {
    std::fs::copy(src, dest)?;
    trash::delete(src).map_err(std::io::Error::other)?;
    Ok(())
}

/// Move `paths` to the OS recycle bin. Reversible by definition; whether the
/// OS actually bins is manual-smoke — headless tests assert `Ok` + source
/// gone (the crate is silent on success either way).
pub fn trash_paths(paths: &[PathBuf]) -> BatchReport {
    let mut report = BatchReport::default();
    for src in paths {
        match trash::delete(src) {
            Ok(()) => report.moved.push(src.clone()),
            Err(e) => report.failed.push((src.clone(), e.to_string())),
        }
    }
    report
}

/// Human report line for a finished op, or `None` on full success (silent
/// is the standard). Pure so the wording is unit-testable; the verb is
/// typed (`BatchVerb`) and the language owns its full sentence via the
/// `i18n` templates, so Spanish word order is free to differ.
pub fn format_report(
    lang: crate::i18n::Language,
    verb: crate::i18n::BatchVerb,
    total: usize,
    report: &BatchReport,
) -> Option<String> {
    let skipped = report.skipped_existing.len();
    let failed = report.failed.len();
    if skipped == 0 && failed == 0 {
        return None;
    }
    let first_failed = report
        .failed
        .first()
        .and_then(|(p, _)| p.file_name())
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    crate::i18n::batch_report(
        lang,
        verb,
        report.moved.len(),
        total,
        skipped,
        failed,
        &first_failed,
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{BatchVerb, Language};
    use tempfile::tempdir;

    fn fixture(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        std::fs::write(&p, b"stub").unwrap();
        p
    }

    #[test]
    fn move_all_ok_relocates_everything() {
        let root = tempdir().unwrap();
        let src = root.path().join("src");
        let dst = root.path().join("dst");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        let a = fixture(&src, "a.png");
        let b = fixture(&src, "b.png");
        let report = move_paths(&[a.clone(), b.clone()], &dst);
        assert_eq!(report.moved.len(), 2);
        assert!(report.skipped_existing.is_empty());
        assert!(report.failed.is_empty());
        assert!(!a.exists() && !b.exists());
        assert!(dst.join("a.png").exists() && dst.join("b.png").exists());
    }

    #[test]
    fn move_collision_skips_without_touching_either_side() {
        let root = tempdir().unwrap();
        let src = root.path().join("src");
        let dst = root.path().join("dst");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        let a = fixture(&src, "a.png");
        let conflict = fixture(&dst, "a.png");
        std::fs::write(&conflict, b"original-bytes").unwrap();
        let report = move_paths(std::slice::from_ref(&a), &dst);
        assert_eq!(report.skipped_existing, vec![a.clone()]);
        assert!(report.moved.is_empty());
        assert!(a.exists(), "source untouched");
        assert_eq!(
            std::fs::read(&conflict).unwrap(),
            b"original-bytes",
            "destination untouched"
        );
    }

    #[test]
    fn move_missing_source_fails_rest_moves() {
        let root = tempdir().unwrap();
        let src = root.path().join("src");
        let dst = root.path().join("dst");
        std::fs::create_dir_all(&src).unwrap();
        std::fs::create_dir_all(&dst).unwrap();
        let a = fixture(&src, "a.png");
        let ghost = src.join("ghost.png");
        let report = move_paths(&[ghost.clone(), a.clone()], &dst);
        assert_eq!(report.moved, vec![a.clone()]);
        assert_eq!(report.failed.len(), 1);
        assert_eq!(report.failed[0].0, ghost);
    }

    #[test]
    fn copy_then_trash_relocates_reversibly() {
        let root = tempdir().unwrap();
        let a = fixture(root.path(), "a.png");
        let target = root.path().join("sub").join("a.png");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        copy_then_trash(&a, &target).unwrap();
        assert!(target.exists());
        assert!(!a.exists(), "source trashed, not left behind");
    }

    #[test]
    fn trash_ok_removes_source() {
        let root = tempdir().unwrap();
        let a = fixture(root.path(), "a.png");
        let report = trash_paths(std::slice::from_ref(&a));
        assert_eq!(report.moved, vec![a.clone()]);
        assert!(!a.exists());
    }

    #[test]
    fn trash_missing_source_reports_failed() {
        let root = tempdir().unwrap();
        let ghost = root.path().join("ghost.png");
        let report = trash_paths(std::slice::from_ref(&ghost));
        assert!(report.moved.is_empty());
        assert_eq!(report.failed.len(), 1);
    }

    #[test]
    fn format_report_silent_on_full_success() {
        let report = BatchReport {
            moved: vec![PathBuf::from("x")],
            ..BatchReport::default()
        };
        assert_eq!(
            format_report(Language::En, BatchVerb::Moved, 1, &report),
            None,
            "`None` on full success is preserved in both languages"
        );
        assert_eq!(
            format_report(Language::Es, BatchVerb::Deleted, 1, &report),
            None
        );
    }

    #[test]
    fn format_report_names_skips_and_failures_in_english() {
        let report = BatchReport {
            moved: vec![PathBuf::from("a")],
            skipped_existing: vec![PathBuf::from("b")],
            failed: vec![(PathBuf::from("c.png"), "boom".into())],
        };
        assert_eq!(
            format_report(Language::En, BatchVerb::Moved, 3, &report),
            Some("Moved 1 of 3 — 1 skipped (already existed), 1 failed (c.png)".into())
        );
    }

    /// S5: the Spanish report owns its full sentence (word order free to
    /// differ from English) with counts + filename passed through untranslated.
    #[test]
    fn format_report_renders_full_es_sentence() {
        let report = BatchReport {
            moved: vec![PathBuf::from("a")],
            skipped_existing: vec![PathBuf::from("b")],
            failed: vec![(PathBuf::from("c.png"), "boom".into())],
        };
        assert_eq!(
            format_report(Language::Es, BatchVerb::Deleted, 3, &report),
            // Native-speaker review: numeral 1 takes singular verb agreement.
            Some("Se eliminó 1 de 3: 1 omitido (ya existía), 1 con error (c.png)".into())
        );
        assert_eq!(
            format_report(Language::Es, BatchVerb::Moved, 2, &report),
            Some("Se movió 1 de 2: 1 omitido (ya existía), 1 con error (c.png)".into())
        );
    }
}
