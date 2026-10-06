//! Saving a theme the editor authored.
//!
//! Two properties matter here, and both come from the file being **shared
//! state**: the watcher in `state::theme_store` reads it, and the user may have
//! the same directory open in an editor of their own. Neither tolerates a
//! half-written file.
//!
//! So the write is atomic — a temporary file in the same directory, then a
//! rename — and it never clobbers. A save that overwrote an existing theme
//! would destroy a file the user authored, and the editor's whole reason for
//! copy-on-write (WU-4) is that a built-in is never edited in place.

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

    let parent = match path.parent() {
        // An empty parent means a bare filename, which `create_dir_all` would
        // reject and which would otherwise write into the CWD.
        Some(parent) if !parent.as_os_str().is_empty() => parent,
        _ => {
            return Err(ShImagesError::Io(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "theme path must have a parent directory",
            )));
        }
    };

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
}
