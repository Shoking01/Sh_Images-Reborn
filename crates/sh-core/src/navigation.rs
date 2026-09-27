//! Folder scanning, natural sorting, and circular navigation.

use crate::errors::{Result, ShImagesError};
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// A list of image paths with a current position.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageList {
    /// All image paths in the scanned directory, naturally sorted.
    pub paths: Vec<PathBuf>,
    /// Index of the current image in `paths`.
    pub current: usize,
}

/// List supported image paths directly inside `dir`, naturally sorted.
/// Returns empty vec when `dir` is unreadable (caller decides the error UX).
/// V3: delegates to [`scan_entries`] so scan/sort has one code path.
pub fn scan_dir(dir: &Path, opts: ScanOptions) -> Vec<PathBuf> {
    scan_entries(dir, opts)
        .into_iter()
        .map(|e| e.path)
        .collect()
}

/// First supported image in `dir`, if any.
pub fn first_supported(dir: &Path, opts: ScanOptions) -> Option<PathBuf> {
    scan_dir(dir, opts).into_iter().next()
}

/// Resolve a path into an ordered image list, placing `path` at `current`.
///
/// Scans the parent directory, filters to supported image extensions,
/// natural-sorts the result, and locates `path` inside it.
///
/// Deliberately NOT parameterized by [`ScanOptions`]: this is the entry
/// point for an EXPLICITLY named file (the CLI argument), and a
/// `show_hidden: false` scan would not contain a dotfile at all — so
/// `resolve("~/.config/holiday.png")` would answer `NotAFile` and the
/// caller (main.rs) would drop the argument the user typed and land on
/// Welcome instead. A file named out loud is not hidden clutter in that
/// moment. The toggle therefore keeps ONE meaning — what a folder LISTING
/// shows — and main.rs pairs this with a `show_hidden: true` re-scan of the
/// same parent so the sibling list and the anchor cannot disagree.
pub fn resolve(path: &Path) -> Result<ImageList> {
    let parent = path
        .parent()
        .ok_or_else(|| ShImagesError::NotAFile(path.display().to_string()))?;
    let paths = scan_dir(parent, ScanOptions { show_hidden: true });
    let current = paths
        .iter()
        .position(|p| p == path)
        .ok_or_else(|| ShImagesError::NotAFile(path.display().to_string()))?;
    Ok(ImageList { paths, current })
}

/// Sortable image metadata + path: the unit of the V3 sort engine.
#[derive(Debug, Clone, PartialEq)]
pub struct ImageEntry {
    /// Absolute filesystem path to the image.
    pub path: PathBuf,
    /// File size in bytes.
    pub size: u64,
    /// Modified time.
    pub modified: SystemTime,
    /// Creation time where the OS reports it (`None` on Linux).
    pub created: Option<SystemTime>,
}

/// Gallery sort criterion.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SortBy {
    /// Natural file-name order (the historical default).
    #[default]
    #[serde(rename = "name")]
    Name,
    /// OS creation time (falls back to modified where unreported).
    #[serde(rename = "created")]
    Created,
    /// Last modification time.
    #[serde(rename = "modified")]
    Modified,
    /// File size in bytes.
    #[serde(rename = "size")]
    Size,
    /// Extension, case-insensitive.
    #[serde(rename = "type")]
    Type,
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum SortDir {
    /// Smallest / oldest / A→Z first.
    #[default]
    #[serde(rename = "asc")]
    Asc,
    /// Largest / newest / Z→A first.
    #[serde(rename = "desc")]
    Desc,
}

/// Borrowed, `Copy` view of the sortable metadata — lets callers sort their
/// own item types (e.g. sh-app's `ImageItem`) through the same comparator
/// without cloning paths into an [`ImageEntry`].
#[derive(Debug, Clone, Copy)]
pub struct MetaView<'a> {
    /// Borrowed path (used for the name/type keys).
    pub path: &'a Path,
    /// File size in bytes.
    pub size: u64,
    /// Modified time.
    pub modified: SystemTime,
    /// Creation time where the OS reports it (`None` on Linux).
    pub created: Option<SystemTime>,
}

impl<'a> From<&'a ImageEntry> for MetaView<'a> {
    fn from(e: &'a ImageEntry) -> Self {
        MetaView {
            path: &e.path,
            size: e.size,
            modified: e.modified,
            created: e.created,
        }
    }
}

