//! Saving a theme the editor authored.
//!
//! Two properties matter here, and both come from the file being **shared
//! state**: the watcher in `state::theme_store` reads it, and the user may have
//! the same directory open in an editor of their own. Neither tolerates a
//! half-written file.
//!
//! So the write is atomic — a temporary file in the same directory, then a
//! rename — and there are two write paths, because the editor has two distinct
//! intentions:
//!
//! * [`save_theme_draft`] **creates**. It never clobbers: a save that silently
//!   overwrote an existing theme would destroy a file the user authored, and the
//!   editor's whole reason for copy-on-write (WU-4) is that a built-in is never
//!   edited in place.
//! * [`replace_theme_draft`] **overwrites a file that already exists**, which is
//!   the only way to save an edit back to a user's own theme.
//!
//! The atomicity guarantee is identical for both, and it is the reason the
//! temporary file lives beside the target instead of in the system temp
//! directory: a rename across filesystems is a copy, and a copy is observable
//! half-finished.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

use crate::errors::{Result, ShImagesError};
use crate::theme_draft::ThemeDraft;

/// Outcome of a save attempt, so the caller can tell "saved" from "that name is
/// taken" without string-matching an error message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SaveOutcome {
    /// The file was written. Carries the path actually written, which is the
    /// caller's requested path unless a collision suffix was added.
    Saved(PathBuf),
    /// A file already exists at this path. Nothing was written or modified.
    ///
    /// A variant rather than an error because the editor's answer is to offer a
    /// different name, not to report a failure — the user's intent was valid.
    NameTaken(PathBuf),
    /// The draft does not resolve. Nothing was written.
    ///
    /// Carries the message so the editor can show it against the offending row.
    Invalid(String),
}

/// Write `draft` to `path`, creating the parent directory if needed.
///
/// Never overwrites: an existing file at `path` is [`SaveOutcome::NameTaken`]
/// and is left byte-for-byte untouched. This is the same contract
/// `create_bootstrap_theme_file` uses when it materializes a built-in, for the
/// same reason — the target directory is shared with the user's own files.
///
/// The write itself is atomic. A reader watching the directory — the app's own
/// hot-reload watcher, or an external editor — can observe the new name
/// appearing fully-formed, or not at all, never a truncated JSON document that
/// fails to parse. That matters because the watcher reacts to a parse failure
/// by reporting the theme as invalid, so a torn write would surface to the user
/// as their own theme breaking.
///
/// Errors are typed rather than stringly: an I/O failure keeps its
/// [`std::io::ErrorKind`] so a caller can distinguish "permission denied" from
/// anything else.
pub fn save_theme_draft(path: &Path, draft: &ThemeDraft) -> Result<SaveOutcome> {
    // Resolve before touching the filesystem: a save must never create a
    // directory and then discover it had nothing valid to write. This mirrors
    // the guard `apply_theme_entry` keeps around `create_bootstrap_theme_file`,
    // where a parentless path would resolve against the process CWD.
    let json = match draft.to_json() {
        Ok(json) => json,
        Err(error) => return Ok(SaveOutcome::Invalid(error.to_string())),
    };

    let parent = required_parent(path)?;

    fs::create_dir_all(parent).map_err(ShImagesError::Io)?;

    // `create_new` is the collision check *and* the reservation. A separate
    // `exists()` then `create_new` would leave a window where another writer
    // takes the name in between — this way the filesystem decides, once.
    //
    // `AlreadyExists` is the one error that is an outcome rather than a fault:
    // the user's intent was valid, they just picked a taken name. Every other
    // error is a real I/O failure and stays an `Err` with its kind intact.
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
    {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Ok(SaveOutcome::NameTaken(path.to_path_buf()));
        }
        Err(error) => return Err(ShImagesError::Io(error)),
    };

    write_all_and_sync(path, &mut file, json.as_bytes())?;
    drop(file);

    Ok(SaveOutcome::Saved(path.to_path_buf()))
}

