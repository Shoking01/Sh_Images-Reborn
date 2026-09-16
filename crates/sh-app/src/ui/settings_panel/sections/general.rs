//! General section rows: hidden-files toggle, recent folders + clear.

/// Label for the recents header: `"Recent folders (N)"`, pure for tests.
pub fn recents_header(count: usize) -> String {
    format!("Recent folders ({count})")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recents_header_counts() {
        assert_eq!(recents_header(0), "Recent folders (0)");
        assert_eq!(recents_header(3), "Recent folders (3)");
    }
}
