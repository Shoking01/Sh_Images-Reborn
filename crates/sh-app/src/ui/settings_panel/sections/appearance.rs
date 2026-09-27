//! Appearance section: theme picker (moved here from the old topbar
//! dropdown) and persisted viewer display controls.

use sh_core::settings::{SLIDESHOW_INTERVAL_MAX_SECS, SLIDESHOW_INTERVAL_MIN_SECS};
use sh_core::theme::Theme;
use std::path::{Path, PathBuf};

/// Stable element ID for the filmstrip toggle row.
pub const FILMSTRIP_TOGGLE_ID: &str = "settings-filmstrip-toggle";
/// Stable element ID for the transparency checkerboard toggle row.
pub const CHECKERBOARD_TOGGLE_ID: &str = "settings-checkerboard-toggle";
/// Stable element ID for the reduced-motion toggle row.
pub const REDUCE_MOTION_TOGGLE_ID: &str = "settings-reduce-motion-toggle";
/// Stable element ID for the slideshow interval row.
pub const SLIDESHOW_INTERVAL_ROW_ID: &str = "settings-slideshow-interval";
/// Stable element ID for the interval decrement button.
pub const SLIDESHOW_INTERVAL_DECREMENT_ID: &str = "settings-slideshow-interval-decrement";
/// Stable element ID for the persisted interval value.
pub const SLIDESHOW_INTERVAL_VALUE_ID: &str = "settings-slideshow-interval-value";
/// Stable element ID for the interval increment button.
pub const SLIDESHOW_INTERVAL_INCREMENT_ID: &str = "settings-slideshow-interval-increment";
/// Number of persisted controls below the theme picker.
pub const APPEARANCE_SETTING_ROW_COUNT: usize = 4;

/// One row in the theme picker, built-in or user-supplied.
///
/// Owned by `App` (not rebuilt per render) so the row list, the row COUNT
/// the scroll geometry is derived from, and the wheel handler's clamp all
/// read the same value — see [`super::super::scroll::appearance_content_h`].
#[derive(Debug, Clone, PartialEq)]
pub struct ThemeEntry {
    /// Key persisted as `Settings::theme`, and the filename on disk.
    pub file_name: String,
    /// Absolute path a hot-reload watcher must follow for this theme.
    pub path: PathBuf,
    /// Parsed theme, or `None` when the underlying file is invalid. A
    /// `None` entry still renders a row (dimmed, not clickable) so a
    /// broken file is visibly present rather than silently missing.
    pub theme: Option<Theme>,
    /// The exact file text that produced `theme`. Carried so applying an
    /// entry can seed the hot-reload dedupe baseline WITHOUT re-reading
    /// the file — the watcher compares its first read against this string,
    /// and a re-derived value would make it re-apply once on the first tick.
    pub text: String,
    /// Theme JSON to (re)write to `path` before applying, when this entry
    /// is built-in-backed and the file is missing. Mirrors the startup
    /// bootstrap contract: the picker only ever applies a theme that has a
    /// real file, because a theme with no file cannot hot-reload.
    pub bootstrap_json: Option<String>,
}

impl ThemeEntry {
    /// Whether this row can be applied at all. A `None` theme means the
    /// file failed validation, so there is nothing to apply.
    pub fn is_selectable(&self) -> bool {
        self.theme.is_some()
    }

    /// Label for the row: the theme's own name, falling back to the file
    /// name when the file is invalid (there is no parsed name to show).
    pub fn display_name(&self) -> String {
        self.theme
            .as_ref()
            .map(|t| t.name.clone())
            .unwrap_or_else(|| self.file_name.clone())
    }
}