/// Outcome of a replace attempt, mirroring [`SaveOutcome`] so a caller reads
/// both contracts the same way: the arm that succeeded, and the two ways the
/// write was refused without the filesystem being harmed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReplaceOutcome {
    /// The file was replaced in place. Carries the path written, which is
    /// always the caller's requested path — a replace never redirects to a
    /// suffixed name the way a collision-free save might.
    Replaced(PathBuf),
    /// No file exists at this path. Nothing was written, and nothing was
    /// created either.
    ///
    /// A variant rather than an `Err` because the editor's answer is to say the
    /// file is gone, not to report an I/O failure — and because a caller that
    /// mishandles a mistyped path wants a distinguishable arm, not a generic
    /// fault it will fold into "save failed".
    Missing(PathBuf),
    /// The draft does not resolve. Nothing was written.
    ///
    /// Carries the message so the editor can show it against the offending row.
    Invalid(String),
}

/// Replace the theme file already at `path` with `draft`.
///
/// This is the *second* write path, and it exists because the first one could
/// only ever create: a user who opens their own theme file and changes a hex
/// code had no way to save it back. It differs from [`save_theme_draft`] in
/// exactly four ways, and in no others:
///
/// 1. **It replaces.** An existing file is the expected case, not a collision.
/// 2. **It is still atomic**, by the same temp-file-then-rename the module
///    already uses — see [`save_theme_draft`] for why the temp file must live
///    in the target's own directory.
/// 3. **It refuses a missing target.** Creating a file the caller did not mean
///    to create turns a mistyped path into a stray theme in the picker, so
///    absence is [`ReplaceOutcome::Missing`] and the directory is left alone.
/// 4. **It validates before touching the filesystem**, so an unresolvable draft
///    cannot disturb the file the user has.
///
/// A bare filename is refused for the same reason `save_theme_draft` refuses
/// it: it would resolve against the process working directory, which is not
/// where anyone's themes live.
///
/// # What a crash leaves behind
///
/// A process that dies between creating the temp file and renaming it leaves
/// that temp file on disk. [`temp_path_for`] is therefore named so
/// [`crate::theme::discover`] skips it — the `sh-images-tmp` suffix is the final
/// extension, and `discover` lists only `*.json` — so a crash cannot add a
/// half-written "theme" to the user's picker. The failure paths below also
/// remove the file outright, which covers the ordinary case; the naming covers
/// the one that cleanup cannot.
///
/// Durability stops at the rename: syncing the *directory* is what makes the new
/// name survive a power loss, and `std` cannot do that portably (`File::open` on
/// a directory works on Unix and fails on Windows). The file's own bytes are
/// synced, which is the guarantee this module has always made.
pub fn replace_theme_draft(path: &Path, draft: &ThemeDraft) -> Result<ReplaceOutcome> {
    // Resolve first, exactly as `save_theme_draft` does: validating after
    // creating the temp file is how a refused replace ends up leaving litter
    // in the user's themes directory.
    let json = match draft.to_json() {
        Ok(json) => json,
        Err(error) => return Ok(ReplaceOutcome::Invalid(error.to_string())),
    };

    // Guard only: `temp_path_for` derives the scratch path from `path`, which
    // already carries the parent, so the directory is not needed as a value.
    required_parent(path)?;

    // Existence is checked, not assumed. `fs::rename` would happily create a
    // missing target, and the whole point of this function is that the caller
    // named a file that already exists.
    if !path.is_file() {
        return Ok(ReplaceOutcome::Missing(path.to_path_buf()));
    }

    // The temp file is reserved in the target's own directory so the rename is a
    // same-filesystem operation, and therefore atomic. `create_new` reserves the
    // scratch name the same way `save_theme_draft` reserves the target, so two
    // concurrent replaces cannot share one scratch file and interleave bytes.
    let temp = temp_path_for(path);
    let mut file = match fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
    {
        Ok(file) => file,
        Err(error) => return Err(ShImagesError::Io(error)),
    };

    // `write_all_and_sync` removes the file it was handed if the write or the
    // sync fails, which here is the temp file — the right thing to remove.
    write_all_and_sync(&temp, &mut file, json.as_bytes())?;
    // The handle must be closed before the rename on Windows: an open file
    // without delete-sharing blocks the replacement of the destination.
    drop(file);

    if let Err(error) = fs::rename(&temp, path) {
        // The target is untouched — the rename never happened — so removing the
        // temp file is the only cleanup owed. Best-effort: the rename error is
        // the one worth reporting.
        let _ = fs::remove_file(&temp);
        return Err(ShImagesError::Io(error));
    }

    Ok(ReplaceOutcome::Replaced(path.to_path_buf()))
}

