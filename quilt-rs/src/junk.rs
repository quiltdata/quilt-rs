use std::collections::HashSet;
use std::path::Path;
use std::sync::OnceLock;

use gitignores::GitIgnore;
use ignore::gitignore::{Gitignore, GitignoreBuilder};

/// Result of checking a file path against known junk patterns.
pub struct Match {
    /// The suggested glob pattern (e.g. "*.pyc")
    pub pattern: String,
}

/// Patterns from Global gitignore templates that are too broad for data packages.
/// These would match legitimate data files if included.
const BLOCKLIST: &[&str] = &[
    "out/",
    "dist/",
    "*_archive",
    "*.log",
    "*.gz",
    "*.zip",
    "*.pdf",
    "*.tar",
    "*.bak",
    "*.orig",
    "*.rar",
    "*.db",
    "*.sql",
    "*.sqlite",
    "*.sqlite3",
    "tags",
    "TAGS",
    "*.idb",
    // Broad temp patterns that catch data
    "*.tmp",
    "*.temp",
];

/// Hardcoded junk patterns not covered by Global templates.
const HARDCODED: &[&str] = &[
    // Python bytecode
    "*.pyc",
    "*.pyo",
    "__pycache__/",
    // Jupyter autosave
    ".ipynb_checkpoints/",
    // JS dependencies
    "node_modules/",
    // Version control
    ".git/",
    ".gitignore",
    ".gitattributes",
    // OS metadata
    ".DS_Store",
    "Thumbs.db",
    "desktop.ini",
    // Office temp/lock files
    "~$*",
    "$~*",
    // Editor swap files
    "*.swp",
    "*.swo",
    "*~",
];

fn build_matcher() -> Gitignore {
    let mut builder = GitignoreBuilder::new("/");

    let blocklist_set: HashSet<&str> = BLOCKLIST.iter().copied().collect();

    // Load all Global templates from the gitignores crate
    for name in gitignores::Global::list() {
        let Some(template) = gitignores::Global::get(name) else {
            continue;
        };
        let content = template.contents();
        for line in content.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }
            if trimmed.starts_with('!') {
                continue;
            }
            if blocklist_set.contains(trimmed) {
                continue;
            }
            let _ = builder.add_line(None, trimmed);
        }
    }

    // Add hardcoded patterns
    for &pattern in HARDCODED {
        let _ = builder.add_line(None, pattern);
    }

    builder.build().expect("failed to build junk matcher")
}

fn global() -> &'static Gitignore {
    static INSTANCE: OnceLock<Gitignore> = OnceLock::new();
    INSTANCE.get_or_init(build_matcher)
}

/// Check if a file path looks like junk. Returns the suggested pattern if so.
///
/// `path` should be a relative path (logical key) within the package.
pub fn check(path: &Path) -> Option<Match> {
    let gitignore = global();

    let is_dir = path.to_string_lossy().ends_with('/');
    let m = gitignore.matched_path_or_any_parents(path, is_dir);

    if !m.is_ignore() {
        return None;
    }

    // Extract the original pattern string from the match
    let pattern = m
        .inner()
        .map(|glob| glob.original().to_string())
        .unwrap_or_else(|| suggest_pattern(path));

    Some(Match { pattern })
}

/// Test whether a gitignore-syntax pattern matches a given path.
pub fn pattern_matches(pattern: &str, path: &str) -> bool {
    let mut builder = GitignoreBuilder::new("/");
    if builder.add_line(None, pattern).is_err() {
        return false;
    }
    let Ok(gi) = builder.build() else {
        return false;
    };
    gi.matched_path_or_any_parents(Path::new(path), false)
        .is_ignore()
}

/// Suggest a glob pattern for a given file path.
/// Falls back to extension-based or exact-path patterns.
fn suggest_pattern(path: &Path) -> String {
    if let Some(ext) = path.extension() {
        format!("*.{}", ext.to_string_lossy())
    } else if let Some(name) = path.file_name() {
        name.to_string_lossy().into_owned()
    } else {
        path.to_string_lossy().into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn detects_python_bytecode() {
        let m = check(&PathBuf::from("data/clean.pyc")).unwrap();
        assert_eq!(m.pattern, "*.pyc");
    }

    #[test]
    fn detects_pycache_file() {
        // A file inside __pycache__ is detected
        let m = check(&PathBuf::from("__pycache__/module.cpython-311.pyc")).unwrap();
        assert!(m.pattern == "*.pyc" || m.pattern == "__pycache__/");
    }

    #[test]
    fn detects_ds_store() {
        let m = check(&PathBuf::from(".DS_Store")).unwrap();
        assert_eq!(m.pattern, ".DS_Store");
    }

    #[test]
    fn detects_node_modules_file() {
        // A file inside node_modules is detected
        let m = check(&PathBuf::from("node_modules/package/index.js")).unwrap();
        assert_eq!(m.pattern, "node_modules/");
    }

    #[test]
    fn detects_editor_swap_files() {
        let m = check(&PathBuf::from("data.csv.swp")).unwrap();
        assert_eq!(m.pattern, "*.swp");
    }

    #[test]
    fn detects_office_temp_files() {
        let m = check(&PathBuf::from("~$report.xlsx")).unwrap();
        assert_eq!(m.pattern, "~$*");
    }

    #[test]
    fn detects_git_file() {
        // A file inside .git is detected
        let m = check(&PathBuf::from(".git/config")).unwrap();
        assert_eq!(m.pattern, ".git/");
    }

    #[test]
    fn detects_gitignore_file() {
        let m = check(&PathBuf::from(".gitignore")).unwrap();
        assert_eq!(m.pattern, ".gitignore");
    }

    #[test]
    fn does_not_flag_parquet() {
        assert!(check(&PathBuf::from("data/results.parquet")).is_none());
    }

    #[test]
    fn does_not_flag_csv() {
        assert!(check(&PathBuf::from("data.csv")).is_none());
    }

    #[test]
    fn does_not_flag_json() {
        assert!(check(&PathBuf::from("config.json")).is_none());
    }

    #[test]
    fn does_not_flag_xlsx() {
        assert!(check(&PathBuf::from("report.xlsx")).is_none());
    }

    #[test]
    fn does_not_flag_txt() {
        assert!(check(&PathBuf::from("readme.txt")).is_none());
    }

    #[test]
    fn blocklisted_patterns_do_not_match() {
        // *.log is blocklisted — should NOT be flagged
        assert!(check(&PathBuf::from("server.log")).is_none());
        // *.zip is blocklisted
        assert!(check(&PathBuf::from("archive.zip")).is_none());
    }

    #[test]
    fn pattern_matches_works() {
        assert!(pattern_matches("*.pyc", "data/clean.pyc"));
        assert!(!pattern_matches("*.pyc", "data/clean.py"));
        assert!(pattern_matches("data/*.csv", "data/file.csv"));
        assert!(!pattern_matches("data/*.csv", "other/file.csv"));
    }
}
