use std::path::Path;

/// Returns true if a path should be ignored even when `notify` reports an
/// event for it. Intentionally a small static deny-list — gitignore-style
/// user filters wait for evidence they are needed.
pub fn is_ignored(path: &Path) -> bool {
    if path.components().any(|c| c.as_os_str() == ".quilt") {
        return true;
    }
    let Some(name) = path.file_name().and_then(|s| s.to_str()) else {
        return false;
    };
    if matches!(name, ".DS_Store" | "Thumbs.db") || name.starts_with("~$") {
        return true;
    }
    matches!(
        path.extension()
            .and_then(|s| s.to_str())
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("swp" | "tmp"),
    )
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
    fn ignores_os_noise() {
        assert!(is_ignored(&PathBuf::from("/x/y/.DS_Store")));
        assert!(is_ignored(&PathBuf::from("/x/y/Thumbs.db")));
    }

    #[test]
    fn ignores_editor_swap_files() {
        assert!(is_ignored(&PathBuf::from("a/.foo.swp")));
        assert!(is_ignored(&PathBuf::from("a/foo.tmp")));
        assert!(is_ignored(&PathBuf::from("a/~$doc.docx")));
    }

    #[test]
    fn passes_ordinary_files() {
        assert!(!is_ignored(&PathBuf::from("a/b/c.txt")));
        assert!(!is_ignored(&PathBuf::from("README.md")));
    }
}
