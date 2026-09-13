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
pub fn scan_dir(dir: &Path) -> Vec<PathBuf> {
    scan_entries(dir).into_iter().map(|e| e.path).collect()
}

/// First supported image in `dir`, if any.
pub fn first_supported(dir: &Path) -> Option<PathBuf> {
    scan_dir(dir).into_iter().next()
}

/// Resolve a path into an ordered image list, placing `path` at `current`.
///
/// Scans the parent directory, filters to supported image extensions,
/// natural-sorts the result, and locates `path` inside it.
pub fn resolve(path: &Path) -> Result<ImageList> {
    let parent = path
        .parent()
        .ok_or_else(|| ShImagesError::NotAFile(path.display().to_string()))?;
    let paths = scan_dir(parent);
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortBy {
    /// Natural file-name order (the historical default).
    Name,
    /// OS creation time (falls back to modified where unreported).
    Created,
    /// Last modification time.
    Modified,
    /// File size in bytes.
    Size,
    /// Extension, case-insensitive.
    Type,
}

/// Sort direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDir {
    /// Smallest / oldest / A→Z first.
    Asc,
    /// Largest / newest / Z→A first.
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

/// Scan `dir` into sortable entries (path + metadata), name-asc ordered.
/// Unreadable dir → empty vec (caller decides the error UX); per-file
/// metadata failure → entry skipped (same tolerance as `filter_map(entry.ok())`).
pub fn scan_entries(dir: &Path) -> Vec<ImageEntry> {
    let mut entries: Vec<ImageEntry> = std::fs::read_dir(dir)
        .map(|read| {
            read.filter_map(|entry| {
                let entry = entry.ok()?;
                let path = entry.path();
                if !path.is_file() || !is_supported(&path) {
                    return None;
                }
                let md = entry.metadata().ok()?;
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
        let paths = scan_dir(dir.path());
        assert_eq!(paths.len(), 2);
        assert_eq!(paths[0].file_name().unwrap(), "img2.png");
        assert_eq!(paths[1].file_name().unwrap(), "img10.png");
    }

    #[test]
    fn first_supported_returns_first_or_none() {
        let dir = tempdir().unwrap();
        assert!(first_supported(dir.path()).is_none());
        touch(dir.path(), "b.jpg");
        touch(dir.path(), "a.png");
        assert_eq!(
            first_supported(dir.path()).unwrap().file_name().unwrap(),
            "a.png"
        );
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
