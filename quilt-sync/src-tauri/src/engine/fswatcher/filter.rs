use std::path::Path;

use crate::engine::quilt;

/// Returns true if a path should be ignored even when `notify` reports an
/// event for it: the app's own bookkeeping under `.quilt`. OS and editor
/// clutter is left to the ignore rules, which spare files the package has.
pub fn is_ignored(path: &Path) -> bool {
    path.components()
        .any(|c| c.as_os_str() == quilt::paths::DOT_QUILT_DIR)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    #[test]
    fn ignores_dot_quilt_subtree() {
        assert!(is_ignored(&PathBuf::from("/home/u/pkg/.quilt/data.json")));
        assert!(is_ignored(&PathBuf::from(".quilt/foo")));
    }

    #[test]
    fn passes_ordinary_files() {
        assert!(!is_ignored(&PathBuf::from("a/b/c.txt")));
        assert!(!is_ignored(&PathBuf::from("README.md")));
        assert!(!is_ignored(&PathBuf::from("/x/y/.DS_Store")));
    }
}
