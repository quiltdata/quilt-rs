use std::path::Path;
use std::path::PathBuf;
use std::sync::OnceLock;

use ignore::Match;
use ignore::gitignore::Gitignore;
use ignore::gitignore::GitignoreBuilder;
use tracing::trace;

use crate::error::FsError;
use crate::{Error, Res};

const QUILTIGNORE: &str = ".quiltignore";

/// Built-in rules, the weakest layer: OS and editor clutter, secrets, caches.
/// One gitignore line each, matched case-insensitively.
pub const DEFAULT_RULES: &[&str] = &[
    ".*/",
    ".DS_Store",
    ".DS_Store?",
    "._*",
    ".LSOverride",
    ".VolumeIcon.icns",
    // The macOS `Icon\r` file. The parser trims a bare trailing `\r`, a class keeps it.
    "Icon[\r]",
    "Thumbs.db",
    "ehthumbs.db",
    "desktop.ini",
    "$RECYCLE.BIN/",
    ".directory",
    ".nfs*",
    "~$*",
    ".~lock.*#",
    ".#*",
    "\\#*\\#",
    "*.swp",
    "*.swo",
    "*~",
    ".env",
    ".env.*",
    ".netrc",
    "*.pem",
    "*.key",
    "*.code-workspace",
    "__pycache__/",
    "*.py[cod]",
    "node_modules/",
];

/// Where the rule that ignored a path came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RuleSource {
    /// [`DEFAULT_RULES`]
    Default,
    /// The user-wide rules file; `line` is 1-based.
    User { path: PathBuf, line: u64 },
    /// The package's own `.quiltignore`; `line` is 1-based.
    Package { line: u64 },
}

/// The rule that ignored a path: its line as written, and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IgnoredBy {
    pub pattern: String,
    pub source: RuleSource,
}

/// A loaded rules file, with the text and line number of each line that became a glob.
struct RuleFile {
    gitignore: Gitignore,
    lines: Vec<(String, u64)>,
}

impl RuleFile {
    /// The matcher reports the last matching glob, and an identical later line
    /// would match too, so the last line with the same text is the one that matched.
    fn line_of(&self, original: &str) -> u64 {
        self.lines
            .iter()
            .rev()
            .find(|(text, _)| text == original)
            .map_or(0, |(_, line)| *line)
    }
}

/// The text `GitignoreBuilder::add_line` keeps as a glob's original, if the line makes one.
fn glob_text(line: &str) -> Option<&str> {
    if line.starts_with('#') {
        return None;
    }
    let text = if line.ends_with("\\ ") {
        line
    } else {
        line.trim_end()
    };
    (!text.is_empty()).then_some(text)
}

fn read_error(path: &Path, err: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Error {
    Error::Fs(FsError::Read {
        path: path.to_path_buf(),
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, err),
    })
}

/// Load a rules file line by line, as `GitignoreBuilder::add` would, keeping line numbers.
/// Returns `None` if the file does not exist.
fn load_file(root: &Path, path: &Path) -> Res<Option<RuleFile>> {
    if !path.is_file() {
        return Ok(None);
    }
    // `trace`: loaded once per status computation, which runs on a timer across
    // every package. It was 44% of an otherwise-quiet log. That the ignore file was
    // read is not news; what it *excluded* is, and the status line carries that count.
    trace!("Loading {}", path.display());
    let contents = std::fs::read_to_string(path).map_err(|err| read_error(path, err))?;
    let mut builder = GitignoreBuilder::new(root);
    let mut lines = Vec::new();
    for (number, line) in (1u64..).zip(contents.lines()) {
        let line = if number == 1 {
            line.trim_start_matches('\u{feff}')
        } else {
            line
        };
        builder
            .add_line(Some(path.to_path_buf()), line)
            .map_err(|err| {
                read_error(
                    path,
                    ignore::Error::WithLineNumber {
                        line: number,
                        err: Box::new(err),
                    },
                )
            })?;
        if let Some(text) = glob_text(line) {
            lines.push((text.to_string(), number));
        }
    }
    let gitignore = builder.build().map_err(|err| read_error(path, err))?;
    Ok(Some(RuleFile { gitignore, lines }))
}