/// Build the picker rows: the built-ins, with user themes merged in.
///
/// Pure and total — no I/O, no GPUI — so the ordering, dedupe, and
/// invalid-file rules are unit-testable without a window.
///
/// # Ordering: built-ins first, then user themes
///
/// The built-ins are the curated set that always parses, and the active
/// theme is usually one of them, so listing them first puts the ✓ marker
/// near the top for the common case. User themes follow in the
/// `discover()` (sorted-by-path) order, which is stable across restarts —
/// a picker that reorders itself between runs is worse than one with a
/// fixed, if arbitrary, tail.
///
/// # Dedupe: by theme NAME, and the USER'S FILE WINS
///
/// [`crate::state::theme_store::theme_startup`] writes a copy of the
/// active built-in into the user theme directory, and
/// `App::apply_theme_entry` does the same for every built-in the user ever
/// picks. So on any machine that has run once, discovery finds a copy of a
/// built-in and naive listing shows that theme twice. When a discovered
/// theme's name matches a built-in's name, the built-in row is REPLACED IN
/// PLACE by the discovered one.
///
/// In-place replacement rather than append-and-drop, because the user's
/// copy is the one worth keeping: it is the file the app deliberately
/// creates so the theme stays EDITABLE, and it is the file the hot-reload
/// watcher can watch. Preferring the built-in would leave the editable copy
/// invisible, which defeats the entire reason the copy exists.
///
/// The known cost: two genuinely different themes that declare the same
/// `name` collapse to one row. That is a user-authored collision in a
/// human-readable label, it is documented rather than silently wrong, and
/// renaming either file resolves it. Dedupe on file identity instead would
/// NOT work — the duplicate IS a different file with identical content.
pub fn merge_theme_entries(
    themes_dir: &Path,
    discovered: &[sh_core::theme::DiscoveredTheme],
) -> Vec<ThemeEntry> {
    let mut entries: Vec<ThemeEntry> = crate::theme_builtins::BUILTIN_THEMES
        .iter()
        .map(|(file, json)| ThemeEntry {
            file_name: (*file).to_string(),
            path: themes_dir.join(file),
            theme: sh_core::theme::parse(json).ok(),
            text: (*json).to_string(),
            bootstrap_json: Some((*json).to_string()),
        })
        .collect();

    for found in discovered {
        // An invalid file has no name to match on, so it can never displace
        // a row IN PLACE: it is always APPENDED as its own entry, leaving
        // every working theme untouched. Its position within the user block
        // still follows `discover()`'s sorted-by-path order.
        let key = found.theme.as_ref().map(|t| t.name.trim().to_lowercase());
        let entry = ThemeEntry {
            file_name: found.file_name.clone(),
            path: found.path.clone(),
            theme: found.theme.clone(),
            text: found.text.clone(),
            // A discovered file already exists on disk by construction.
            bootstrap_json: None,
        };
        match key.and_then(|k| {
            entries
                .iter()
                .position(|e| theme_name_key(e) == Some(k.clone()))
        }) {
            Some(idx) => entries[idx] = entry,
            None => entries.push(entry),
        }
    }
    entries
}

/// The dedupe key for a row: its parsed theme name, normalized.
///
/// `None` for a row whose file is invalid, which is what keeps a broken
/// file from silently replacing a working one.
fn theme_name_key(entry: &ThemeEntry) -> Option<String> {
    entry.theme.as_ref().map(|t| t.name.trim().to_lowercase())
}

/// Built-in rows only, with no filesystem access at all.
///
/// Used to seed `App` so the first Settings open never renders an empty
/// picker while discovery is in flight, and so a failed discovery still
/// leaves the four themes the app has always shipped.
pub fn builtin_theme_entries(themes_dir: &Path) -> Vec<ThemeEntry> {
    merge_theme_entries(themes_dir, &[])
}