/// Case-insensitive extension key for a bare path (empty when absent).
fn type_key_of(path: &Path) -> String {
    path.extension()
        .and_then(|x| x.to_str())
        .unwrap_or("")
        .to_ascii_lowercase()
}

/// Compare two metadata views under criterion + direction. Ties break by
/// natural name order (always ascending), so output is deterministic for
/// every criterion. This is the single comparator both [`sort_entries`]
/// (core) and `Session::resort` (sh-app) use — one sort engine, two callers.
pub fn compare_meta(a: &MetaView, b: &MetaView, by: SortBy, dir: SortDir) -> std::cmp::Ordering {
    let primary = match by {
        SortBy::Name => std::cmp::Ordering::Equal,
        SortBy::Created => a
            .created
            .unwrap_or(a.modified)
            .cmp(&b.created.unwrap_or(b.modified)),
        SortBy::Modified => a.modified.cmp(&b.modified),
        SortBy::Size => a.size.cmp(&b.size),
        SortBy::Type => type_key_of(a.path).cmp(&type_key_of(b.path)),
    };
    let directed = match dir {
        SortDir::Asc => primary,
        SortDir::Desc => primary.reverse(),
    };
    // Direction applies ONLY to the primary; the name tiebreak always stays
    // asc — groups stay internally stable regardless of input order.
    directed.then_with(|| natord::compare(&path_key(a.path), &path_key(b.path)))
}

/// Sort entries in place by criterion + direction. Pure; no I/O.
pub fn sort_entries(entries: &mut [ImageEntry], by: SortBy, dir: SortDir) {
    entries.sort_by(|a, b| compare_meta(&MetaView::from(a), &MetaView::from(b), by, dir));
}

/// What a directory scan may include beyond the plain
/// `is_file() && is_supported` rule.
///
/// A struct, not a bare `show_hidden: bool` parameter: the flag's default is
/// `false`, which makes a positional bool at the call site a coin flip nobody
/// can read — `scan_entries(dir, true)` does not say WHAT `true` buys, and the
/// failure mode (silently showing a user's dotfiles) is the kind that is only
/// noticed months later. A named field states the intent, and the next filter
/// to be needed (an extension allowlist, a size cap) lands here instead of
/// breaking every signature a second time.
///
/// `Copy` because callers capture it by value into background scan tasks
/// (sh-app's `spawn_list_load`), where a `Clone` would be noise per entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct ScanOptions {
    /// Include hidden entries: dot-prefixed names, plus — on Windows — the
    /// `FILE_ATTRIBUTE_HIDDEN` bit (see [`is_hidden`]).
    ///
    /// `Default` is `false` to match `Settings::show_hidden_files`, so a scan
    /// that forgets to pass options behaves like a user who never touched
    /// the toggle rather than like one who asked to see everything.
    pub show_hidden: bool,
}

/// Scan `dir` into sortable entries (path + metadata), name-asc ordered.
/// Unreadable dir → empty vec (caller decides the error UX); per-file
/// metadata failure → entry skipped (same tolerance as `filter_map(entry.ok())`).
///
/// `opts.show_hidden` is the ONLY thing [`ScanOptions`] changes here: the
/// `is_file() && is_supported` pre-filter runs first and is untouched, so a
/// directory named `.hidden_dir` and a dot-prefixed `notes.txt` stay out for
/// exactly the reasons they always did. The hidden check comes after the
/// metadata read because the Windows half of the predicate needs those
/// attributes — and it is the cheaper order anyway, since `is_supported`
/// rejects most entries before any `stat` is paid twice.
pub fn scan_entries(dir: &Path, opts: ScanOptions) -> Vec<ImageEntry> {
    let mut entries: Vec<ImageEntry> = std::fs::read_dir(dir)
        .map(|read| {
            read.filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if !path.is_file() || !is_supported(&path) {
                    return None;
                }
                let md = entry.metadata().ok()?;
                if !opts.show_hidden && is_hidden(&path, &md) {
                    return None;
                }
                Some(ImageEntry {
                    created: md.created().ok(),
                    modified: md.modified().ok()?,
                    size: md.len(),
                    path,
                })
            })
            .collect()
        })
        .unwrap_or_default();
    sort_entries(&mut entries, SortBy::Name, SortDir::Asc);
    entries
}