/// Try to load a `.quiltignore` file from the given directory.
/// Returns `None` if the file does not exist.
pub fn load(dir: &Path) -> Res<Option<Gitignore>> {
    Ok(load_file(dir, &dir.join(QUILTIGNORE))?.map(|file| file.gitignore))
}

fn defaults() -> &'static Gitignore {
    static INSTANCE: OnceLock<Gitignore> = OnceLock::new();
    INSTANCE.get_or_init(|| {
        let mut builder = GitignoreBuilder::new("/");
        builder
            .case_insensitive(true)
            .expect("case_insensitive never fails");
        for line in DEFAULT_RULES {
            builder
                .add_line(None, line)
                .expect("built-in ignore rule parses");
        }
        builder.build().expect("built-in ignore rules build")
    })
}

/// The ignore rules for one package, strongest first: the package's
/// `.quiltignore`, the user-wide file, then [`DEFAULT_RULES`].
///
/// The first layer with a matching line decides, and within a layer the last
/// matching line wins, so a `!pattern` brings a path back from weaker layers.
/// Only the package layer can hide a path the package already has; the other two
/// apply to new files only, so personal rules never change a shared package.
pub struct Rules {
    package: Option<RuleFile>,
    user: Option<(PathBuf, RuleFile)>,
}

impl Rules {
    /// Load the package's `.quiltignore` and the user-wide `user_file`.
    /// Either may be missing; an unreadable or invalid one is an error.
    pub fn load(package_dir: &Path, user_file: &Path) -> Res<Rules> {
        let package = load_file(package_dir, &package_dir.join(QUILTIGNORE))?;
        let user = load_file(package_dir, user_file)?.map(|file| (user_file.to_path_buf(), file));
        Ok(Rules { package, user })
    }

    /// The rule ignoring the directory at `relative_path`, if any.
    /// `tracked` is whether the directory holds a file the package has.
    #[must_use]
    pub fn dir_ignored_by(&self, relative_path: &Path, tracked: bool) -> Option<IgnoredBy> {
        self.ignored_by(relative_path, true, tracked)
    }

    /// The rule ignoring the file at `relative_path`, if any.
    /// `tracked` is whether the package already has the file.
    #[must_use]
    pub fn file_ignored_by(&self, relative_path: &Path, tracked: bool) -> Option<IgnoredBy> {
        self.ignored_by(relative_path, false, tracked)
    }

    fn ignored_by(&self, relative_path: &Path, is_dir: bool, tracked: bool) -> Option<IgnoredBy> {
        if let Some(file) = &self.package {
            match file
                .gitignore
                .matched_path_or_any_parents(relative_path, is_dir)
            {
                Match::Ignore(glob) => {
                    return Some(IgnoredBy {
                        pattern: glob.original().to_string(),
                        source: RuleSource::Package {
                            line: file.line_of(glob.original()),
                        },
                    });
                }
                Match::Whitelist(_) => return None,
                Match::None => {}
            }
        }
        if tracked || (!is_dir && relative_path == Path::new(QUILTIGNORE)) {
            return None;
        }
        if let Some((path, file)) = &self.user {
            match file
                .gitignore
                .matched_path_or_any_parents(relative_path, is_dir)
            {
                Match::Ignore(glob) => {
                    return Some(IgnoredBy {
                        pattern: glob.original().to_string(),
                        source: RuleSource::User {
                            path: path.clone(),
                            line: file.line_of(glob.original()),
                        },
                    });
                }
                Match::Whitelist(_) => return None,
                Match::None => {}
            }
        }
        match defaults().matched_path_or_any_parents(relative_path, is_dir) {
            Match::Ignore(glob) => Some(IgnoredBy {
                pattern: glob.original().to_string(),
                source: RuleSource::Default,
            }),
            Match::Whitelist(_) | Match::None => None,
        }
    }
}