/// Adjust a slideshow interval while keeping the persisted inclusive bounds.
pub fn adjust_slideshow_interval(current: u32, delta: i32) -> u32 {
    let adjusted = if delta < 0 {
        current.saturating_sub(delta.unsigned_abs())
    } else {
        current.saturating_add(delta as u32)
    };
    adjusted.clamp(SLIDESHOW_INTERVAL_MIN_SECS, SLIDESHOW_INTERVAL_MAX_SECS)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn settings_rows_use_stable_unique_element_ids() {
        let ids = [
            FILMSTRIP_TOGGLE_ID,
            CHECKERBOARD_TOGGLE_ID,
            REDUCE_MOTION_TOGGLE_ID,
            SLIDESHOW_INTERVAL_ROW_ID,
            SLIDESHOW_INTERVAL_DECREMENT_ID,
            SLIDESHOW_INTERVAL_VALUE_ID,
            SLIDESHOW_INTERVAL_INCREMENT_ID,
        ];

        for (index, id) in ids.iter().enumerate() {
            assert!(id.starts_with("settings-"));
            assert!(!ids[index + 1..].contains(id), "duplicate element id: {id}");
        }
    }

    #[test]
    fn settings_slideshow_interval_adjustment_stays_within_bounds() {
        assert_eq!(adjust_slideshow_interval(3, -1), 2);
        assert_eq!(adjust_slideshow_interval(3, 1), 4);
        assert_eq!(adjust_slideshow_interval(1, -1), 1);
        assert_eq!(adjust_slideshow_interval(60, 1), 60);
        assert_eq!(adjust_slideshow_interval(30, i32::MIN), 1);
        assert_eq!(adjust_slideshow_interval(30, i32::MAX), 60);
    }

    #[test]
    fn reduce_motion_row_has_a_stable_id_and_counts_as_a_setting_row() {
        assert_eq!(REDUCE_MOTION_TOGGLE_ID, "settings-reduce-motion-toggle");
        assert_eq!(APPEARANCE_SETTING_ROW_COUNT, 4);
    }

    // ── Theme discovery: dedupe, ordering, invalid files ──

    mod discovery {
        use super::*;

        /// A minimal valid theme with a caller-chosen `name`, so dedupe can
        /// be driven by identity rather than by reusing the built-ins.
        fn theme_json(name: &str) -> String {
            format!(
                r##"{{
                "name": "{name}",
                "author": "test",
                "version": 1,
                "colors": {{ "background": "#000", "surface": "#111", "text": "#fff", "accent": "#0f0" }},
                "spacing": {{ "xs": 4, "sm": 8, "md": 16, "lg": 24 }},
                "radii": {{ "sm": 2, "md": 6, "lg": 12 }},
                "typography": {{ "family": "Inter", "sizes": {{ "caption": 11, "body": 14, "title": 18 }} }}
            }}"##
            )
        }

        fn write(dir: &Path, name: &str, body: &str) -> PathBuf {
            let p = dir.join(name);
            std::fs::write(&p, body).expect("fixture write");
            p
        }

        fn names(entries: &[ThemeEntry]) -> Vec<String> {
            entries.iter().map(|e| e.display_name()).collect()
        }

        /// With no user files, the picker is exactly the built-ins, in
        /// `BUILTIN_THEMES` order — the pre-existing behavior.
        #[test]
        fn builtin_only_listing_is_the_builtin_order() {
            let entries = builtin_theme_entries(Path::new("/themes"));
            assert_eq!(entries.len(), crate::theme_builtins::BUILTIN_THEMES.len());
            for (entry, (file, json)) in entries
                .iter()
                .zip(crate::theme_builtins::BUILTIN_THEMES.iter())
            {
                assert_eq!(&entry.file_name, file);
                assert_eq!(entry.path, Path::new("/themes").join(file));
                assert_eq!(&entry.text, json);
                assert!(entry.is_selectable());
            }
        }

        /// User themes appear AFTER the built-ins, not interleaved.
        #[test]
        fn user_themes_follow_the_builtins() {
            let dir = tempfile::tempdir().unwrap();
            write(dir.path(), "mine.json", &theme_json("Mine"));
            let found = sh_core::theme::load_discovered(dir.path());

            let entries = merge_theme_entries(dir.path(), &found);

            let builtins = crate::theme_builtins::BUILTIN_THEMES.len();
            assert_eq!(entries.len(), builtins + 1);
            assert_eq!(&entries[builtins].display_name(), "Mine");
            assert!(entries[..builtins]
                .iter()
                .all(|e| e.bootstrap_json.is_some()));
        }

        /// The `theme_startup` duplicate. A user-directory copy of a
        /// built-in must collapse into ONE row that points at the USER
        /// FILE — preferring the built-in would make the editable,
        /// hot-reloadable copy invisible, which is the whole reason the app
        /// creates it.
        #[test]
        fn builtin_copied_into_the_user_dir_dedupes_onto_the_user_file() {
            let dir = tempfile::tempdir().unwrap();
            let (builtin_file, builtin_json) = crate::theme_builtins::BUILTIN_THEMES[0];
            let user_path = write(dir.path(), builtin_file, builtin_json);
            let found = sh_core::theme::load_discovered(dir.path());

            let entries = merge_theme_entries(dir.path(), &found);

            assert_eq!(
                entries.len(),
                crate::theme_builtins::BUILTIN_THEMES.len(),
                "the copy must not add a row"
            );
            let idx = entries
                .iter()
                .position(|e| e.file_name == *builtin_file)
                .expect("builtin row survives");
            assert_eq!(entries[idx].path, user_path, "the user's file must win");
            assert_eq!(
                entries[idx].bootstrap_json, None,
                "a discovered file needs no bootstrap write"
            );
            // In place, so the ordering claim still holds.
            assert_eq!(idx, 0);
        }

        /// The dedupe key is the theme NAME, not the filename: a renamed copy
        /// of a built-in still collapses onto it.
        #[test]
        fn dedupe_follows_the_theme_name_not_the_filename() {
            let dir = tempfile::tempdir().unwrap();
            let (builtin_file, builtin_json) = crate::theme_builtins::BUILTIN_THEMES[0];
            let builtin_name = sh_core::theme::parse(builtin_json).unwrap().name;
            let renamed = write(dir.path(), "totally-different-name.json", builtin_json);
            let found = sh_core::theme::load_discovered(dir.path());

            let entries = merge_theme_entries(dir.path(), &found);

            assert_eq!(entries.len(), crate::theme_builtins::BUILTIN_THEMES.len());
            assert_eq!(entries[0].file_name, "totally-different-name.json");
            assert_eq!(entries[0].path, renamed);
            assert_eq!(entries[0].display_name(), builtin_name);
            assert_ne!(entries[0].file_name.as_str(), builtin_file);
        }

        /// Name matching is case- and whitespace-insensitive, so a
        /// cosmetically different copy still dedupes instead of showing the
        /// same theme twice.
        #[test]
        fn dedupe_ignores_name_case_and_padding() {
            let dir = tempfile::tempdir().unwrap();
            let (_, builtin_json) = crate::theme_builtins::BUILTIN_THEMES[0];
            let name = sh_core::theme::parse(builtin_json).unwrap().name;
            write(dir.path(), "copy.json", &theme_json(&name.to_uppercase()));
            let found = sh_core::theme::load_discovered(dir.path());

            let entries = merge_theme_entries(dir.path(), &found);

            assert_eq!(entries.len(), crate::theme_builtins::BUILTIN_THEMES.len());
        }

        /// One broken file must not break the picker: every valid theme is
        /// still listed, and the broken one is PRESENT as an unselectable
        /// row rather than silently dropped.
        #[test]
        fn one_invalid_theme_leaves_the_rest_of_the_picker_intact() {
            let dir = tempfile::tempdir().unwrap();
            write(dir.path(), "broken.json", "{ this is not json");
            write(dir.path(), "good.json", &theme_json("Good"));
            let found = sh_core::theme::load_discovered(dir.path());
            assert_eq!(found.iter().filter(|d| d.error.is_some()).count(), 1);

            let entries = merge_theme_entries(dir.path(), &found);

            // All four built-ins plus the good user theme, all selectable.
            let selectable = entries.iter().filter(|e| e.is_selectable()).count();
            assert_eq!(selectable, crate::theme_builtins::BUILTIN_THEMES.len() + 1);
            assert!(names(&entries).contains(&"Good".to_string()));
            // The broken file is visible but inert, and it displaced nothing.
            let broken = entries
                .iter()
                .find(|e| e.file_name == "broken.json")
                .expect("a broken file still gets a row");
            assert!(!broken.is_selectable());
            assert_eq!(
                broken.display_name(),
                "broken.json",
                "falls back to filename"
            );
            assert_eq!(broken.theme, None);
            // Every built-in row is still a working, built-in-backed entry.
            let builtins = crate::theme_builtins::BUILTIN_THEMES.len();
            for (entry, (file, _)) in entries
                .iter()
                .take(builtins)
                .zip(crate::theme_builtins::BUILTIN_THEMES.iter())
            {
                assert_eq!(&entry.file_name, file);
                assert!(entry.is_selectable());
            }
            // And the two user rows sit after them, in path-sorted order.
            let user: Vec<&str> = entries[builtins..]
                .iter()
                .map(|e| e.file_name.as_str())
                .collect();
            assert_eq!(user, vec!["broken.json", "good.json"]);
        }

        /// A file that parses as JSON but fails theme validation is the
        /// same case as unparseable: present, inert, explained by its name.
        #[test]
        fn schema_invalid_theme_is_listed_but_unselectable() {
            let dir = tempfile::tempdir().unwrap();
            // Valid JSON, invalid theme: no name / bad color.
            write(dir.path(), "schema-bad.json", r#"{"name": ""}"#);
            let found = sh_core::theme::load_discovered(dir.path());
            let entries = merge_theme_entries(dir.path(), &found);

            let bad = entries
                .iter()
                .find(|e| e.file_name == "schema-bad.json")
                .expect("row exists");
            assert!(!bad.is_selectable());
            assert_eq!(bad.theme, None);
        }

        /// An empty or missing directory is the normal first-run state, not
        /// an error: the picker falls back to the built-ins alone.
        #[test]
        fn missing_themes_directory_yields_builtins_only() {
            let dir = tempfile::tempdir().unwrap();
            let found = sh_core::theme::load_discovered(&dir.path().join("nope"));
            assert!(found.is_empty());
            let entries = merge_theme_entries(&dir.path().join("nope"), &found);
            assert_eq!(entries.len(), crate::theme_builtins::BUILTIN_THEMES.len());
        }

        /// Every row that is selectable must carry the real file text, since
        /// that string seeds the hot-reload dedupe baseline. A row whose
        /// `text` disagreed with disk would make the watcher re-apply once
        /// on the next tick.
        #[test]
        fn every_selectable_row_carries_its_own_file_text() {
            let dir = tempfile::tempdir().unwrap();
            let mine = theme_json("Mine");
            write(dir.path(), "mine.json", &mine);
            let found = sh_core::theme::load_discovered(dir.path());

            let entries = merge_theme_entries(dir.path(), &found);

            let user = entries.iter().find(|e| e.file_name == "mine.json").unwrap();
            assert_eq!(user.text, mine);
            assert_eq!(sh_core::theme::parse(&user.text).unwrap().name, "Mine");
        }
    }
}