/// Is this one entry one the user thinks of as hidden?
///
/// PER-ENTRY by design, never per-path: only the entry's own name and its
/// own attributes are consulted, and no ancestor is ever stat-ed. So
/// `.git/config.png` still SHOWS with `show_hidden: false` — the dot that
/// would hide it belongs to the DIRECTORY, and a folder listing does not
/// list directories (the `is_file()` pre-filter drops `.git` itself, and the
/// listing is not recursive, so an ancestor could only ever REMOVE files the
/// user is looking straight at). Making
/// visibility depend on WHERE the folder sits would mean the same file is
/// visible at `C:\pics\.x\a.png` and invisible after being moved up one
/// level — a rule no user can predict, and one that would cost a `stat` per
/// ancestor per entry to evaluate. The dot prefix is checked first because
/// it is the universal convention: it is the only hidden marker that
/// survives a FAT32 stick, a zip extract and an ext4 mount identically.
fn is_hidden(path: &Path, md: &std::fs::Metadata) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.starts_with('.'))
        || is_hidden_by_attribute(md)
}

/// Windows half of [`is_hidden`]: the `FILE_ATTRIBUTE_HIDDEN` bit, which is
/// what Explorer greys out and what a `.bat` file copied off a Windows box
/// can carry even when its name says nothing.
///
/// `cfg`-SPLIT rather than cfg'd to a constant `false`: off Windows the
/// attribute does not exist, and naming it at all would import a
/// Windows-only assumption into a crate that is otherwise pure and
/// cross-platform-clean (the app binary is Windows-only in practice, this
/// library is not). The split also keeps the non-Windows build free of a
/// parameter it would have to ignore.
#[cfg(windows)]
fn is_hidden_by_attribute(md: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    /// `FILE_ATTRIBUTE_HIDDEN` from the Win32 `FILE_ATTRIBUTE_*` set.
    const FILE_ATTRIBUTE_HIDDEN: u32 = 0x2;
    md.file_attributes() & FILE_ATTRIBUTE_HIDDEN != 0
}

/// Non-Windows counterpart of [`is_hidden_by_attribute`]: always false, so
/// the leading-dot rule above is the whole convention there. (macOS also
/// carries a `UF_HIDDEN` flag that `std` does not expose; the dot prefix is
/// the portable half of the rule and the one every Unix user expects.)
#[cfg(not(windows))]
fn is_hidden_by_attribute(_md: &std::fs::Metadata) -> bool {
    false
}

/// Returns `next` index with circular wrap-around.
pub fn next_index(list: &ImageList) -> usize {
    if list.paths.is_empty() {
        0
    } else {
        (list.current + 1) % list.paths.len()
    }
}

/// Returns `prev` index with circular wrap-around.
pub fn prev_index(list: &ImageList) -> usize {
    if list.paths.is_empty() {
        0
    } else {
        (list.current + list.paths.len() - 1) % list.paths.len()
    }
}

/// Returns true if `path` has a supported image extension.
pub fn is_supported(path: &Path) -> bool {
    path.extension()
        .and_then(|ext| ext.to_str())
        .map(|ext| {
            matches!(
                ext.to_ascii_lowercase().as_str(),
                "png" | "jpg" | "jpeg" | "gif" | "bmp" | "webp" | "tif" | "tiff"
            )
        })
        .unwrap_or(false)
}

