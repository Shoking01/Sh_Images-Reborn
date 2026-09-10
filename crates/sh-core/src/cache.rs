//! LRU cache for decoded images with a byte budget.

use crate::decode::DecodedImage;
use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::Arc;

/// LRU cache of decoded images bounded by entry count and byte budget.
#[derive(Debug)]
pub struct DecodeCache {
    capacity: usize,
    byte_limit: usize,
    entries: VecDeque<(PathBuf, Arc<DecodedImage>)>,
    bytes: usize,
}

impl DecodeCache {
    /// Create a cache with at most `capacity` entries and `byte_limit` total decoded bytes.
    pub fn new(capacity: usize, byte_limit: usize) -> Self {
        Self {
            capacity,
            byte_limit,
            entries: VecDeque::new(),
            bytes: 0,
        }
    }

    /// Look up a path; on hit, move it to the most-recently-used position.
    pub fn get(&mut self, path: &PathBuf) -> Option<Arc<DecodedImage>> {
        let pos = self.entries.iter().position(|(p, _)| p == path)?;
        let entry = self.entries.remove(pos)?;
        self.entries.push_back(entry.clone());
        Some(entry.1)
    }

    /// Insert an image, evicting least-recently-used entries to stay within limits.
    pub fn insert(&mut self, path: PathBuf, image: Arc<DecodedImage>) {
        // If already present, remove first so byte accounting stays correct.
        if let Some(pos) = self.entries.iter().position(|(p, _)| p == &path) {
            if let Some((_, old)) = self.entries.remove(pos) {
                self.bytes = self.bytes.saturating_sub(old.rgba.len());
            }
        }
        self.bytes = self.bytes.saturating_add(image.rgba.len());
        self.entries.push_back((path, image));

        while self.entries.len() > self.capacity || self.bytes > self.byte_limit {
            let Some((_, removed)) = self.entries.pop_front() else {
                break;
            };
            self.bytes = self.bytes.saturating_sub(removed.rgba.len());
        }
    }

    /// Current decoded byte total.
    pub fn bytes(&self) -> usize {
        self.bytes
    }

    /// Current entry count.
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    /// Returns true when the cache is empty.
    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use proptest::prop_assert;

    fn img(bytes: usize) -> Arc<DecodedImage> {
        Arc::new(DecodedImage {
            width: 1,
            height: (bytes / 4) as u32,
            rgba: vec![0; bytes],
        })
    }

    #[test]
    fn evicts_least_recently_used() {
        let mut c = DecodeCache::new(2, 1_000_000);
        c.insert(PathBuf::from("a"), img(10));
        c.insert(PathBuf::from("b"), img(10));
        c.insert(PathBuf::from("c"), img(10));
        assert_eq!(c.len(), 2);
        assert!(c.get(&PathBuf::from("a")).is_none());
        assert!(c.get(&PathBuf::from("b")).is_some());
        assert!(c.get(&PathBuf::from("c")).is_some());
    }

    #[test]
    fn get_refreshes_position() {
        let mut c = DecodeCache::new(2, 1_000_000);
        c.insert(PathBuf::from("a"), img(10));
        c.insert(PathBuf::from("b"), img(10));
        let _ = c.get(&PathBuf::from("a")); // a becomes MRU
        c.insert(PathBuf::from("c"), img(10)); // evicts b
        assert!(c.get(&PathBuf::from("b")).is_none());
        assert!(c.get(&PathBuf::from("a")).is_some());
    }

    #[test]
    fn enforces_byte_budget() {
        let mut c = DecodeCache::new(10, 50);
        c.insert(PathBuf::from("a"), img(40));
        c.insert(PathBuf::from("b"), img(40));
        assert!(c.bytes() <= 50);
        assert!(c.get(&PathBuf::from("a")).is_none());
        assert!(c.get(&PathBuf::from("b")).is_some());
    }

    #[test]
    fn reinsert_same_path_does_not_double_count() {
        let mut c = DecodeCache::new(10, 1_000_000);
        c.insert(PathBuf::from("a"), img(40));
        c.insert(PathBuf::from("a"), img(40));
        assert_eq!(c.bytes(), 40);
        assert_eq!(c.len(), 1);
    }

    proptest::proptest! {
        #[test]
        fn never_exceeds_budget_after_any_inserts(ops: Vec<(u8, usize)>, budget in 10usize..200) {
            let mut c = DecodeCache::new(5, budget);
            for (key, size) in ops {
                let k = format!("k{key}");
                c.insert(PathBuf::from(k), img(size % 60 + 1));
            }
            prop_assert!(c.bytes() <= budget);
        }
    }
}