/// Check whether the given path should be ignored.
/// `relative_path` is the logical key (relative to the package home).
/// `is_dir` must be `true` for directories (so `dir/` patterns work).
#[must_use]
pub fn is_ignored(gitignore: &Gitignore, relative_path: &Path, is_dir: bool) -> bool {
    gitignore
        .matched_path_or_any_parents(relative_path, is_dir)
        .is_ignore()
}

/// If the path is ignored, return the original pattern string that matched it.
#[must_use]
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

    /// Rules for a package dir holding `package` (if any) and a user file holding `user` (if any).
    fn rules(package: Option<&str>, user: Option<&str>) -> (TempDir, Rules) {
        let dir = TempDir::new().unwrap();
        let package_dir = dir.path().join("pkg");
        std::fs::create_dir(&package_dir).unwrap();
        if let Some(text) = package {
            std::fs::write(package_dir.join(".quiltignore"), text).unwrap();
        }
        let user_file = dir.path().join("user-quiltignore");
        if let Some(text) = user {
            std::fs::write(&user_file, text).unwrap();
        }
        let rules = Rules::load(&package_dir, &user_file).unwrap();
        (dir, rules)
    }

    fn source(by: Option<IgnoredBy>) -> Option<RuleSource> {
        by.map(|by| by.source)
    }

    #[test]
    fn every_default_rule_parses() {
        for line in DEFAULT_RULES {
            let mut builder = GitignoreBuilder::new("/");
            assert!(builder.add_line(None, line).is_ok(), "{line:?}");
            assert!(glob_text(line).is_some(), "{line:?} makes no glob");
        }
        assert_eq!(defaults().len(), DEFAULT_RULES.len());
    }

    #[test]
    fn defaults_ignore_clutter_and_secrets() {
        let (_dir, rules) = rules(None, None);
        for file in [
            ".DS_Store",
            "sub/.DS_Store",
            ".env",
            ".env.local",
            "Icon\r",
            "~$report.xlsx",
            "notes.txt~",
            "key.pem",
            "mod.pyc",
            "node_modules/a/index.js",
            ".git/config",
        ] {
            assert_eq!(
                source(rules.file_ignored_by(Path::new(file), false)),
                Some(RuleSource::Default),
                "{file}"
            );
        }
        for dir in [".git", ".venv", "node_modules", "a/__pycache__"] {
            assert_eq!(
                source(rules.dir_ignored_by(Path::new(dir), false)),
                Some(RuleSource::Default),
                "{dir}"
            );
        }
        assert!(rules.file_ignored_by(Path::new("Icon"), false).is_none());
        assert!(
            rules
                .file_ignored_by(Path::new("data.csv"), false)
                .is_none()
        );
    }

    #[test]
    fn defaults_keep_zarr_metadata() {
        let (_dir, rules) = rules(None, None);
        for name in [".zattrs", ".zarray", ".zgroup", ".zmetadata"] {
            assert!(rules.file_ignored_by(Path::new(name), false).is_none());
            let nested = Path::new("img.zarr/0").join(name);
            assert!(rules.file_ignored_by(&nested, false).is_none());
        }
    }

    #[test]
    fn defaults_are_case_insensitive_but_files_are_not() {
        let (_dir, rules) = rules(Some("*.csv\n"), Some("*.tmp\n"));
        assert!(
            rules
                .file_ignored_by(Path::new(".ds_store"), false)
                .is_some()
        );
        assert!(rules.file_ignored_by(Path::new("FOO.PEM"), false).is_some());
        assert!(rules.file_ignored_by(Path::new("a.csv"), false).is_some());
        assert!(rules.file_ignored_by(Path::new("A.CSV"), false).is_none());
        assert!(rules.file_ignored_by(Path::new("a.tmp"), false).is_some());
        assert!(rules.file_ignored_by(Path::new("A.TMP"), false).is_none());
    }

    #[test]
    fn user_whitelist_beats_default() {
        let (_dir, rules) = rules(None, Some("!.env\n"));
        assert!(rules.file_ignored_by(Path::new(".env"), false).is_none());
        assert!(
            rules
                .file_ignored_by(Path::new(".DS_Store"), false)
                .is_some()
        );
    }

    #[test]
    fn package_whitelist_beats_user_and_default() {
        let (_dir, rules) = rules(Some("!.env\n!a.tmp\n"), Some("*.tmp\n"));
        assert!(rules.file_ignored_by(Path::new(".env"), false).is_none());
        assert!(rules.file_ignored_by(Path::new("a.tmp"), false).is_none());
        assert!(rules.file_ignored_by(Path::new("b.tmp"), false).is_some());
    }

    #[test]
    fn package_ignore_beats_user_whitelist() {
        let (_dir, rules) = rules(Some("*.csv\n"), Some("!a.csv\n"));
        let by = rules.file_ignored_by(Path::new("a.csv"), false).unwrap();
        assert_eq!(by.pattern, "*.csv");
        assert_eq!(by.source, RuleSource::Package { line: 1 });
        // Tracked or not, a package rule ignores the path.
        assert!(rules.file_ignored_by(Path::new("a.csv"), true).is_some());
    }

    #[test]
    fn app_layers_skip_tracked_paths() {
        let (_dir, rules) = rules(None, Some("*.tmp\n"));
        assert!(rules.file_ignored_by(Path::new("a.tmp"), true).is_none());
        assert!(
            rules
                .file_ignored_by(Path::new(".DS_Store"), true)
                .is_none()
        );
        assert!(rules.dir_ignored_by(Path::new(".git"), true).is_none());
        assert!(rules.dir_ignored_by(Path::new(".git"), false).is_some());
    }

    #[test]
    fn user_rule_reports_file_and_line() {
        let (dir, rules) = rules(None, Some("# comment\n\n*.tmp\n*.bak\n*.tmp\n"));
        let by = rules.file_ignored_by(Path::new("a.tmp"), false).unwrap();
        assert_eq!(by.pattern, "*.tmp");
        assert_eq!(
            by.source,
            RuleSource::User {
                path: dir.path().join("user-quiltignore"),
                line: 5,
            }
        );
        let by = rules.file_ignored_by(Path::new("a.bak"), false).unwrap();
        assert_eq!(
            by.source,
            RuleSource::User {
                path: dir.path().join("user-quiltignore"),
                line: 4,
            }
        );
    }

    #[test]
    fn package_rule_reports_line_after_bom_and_trailing_space() {
        let (_dir, rules) = rules(Some("\u{feff}*.log\n*.csv   \n"), None);
        let by = rules.file_ignored_by(Path::new("a.log"), false).unwrap();
        assert_eq!(
            (by.pattern.as_str(), by.source),
            ("*.log", RuleSource::Package { line: 1 })
        );
        let by = rules.file_ignored_by(Path::new("a.csv"), false).unwrap();
        assert_eq!(
            (by.pattern.as_str(), by.source),
            ("*.csv", RuleSource::Package { line: 2 })
        );
    }

    #[test]
    fn app_layers_never_ignore_the_package_quiltignore() {
        let (_dir, user_dotfiles) = rules(None, Some(".*\n"));
        assert!(
            user_dotfiles
                .file_ignored_by(Path::new(".quiltignore"), false)
                .is_none()
        );
        assert!(
            user_dotfiles
                .file_ignored_by(Path::new(".hidden"), false)
                .is_some()
        );
        // Only the top-level one is the package's.
        assert!(
            user_dotfiles
                .file_ignored_by(Path::new("sub/.quiltignore"), false)
                .is_some()
        );
        // The package's own rules still may.
        let (_dir, rules) = rules(Some(".quiltignore\n"), None);
        assert!(
            rules
                .file_ignored_by(Path::new(".quiltignore"), false)
                .is_some()
        );
    }

    #[test]
    fn unreadable_or_invalid_user_file_is_an_error() {
        let dir = TempDir::new().unwrap();
        let user_file = dir.path().join("user-quiltignore");
        std::fs::write(&user_file, b"ok\n\xff\n").unwrap();
        assert!(Rules::load(dir.path(), &user_file).is_err());
        std::fs::write(&user_file, "ok\na{b\n").unwrap();
        assert!(Rules::load(dir.path(), &user_file).is_err());
    }
}