/// Key used for natural sort (file name lowercase, or full path fallback).
fn path_key(path: &Path) -> String {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(|name| name.to_lowercase())
        .unwrap_or_else(|| path.display().to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::tempdir;

    fn touch(dir: &Path, name: &str) -> PathBuf {
        let p = dir.join(name);
        fs::write(&p, b"x").unwrap();
        p
    }

    #[test]
    fn resolve_scans_parent_and_orders_naturally() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "img10.png");
        touch(dir.path(), "img2.png");
        touch(dir.path(), "img1.png");
        touch(dir.path(), "notes.txt");

        let target = dir.path().join("img2.png");
        let list = resolve(&target).unwrap();

        assert_eq!(list.paths.len(), 3);
        assert_eq!(list.paths[0].file_name().unwrap(), "img1.png");
        assert_eq!(list.paths[1].file_name().unwrap(), "img2.png");
        assert_eq!(list.paths[2].file_name().unwrap(), "img10.png");
        assert_eq!(list.current, 1);
    }

    #[test]
    fn filters_non_image_files() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "a.png");
        touch(dir.path(), "b.txt");
        touch(dir.path(), "c.jpg");
        let list = resolve(&dir.path().join("a.png")).unwrap();
        assert_eq!(list.paths.len(), 2);
    }

    #[test]
    fn handles_unicode_filenames() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "foto áéí.png");
        touch(dir.path(), "foto ü.png");
        let list = resolve(&dir.path().join("foto áéí.png")).unwrap();
        assert_eq!(list.paths.len(), 2);
        assert_eq!(list.current, 0);
    }

    #[test]
    fn next_and_prev_wrap_circularly() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "a.png");
        touch(dir.path(), "b.png");
        touch(dir.path(), "c.png");

        let mut list = resolve(&dir.path().join("a.png")).unwrap();
        assert_eq!(next_index(&list), 1);
        list.current = 1;
        assert_eq!(next_index(&list), 2);
        list.current = 2;
        assert_eq!(next_index(&list), 0); // wraps

        list.current = 0;
        assert_eq!(prev_index(&list), 2); // wraps backward
        list.current = 2;
        assert_eq!(prev_index(&list), 1);
    }

    #[test]
    fn missing_path_errors() {
        let dir = tempdir().unwrap();
        let err = resolve(&dir.path().join("nope.png")).unwrap_err();
        assert!(matches!(err, ShImagesError::NotAFile(_)));
    }

    #[test]
    fn is_supported_case_insensitive() {
        assert!(is_supported(Path::new("x.PNG")));
        assert!(is_supported(Path::new("x.JpEg")));
        assert!(is_supported(Path::new("x.tiff")));
        assert!(!is_supported(Path::new("x.txt")));
        assert!(!is_supported(Path::new("x")));
    }

    #[test]
    fn empty_list_next_and_prev_return_zero() {
        let list = ImageList {
            paths: Vec::new(),
            current: 0,
        };
        assert_eq!(next_index(&list), 0);
        assert_eq!(prev_index(&list), 0);
    }

    #[test]
    fn sort_is_case_insensitive() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "img1.png");
        touch(dir.path(), "IMG2.png");
        touch(dir.path(), "img10.png");

        let list = resolve(&dir.path().join("IMG2.png")).unwrap();

        let names: Vec<&str> = list
            .paths
            .iter()
            .map(|p| p.file_name().unwrap().to_str().unwrap())
            .collect();
        // The lowercased sort key maps IMG2.png -> "img2", so it interleaves
        // naturally: img1 < img2 < img10.
        assert_eq!(names[0], "img1.png");
        assert_eq!(names[1], "IMG2.png");
        assert_eq!(names[2], "img10.png");
        assert_eq!(list.current, 1);
    }

    #[test]
    fn scan_dir_lists_supported_naturally_sorted() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "img10.png");
        touch(dir.path(), "img2.png");
        touch(dir.path(), "notes.txt");
        let paths = scan_dir(dir.path(), ScanOptions::default());
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].file_name().unwrap(), "img2.png");
        assert_eq!(paths[1].file_name().unwrap(), "img10.png");
    }

    #[test]
    fn first_supported_returns_first_or_none() {
        let dir = tempdir().unwrap();
        assert!(first_supported(dir.path(), ScanOptions::default()).is_none());
        touch(dir.path(), "b.jpg");
        touch(dir.path(), "a.png");
        assert_eq!(
            first_supported(dir.path(), ScanOptions::default())
                .unwrap()
                .file_name()
                .unwrap(),
            "a.png"
        );
    }

    // ── Hidden-file filter (Settings::show_hidden_files) ──

    fn scanned_names(dir: &Path, show_hidden: bool) -> Vec<String> {
        scan_entries(dir, ScanOptions { show_hidden })
            .into_iter()
            .map(|e| e.path.file_name().unwrap().to_str().unwrap().to_string())
            .collect()
    }

    #[test]
    fn hidden_dotfile_is_excluded_unless_show_hidden_is_set() {
        let dir = tempdir().unwrap();
        touch(dir.path(), "a.png");
        touch(dir.path(), ".thumb.png");

        // Off (the shipped default): the dot-prefixed image is not listed.
        assert_eq!(scanned_names(dir.path(), false), vec!["a.png"]);
        // On: it is, and the sort still puts the dot name first ('.' < 'a').
        assert_eq!(scanned_names(dir.path(), true), vec![".thumb.png", "a.png"]);
    }

    #[test]
    fn hidden_filter_does_not_weaken_the_file_and_extension_rules() {
        let dir = tempdir().unwrap();
        // A dot-prefixed NON-image: excluded by `is_supported`, never by the
        // hidden rule, so showing hidden files must not admit it.
        touch(dir.path(), ".notes.txt");
        // A dot-prefixed directory holding a real image: excluded by
        // `is_file()`, so `show_hidden` must not turn the listing recursive.
        let nested = dir.path().join(".hidden_dir");
        fs::create_dir(&nested).unwrap();
        fs::write(nested.join("inner.png"), b"x").unwrap();
        touch(dir.path(), "a.png");

        let both = scanned_names(dir.path(), true);
        assert_eq!(both, vec!["a.png"]);
        assert_eq!(scanned_names(dir.path(), false), vec!["a.png"]);
    }

    /// The per-entry half of the rule, tested directly because it is the only
    /// half that is platform-independent: a leading dot means hidden on every
    /// OS, whatever the attributes say. Runs identically on Windows and Unix.
    #[test]
    fn is_hidden_follows_the_leading_dot_on_every_platform() {
        let dir = tempdir().unwrap();
        let dot = touch(dir.path(), ".thumb.png");
        let plain = touch(dir.path(), "thumb.png");
        // A file whose NAME is a lone dot cannot be a regular file on any
        // supported platform, so the entry name is checked through a path
        // rather than through a created fixture.
        let bare = dir.path().join(".");
        let md = fs::metadata(&plain).unwrap();

        assert!(is_hidden(&dot, &md), "dot-prefixed name is hidden");
        assert!(!is_hidden(&plain, &md), "plain name is not hidden");
        assert!(
            is_hidden(&bare, &md),
            "the predicate is literally a leading-dot test, not a suffix list"
        );
    }

    /// The Windows half: `FILE_ATTRIBUTE_HIDDEN` on a file whose name says
    /// nothing. Set through the built-in `attrib` (no new dependency, and
    /// `std` has no API to set the attribute), and skipped — not silently
    /// passed — where that binary is unavailable or the filesystem refuses
    /// the flag.
    #[cfg(windows)]
    #[test]
    fn windows_hidden_attribute_excludes_a_plain_named_image() {
        let dir = tempdir().unwrap();
        let hidden = touch(dir.path(), "secret.png");
        let plain = touch(dir.path(), "public.png");
        let set = std::process::Command::new("attrib")
            .arg("+h")
            .arg(&hidden)
            .status()
            .is_ok_and(|s| s.success());
        if !set {
            eprintln!("attrib +h unavailable here; attribute half not exercised");
        } else {
            use std::os::windows::fs::MetadataExt;
            assert_eq!(
                fs::metadata(&hidden).unwrap().file_attributes() & 0x2,
                0x2,
                "precondition: the fixture really carries FILE_ATTRIBUTE_HIDDEN"
            );
            assert!(
                is_hidden(&hidden, &fs::metadata(&hidden).unwrap()),
                "a hidden-attributed file is hidden whatever its name"
            );
        }
        // Always meaningful, set or not: a file that never got the attribute
        // must not be filtered by the Windows half.
        assert!(!is_hidden(&plain, &fs::metadata(&plain).unwrap()));
        // And the scan honours whichever state the attribute ended up in.
        let listed = scanned_names(dir.path(), false);
        assert!(
            listed.contains(&"public.png".to_string()),
            "the public file is always listed"
        );
        assert_eq!(
            listed.contains(&"secret.png".to_string()),
            !set,
            "the hidden-attributed file is listed only when the flag is set"
        );
        assert!(scanned_names(dir.path(), true).contains(&"secret.png".to_string()));
    }

    #[cfg(unix)]
    #[test]
    fn non_utf8_filename_uses_display_fallback_in_sort() {
        use std::os::unix::ffi::OsStringExt;

        let dir = tempdir().unwrap();
        touch(dir.path(), "a.png");
        // Invalid UTF-8 byte in the name; the extension itself stays valid UTF-8,
        // so the file still passes `is_supported` and only `path_key` falls back.
        let weird = std::ffi::OsString::from_vec(b"foto\xFF.png".to_vec());
        let weird_path = dir.path().join(&weird);
        fs::write(&weird_path, b"x").unwrap();

        let list = resolve(&weird_path).unwrap();
        assert_eq!(list.paths.len(), 2);
        // "a.png" sorts first; the lossy-displayed fallback key lands last.
        assert_eq!(list.current, 1);
        assert_eq!(list.paths[1].file_name().unwrap(), weird.as_os_str());
    }

    // ── V3 sort engine: pure comparator tests (no I/O) ──

    use std::time::{Duration, SystemTime};

    fn entry(
        path: &str,
        size: u64,
        modified: SystemTime,
        created: Option<SystemTime>,
    ) -> ImageEntry {
        ImageEntry {
            path: PathBuf::from(path),
            size,
            modified,
            created,
        }
    }

    const EPOCH: SystemTime = SystemTime::UNIX_EPOCH;

    #[test]
    fn sort_entries_by_size_desc_reorders_and_tiebreaks_by_name() {
        let mut entries = vec![
            entry("b.png", 300, EPOCH, Some(EPOCH)),
            entry("a.png", 300, EPOCH, Some(EPOCH)),
            entry("c.png", 100, EPOCH, Some(EPOCH)),
        ];
        sort_entries(&mut entries, SortBy::Size, SortDir::Desc);
        // 300-byte files first (name-asc tiebreak), then 100.
        assert_eq!(entries[0].path, PathBuf::from("a.png"));
        assert_eq!(entries[1].path, PathBuf::from("b.png"));
        assert_eq!(entries[2].path, PathBuf::from("c.png"));
    }

    #[test]
    fn sort_entries_by_created_falls_back_to_modified_when_none() {
        let t_early = EPOCH;
        let t_late = EPOCH + Duration::from_secs(10);
        let mut entries = vec![
            entry("late.png", 0, t_late, None),
            entry("early.png", 0, t_early, None),
        ];
        sort_entries(&mut entries, SortBy::Created, SortDir::Asc);
        assert_eq!(entries[0].path, PathBuf::from("early.png"));
        assert_eq!(entries[1].path, PathBuf::from("late.png"));
    }

    #[test]
    fn sort_entries_by_type_groups_extensions_case_insensitive() {
        let mut entries = vec![
            entry("z.PNG", 0, EPOCH, Some(EPOCH)),
            entry("a.jpg", 0, EPOCH, Some(EPOCH)),
            entry("m.png", 0, EPOCH, Some(EPOCH)),
        ];
        sort_entries(&mut entries, SortBy::Type, SortDir::Asc);
        // ext compare is case-insensitive: jpg < png; name tiebreak within png.
        assert_eq!(entries[0].path, PathBuf::from("a.jpg"));
        assert_eq!(entries[1].path, PathBuf::from("m.png"));
        assert_eq!(entries[2].path, PathBuf::from("z.PNG"));
    }

    #[test]
    fn sort_entries_by_modified_desc_inverts_order() {
        let t_early = EPOCH;
        let t_late = EPOCH + Duration::from_secs(10);
        let mut entries = vec![
            entry("early.png", 0, t_early, Some(t_early)),
            entry("late.png", 0, t_late, Some(t_late)),
        ];
        sort_entries(&mut entries, SortBy::Modified, SortDir::Desc);
        assert_eq!(entries[0].path, PathBuf::from("late.png"));
        assert_eq!(entries[1].path, PathBuf::from("early.png"));
    }

    #[test]
    fn sort_entries_name_natural_order_matches_scan_dir() {
        let mut entries = vec![
            entry("img10.png", 0, EPOCH, Some(EPOCH)),
            entry("img2.png", 0, EPOCH, Some(EPOCH)),
        ];
        sort_entries(&mut entries, SortBy::Name, SortDir::Asc);
        // Natural sort: img2 < img10 (today's scan_dir behavior).
        assert_eq!(entries[0].path, PathBuf::from("img2.png"));
        assert_eq!(entries[1].path, PathBuf::from("img10.png"));
    }
}