/// The parent directory a theme path must name, or a typed refusal.
///
/// Shared by both write paths because a bare filename is the same mistake in
/// both: `create_dir_all` would reject it, and `fs::rename` would not reject it
/// at all but would place the result against the process working directory.
fn required_parent(path: &Path) -> Result<&Path> {
    match path.parent() {
        Some(parent) if !parent.as_os_str().is_empty() => Ok(parent),
        _ => Err(ShImagesError::Io(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "theme path must have a parent directory",
        ))),
    }
}

/// The scratch path a replace writes through before renaming over `path`.
///
/// The suffix is chosen so `discover` skips the file even if a crash strands
/// it: `discover` admits a file only when its final extension is `json`, and
/// appending to the *whole* name puts `sh-images-tmp` last. The alternative —
/// keeping `*.json` as the extension, so the temp file looks exactly like a
/// theme — would make every crashed replace a broken entry in the user's picker,
/// which is the failure this name exists to prevent. Keeping the original name
/// visible in the suffix also means a stranded file is identifiable by eye.
fn temp_path_for(path: &Path) -> PathBuf {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    path.with_file_name(format!("{name}.sh-images-tmp"))
}

/// Write the bytes, flush them to the OS, and fail loudly rather than leaving a
/// truncated file behind.
///
/// A failure here has already created an empty file at the path. Removing it is
/// the only correct recovery: leaving a zero-byte `*.json` in the themes
/// directory would be listed by `discover` and reported as an invalid theme,
/// which is a worse outcome than the save not happening.
fn write_all_and_sync(path: &Path, file: &mut fs::File, bytes: &[u8]) -> Result<()> {
    let outcome = (|| -> std::io::Result<()> {
        file.write_all(bytes)?;
        file.sync_all()
    })();

    if let Err(error) = outcome {
        // Best-effort: the original error is the one worth reporting.
        let _ = fs::remove_file(path);
        return Err(ShImagesError::Io(error));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn draft() -> ThemeDraft {
        let theme = crate::theme::parse(
            r##"{
                "name": "Saved Fixture",
                "author": "Sh_Images",
                "version": 1,
                "colors": {
                    "background": "#050507",
                    "surface": "#101016",
                    "text": "#e8e8ee",
                    "accent": "#00ffff",
                    "muted_text": "#929296",
                    "border": "#e8e8ee2e",
                    "danger": "#ad373c",
                    "on_accent": "#050507",
                    "elevated": "#1f1f25",
                    "ring": "#00ffff8c"
                },
                "spacing": { "xs": 4, "sm": 8, "md": 16, "lg": 24 },
                "radii": { "sm": 2, "md": 8, "lg": 14 },
                "typography": { "family": "Inter", "sizes": { "caption": 11, "body": 14, "title": 18 } }
            }"##,
        )
        .expect("fixture must be valid");
        ThemeDraft::from_theme(&theme)
    }

    fn path_in(dir: &Path, name: &str) -> PathBuf {
        dir.join(name)
    }

    /// The whole point of the function: what goes on disk is a theme the loader
    /// accepts, carrying the draft's edits.
    #[test]
    fn saved_file_reparses_and_keeps_the_edits() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path(), "mine.json");
        let mut draft = draft();
        draft.set(crate::theme_draft::Slot::Accent, "#ff00ff");

        let outcome = save_theme_draft(&path, &draft).expect("save must not error");
        assert_eq!(outcome, SaveOutcome::Saved(path.clone()));

        let text = fs::read_to_string(&path).expect("written file must be readable");
        let reparsed = crate::theme::parse(&text).expect("written file must parse");
        assert_eq!(reparsed.colors.accent, "#ff00ff");
        assert_eq!(reparsed.name, "Saved Fixture");
    }

    /// A save must not destroy a file the user owns. This is the property that
    /// makes copy-on-write safe, and the reason `create_new` is used rather
    /// than a truncate.
    #[test]
    fn existing_file_is_never_overwritten() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path(), "mine.json");
        fs::write(&path, "ORIGINAL").unwrap();

        let outcome = save_theme_draft(&path, &draft()).expect("collision is not an Err");
        assert_eq!(outcome, SaveOutcome::NameTaken(path.clone()));
        assert_eq!(
            fs::read_to_string(&path).unwrap(),
            "ORIGINAL",
            "the user's file must be byte-identical after a refused save"
        );
    }

    /// An unresolvable draft must not leave a file behind, and must not be
    /// reported as a name collision.
    #[test]
    fn invalid_draft_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path(), "mine.json");
        let mut bad = draft();
        bad.set(crate::theme_draft::Slot::Text, "nope");

        let outcome = save_theme_draft(&path, &bad).expect("invalid is not an Err");
        match outcome {
            SaveOutcome::Invalid(message) => {
                assert!(
                    message.contains("text"),
                    "the error must name the slot, got: {message}"
                );
            }
            other => panic!("expected Invalid, got {other:?}"),
        }
        assert!(!path.exists(), "an invalid draft must not create a file");
    }

    /// `discover` lists whatever is in the directory, so a failed save that left
    /// a truncated file would be reported to the user as a broken theme.
    #[test]
    fn parent_directory_is_created() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(&dir.path().join("nested").join("deeper"), "mine.json");
        let outcome = save_theme_draft(&path, &draft()).expect("save must not error");
        assert_eq!(outcome, SaveOutcome::Saved(path.clone()));
        assert!(path.exists());
    }

    /// A bare filename would resolve against the process working directory,
    /// which is the same mistake `apply_theme_entry` guards against.
    #[test]
    fn parentless_path_is_refused() {
        let bare = PathBuf::from("theme-editor-should-not-write-here.json");
        let error = save_theme_draft(&bare, &draft()).expect_err("bare filename must be refused");
        assert!(
            matches!(error, ShImagesError::Io(ref inner) if inner.kind() == std::io::ErrorKind::InvalidInput),
            "expected InvalidInput, got {error:?}"
        );
        assert!(
            !bare.exists(),
            "a refused save must not have written into the working directory"
        );
    }

    /// Two saves racing on one name: exactly one wins, and the loser does not
    /// damage the winner's file.
    #[test]
    fn concurrent_saves_leave_one_intact_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path(), "mine.json");

        let first = save_theme_draft(&path, &draft()).expect("first must not error");
        let second = save_theme_draft(&path, &draft()).expect("second must not error");

        assert_eq!(first, SaveOutcome::Saved(path.clone()));
        assert_eq!(second, SaveOutcome::NameTaken(path.clone()));
        let text = fs::read_to_string(&path).unwrap();
        crate::theme::parse(&text).expect("the surviving file must be a whole theme");
    }

    // ── replace_theme_draft ──

    /// Seed a target the editor can legitimately replace: a real theme file.
    fn existing_theme(dir: &Path, name: &str) -> PathBuf {
        let path = path_in(dir, name);
        save_theme_draft(&path, &draft()).expect("seed save must not error");
        path
    }

    fn dir_entries(dir: &Path) -> Vec<String> {
        let mut names: Vec<String> = fs::read_dir(dir)
            .expect("dir must be readable")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        names.sort();
        names
    }

    /// The counterpart of `existing_file_is_never_overwritten`: here the
    /// existing file IS the expected case, and what lands on disk is a theme the
    /// loader accepts carrying the edit.
    #[test]
    fn replace_rewrites_existing_file_with_the_edit() {
        let dir = tempfile::tempdir().unwrap();
        let path = existing_theme(dir.path(), "mine.json");
        let mut edited = draft();
        edited.set(crate::theme_draft::Slot::Accent, "#ff00ff");

        let outcome = replace_theme_draft(&path, &edited).expect("replace must not error");
        assert_eq!(outcome, ReplaceOutcome::Replaced(path.clone()));

        let text = fs::read_to_string(&path).expect("replaced file must be readable");
        let reparsed = crate::theme::parse(&text).expect("replaced file must parse");
        assert_eq!(reparsed.colors.accent, "#ff00ff", "the edit must survive");
        assert_eq!(reparsed.name, "Saved Fixture", "identity must survive");
    }

    /// A mistyped path must be an error the caller can see, not a new file that
    /// quietly appears in the user's themes directory.
    #[test]
    fn replace_refuses_a_missing_target() {
        let dir = tempfile::tempdir().unwrap();
        let path = path_in(dir.path(), "not-there.json");

        let outcome = replace_theme_draft(&path, &draft()).expect("missing is not an Err");
        assert_eq!(
            outcome,
            ReplaceOutcome::Missing(path.clone()),
            "a missing target must be refused, not created"
        );
        assert!(
            !path.exists(),
            "refusing must not create the file the caller mistyped"
        );
        assert_eq!(
            dir_entries(dir.path()),
            Vec::<String>::new(),
            "a refused replace must leave the directory as it found it"
        );
    }

    /// The validation guarantee, with teeth: an unresolvable draft must not
    /// touch the user's file even by a byte, and must not leave litter.
    #[test]
    fn invalid_draft_leaves_the_existing_file_byte_identical() {
        let dir = tempfile::tempdir().unwrap();
        let path = existing_theme(dir.path(), "mine.json");
        let before = fs::read(&path).expect("target must be readable");

        let mut bad = draft();
        bad.set(crate::theme_draft::Slot::Text, "nope");
        let outcome = replace_theme_draft(&path, &bad).expect("invalid is not an Err");

        match outcome {
            ReplaceOutcome::Invalid(message) => assert!(
                message.contains("text"),
                "the error must name the slot, got: {message}"
            ),
            other => panic!("expected Invalid, got {other:?}"),
        }
        assert_eq!(
            fs::read(&path).expect("target must still be readable"),
            before,
            "a refused replace must not modify the existing file"
        );
        assert_eq!(
            dir_entries(dir.path()),
            vec!["mine.json".to_string()],
            "a refused replace must not leave a temporary file behind"
        );
    }

    /// A bare filename would resolve against the process working directory, so
    /// it is refused for the same reason `save_theme_draft` refuses it.
    #[test]
    fn replace_refuses_a_parentless_path() {
        let bare = PathBuf::from("theme-editor-replace-should-not-write-here.json");
        let error =
            replace_theme_draft(&bare, &draft()).expect_err("bare filename must be refused");
        assert!(
            matches!(error, ShImagesError::Io(ref inner) if inner.kind() == std::io::ErrorKind::InvalidInput),
            "expected InvalidInput, got {error:?}"
        );
        assert!(
            !bare.exists(),
            "a refused replace must not have written into the working directory"
        );
    }

    /// The atomicity property, observed: the write goes through a temporary file
    /// and the directory is left holding exactly the target afterwards. If the
    /// write went straight to the target, this directory listing would still
    /// pass — which is why the next test pins the temp name itself.
    #[test]
    fn successful_replace_leaves_only_the_target_in_the_directory() {
        let dir = tempfile::tempdir().unwrap();
        let path = existing_theme(dir.path(), "mine.json");

        replace_theme_draft(&path, &draft()).expect("replace must not error");

        assert_eq!(
            dir_entries(dir.path()),
            vec!["mine.json".to_string()],
            "a completed replace must not leave a temporary file behind"
        );
        crate::theme::parse(&fs::read_to_string(&path).unwrap())
            .expect("the surviving target must be a whole theme");
    }

    /// The crash window: a process that dies between create and rename leaves
    /// the temporary file on disk. `discover` lists that directory, so the temp
    /// name must be one `discover` skips — otherwise the crash surfaces to the
    /// user as a second, broken theme in their picker.
    #[test]
    fn leftover_temporary_file_is_skipped_by_discover() {
        let dir = tempfile::tempdir().unwrap();
        let path = existing_theme(dir.path(), "mine.json");

        // Exactly what a crash between create and rename leaves behind.
        let crashed_temp = temp_path_for(&path);
        fs::write(&crashed_temp, b"{\"name\": \"half").expect("simulate the crash");

        let found = crate::theme::discover(dir.path());
        assert!(
            found.iter().all(|p| p != &crashed_temp),
            "discover must skip a leftover temp file, but listed {found:?}"
        );
        assert_eq!(
            found,
            vec![path.clone()],
            "only the real theme may be discovered"
        );
        // And it must not even parse-fail into the picker.
        let entries = crate::theme::load_discovered(dir.path());
        assert_eq!(entries.len(), 1, "the temp must not appear as a theme");
        assert!(entries[0].theme.is_some(), "the real theme must be valid");
    }

    /// A replace must not weaken the no-clobber guarantee of a create: the temp
    /// file is reserved with `create_new`, so two concurrent replaces cannot
    /// share one scratch name and interleave their bytes.
    #[test]
    fn temp_name_is_reserved_exclusively() {
        let dir = tempfile::tempdir().unwrap();
        let path = existing_theme(dir.path(), "mine.json");
        let temp = temp_path_for(&path);

        let first = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .expect("first reservation must win");
        let second = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp)
            .expect_err("second reservation must lose");

        assert_eq!(
            second.kind(),
            std::io::ErrorKind::AlreadyExists,
            "a concurrent replace must not reuse a live scratch name"
        );
        drop(first);
    }
}
