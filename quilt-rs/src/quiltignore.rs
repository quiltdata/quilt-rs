use std::path::Path;

use ignore::gitignore::{Gitignore, GitignoreBuilder};
use tracing::debug;

use crate::error::FsError;
use crate::{Error, Res};

const QUILTIGNORE: &str = ".quiltignore";

/// Try to load a `.quiltignore` file from the given directory.
/// Returns `None` if the file does not exist.
pub fn load(dir: &Path) -> Res<Option<Gitignore>> {
    let path = dir.join(QUILTIGNORE);
    if !path.is_file() {
        return Ok(None);
    }
    debug!("Loading {}", path.display());
    let mut builder = GitignoreBuilder::new(dir);
    if let Some(err) = builder.add(&path) {
        return Err(Error::Fs(FsError::Read {
            path,
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, err),
        }));
    }
    let gitignore = builder.build().map_err(|err| {
        Error::Fs(FsError::Read {
            path,
            source: std::io::Error::new(std::io::ErrorKind::InvalidData, err),
        })
    })?;
    Ok(Some(gitignore))
}

/// Check whether the given path should be ignored.
/// `relative_path` is the logical key (relative to the package home).
/// `is_dir` must be `true` for directories (so `dir/` patterns work).
pub fn is_ignored(gitignore: &Gitignore, relative_path: &Path, is_dir: bool) -> bool {
    gitignore
        .matched_path_or_any_parents(relative_path, is_dir)
        .is_ignore()
}

/// If the path is ignored, return the original pattern string that matched it.
pub fn matched_pattern(
    gitignore: &Gitignore,
    relative_path: &Path,
    is_dir: bool,
) -> Option<String> {
    let m = gitignore.matched_path_or_any_parents(relative_path, is_dir);
    if m.is_ignore() {
        m.inner().map(|glob| glob.original().to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn load_returns_none_when_no_file() {
        let dir = TempDir::new().unwrap();
        let result = load(dir.path()).unwrap();
        assert!(result.is_none());
    }

    #[test]
    fn load_returns_some_when_file_exists() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".quiltignore"), "*.log\n").unwrap();
        let result = load(dir.path()).unwrap();
        assert!(result.is_some());
    }

    #[test]
    fn is_ignored_simple_glob() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".quiltignore"), "*.log\n").unwrap();
        let gi = load(dir.path()).unwrap().unwrap();
        assert!(is_ignored(&gi, Path::new("app.log"), false));
        assert!(!is_ignored(&gi, Path::new("app.txt"), false));
    }

    #[test]
    fn is_ignored_negation() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".quiltignore"), "*.log\n!important.log\n").unwrap();
        let gi = load(dir.path()).unwrap().unwrap();
        assert!(is_ignored(&gi, Path::new("debug.log"), false));
        assert!(!is_ignored(&gi, Path::new("important.log"), false));
    }

    #[test]
    fn is_ignored_directory_pattern() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".quiltignore"), "cache/\n").unwrap();
        let gi = load(dir.path()).unwrap().unwrap();
        assert!(is_ignored(&gi, Path::new("cache"), true));
        assert!(!is_ignored(&gi, Path::new("cache"), false));
    }

    #[test]
    fn is_ignored_globstar() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".quiltignore"), "**/temp/*.tmp\n").unwrap();
        let gi = load(dir.path()).unwrap().unwrap();
        assert!(is_ignored(&gi, Path::new("a/b/temp/foo.tmp"), false));
        assert!(!is_ignored(&gi, Path::new("a/b/temp/foo.txt"), false));
    }

    #[test]
    fn is_ignored_rooted_pattern() {
        let dir = TempDir::new().unwrap();
        std::fs::write(dir.path().join(".quiltignore"), "/build\n").unwrap();
        let gi = load(dir.path()).unwrap().unwrap();
        assert!(is_ignored(&gi, Path::new("build"), false));
        assert!(!is_ignored(&gi, Path::new("src/build"), false));
    }

    #[test]
    fn comments_and_blank_lines_are_ignored() {
        let dir = TempDir::new().unwrap();
        std::fs::write(
            dir.path().join(".quiltignore"),
            "# this is a comment\n\n*.tmp\n",
        )
        .unwrap();
        let gi = load(dir.path()).unwrap().unwrap();
        assert!(is_ignored(&gi, Path::new("foo.tmp"), false));
        assert!(!is_ignored(&gi, Path::new("# this is a comment"), false));
    }
}
