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
#[derive(Clone)]
struct RuleFile {
    gitignore: Gitignore,
    lines: Vec<(String, u64)>,
    /// The file as read, so lines can be added to it in memory.
    contents: String,
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

/// Load a rules file. Returns `None` if the file does not exist.
fn load_file(root: &Path, path: &Path) -> Res<Option<RuleFile>> {
    if !path.is_file() {
        return Ok(None);
    }
    // `trace`: loaded once per status computation, which runs on a timer across
    // every package. It was 44% of an otherwise-quiet log. That the ignore file was
    // read is not news; what it *excluded* is, and the status line carries that count.
    trace!("Loading {}", path.display());
    let contents = std::fs::read_to_string(path).map_err(|err| read_error(path, err))?;
    parse_file(root, path, contents).map(Some)
}

/// Build a rules file from its text, as `GitignoreBuilder::add` would, keeping line numbers.
fn parse_file(root: &Path, path: &Path, contents: String) -> Res<RuleFile> {
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
    Ok(RuleFile {
        gitignore,
        lines,
        contents,
    })
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
    package_dir: PathBuf,
    package: Option<RuleFile>,
    user: Option<(PathBuf, RuleFile)>,
    user_file: PathBuf,
}

impl Rules {
    /// Load the package's `.quiltignore` and the user-wide `user_file`.
    /// Either may be missing; an unreadable or invalid one is an error.
    pub fn load(package_dir: &Path, user_file: &Path) -> Res<Rules> {
        let package = load_file(package_dir, &package_dir.join(QUILTIGNORE))?;
        let user = load_file(package_dir, user_file)?.map(|file| (user_file.to_path_buf(), file));
        Ok(Rules {
            package_dir: package_dir.to_path_buf(),
            package,
            user,
            user_file: user_file.to_path_buf(),
        })
    }

    /// The file a [`Scope`]'s lines go to.
    #[must_use]
    pub fn file(&self, scope: Scope) -> PathBuf {
        match scope {
            Scope::Package => self.package_dir.join(QUILTIGNORE),
            Scope::AllPackages => self.user_file.clone(),
        }
    }

    /// These rules with `lines` added to the end of `scope`'s file, in memory.
    pub fn with_lines(&self, scope: Scope, lines: &[String]) -> Res<Rules> {
        let path = self.file(scope);
        let current = match scope {
            Scope::Package => self.package.as_ref(),
            Scope::AllPackages => self.user.as_ref().map(|(_, file)| file),
        };
        let contents = appended(current.map_or("", |file| file.contents.as_str()), lines);
        let file = parse_file(&self.package_dir, &path, contents)?;
        let (package, user) = match scope {
            Scope::Package => (Some(file), self.user.clone()),
            Scope::AllPackages => (self.package.clone(), Some((path, file))),
        };
        Ok(Rules {
            package_dir: self.package_dir.clone(),
            package,
            user,
            user_file: self.user_file.clone(),
        })
    }

    /// The `!` lines that bring back the file or folder at `relative_path` in
    /// `scope`, outermost first: one for each folder above it the walk would
    /// still skip, then one for the path itself. Empty when nothing ignores it.
    /// `None` when `scope`'s file cannot bring it back: a package rule hides
    /// it, and the user-wide file is weaker.
    /// `tracked` says whether the package has the file, or a file in the folder.
    pub fn stop_ignoring_lines(
        &self,
        relative_path: &Path,
        is_dir: bool,
        tracked: impl Fn(&Path) -> bool,
        scope: Scope,
    ) -> Res<Option<Vec<String>>> {
        let mut lines = Vec::new();
        let mut rules = None;
        let mut prefix = PathBuf::new();
        let mut components = relative_path.components().peekable();
        while let Some(component) = components.next() {
            prefix.push(component);
            let dir = is_dir || components.peek().is_some();
            let current = rules.as_ref().unwrap_or(self);
            if current.ignored_by(&prefix, dir, tracked(&prefix)).is_none() {
                continue;
            }
            lines.push(stop_line(&prefix, dir, scope)?);
            let next = self.with_lines(scope, &lines)?;
            if next.ignored_by(&prefix, dir, tracked(&prefix)).is_some() {
                return Ok(None);
            }
            rules = Some(next);
        }
        Ok(Some(lines))
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

/// Where *Stop ignoring* writes its `!` lines.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    /// The package's `.quiltignore`, anchored to the path: `!/.snakemake/`.
    /// It travels with the package.
    Package,
    /// The user-wide file, by name only: `!.snakemake/`. Personal.
    AllPackages,
}

/// The `!` line for one path: anchored for the package, by name for the user.
fn stop_line(relative_path: &Path, is_dir: bool, scope: Scope) -> Res<String> {
    let mut names = Vec::new();
    let unnamable = |why: &str| {
        Error::Fs(FsError::Write {
            path: relative_path.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!("the name {why}, so no rule can name it"),
            ),
        })
    };
    for component in relative_path.components() {
        let name = component
            .as_os_str()
            .to_str()
            .ok_or_else(|| unnamable("is not UTF-8"))?;
        // A rule is one line.
        if name.contains('\n') {
            return Err(unnamable("holds a line break"));
        }
        names.push(escape(name));
    }
    let mut line = match scope {
        Scope::Package => format!("!/{}", names.join("/")),
        Scope::AllPackages => format!("!{}", names.last().map_or("", String::as_str)),
    };
    if is_dir {
        line.push('/');
    }
    Ok(line)
}

/// A name as a gitignore glob that matches only itself.
fn escape(name: &str) -> String {
    let mut glob = String::with_capacity(name.len());
    let last = name.chars().count().saturating_sub(1);
    for (i, c) in name.chars().enumerate() {
        // The parser trims trailing whitespace and a trailing `\` before a
        // folder's `/`, so those go in a class, which it leaves alone.
        if c == '\\' || (i == last && c.is_whitespace()) {
            glob.push('[');
            glob.push(c);
            glob.push(']');
            continue;
        }
        if matches!(c, '*' | '?' | '[' | ']' | '{' | '}') {
            glob.push('\\');
        }
        glob.push(c);
    }
    glob
}

/// `contents` with `lines` after it, each on a line of its own.
fn appended(contents: &str, lines: &[String]) -> String {
    let mut text = contents.to_string();
    if !text.is_empty() && !text.ends_with('\n') {
        text.push('\n');
    }
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    text
}

/// Add `lines` to the end of the rules file at `path`, creating it and its
/// folder if missing.
pub fn append_lines(path: &Path, lines: &[String]) -> Res {
    use std::io::Write;

    let write_error = |source| {
        Error::Fs(FsError::Write {
            path: path.to_path_buf(),
            source,
        })
    };
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|source| {
            Error::Fs(FsError::DirectoryCreate {
                path: dir.to_path_buf(),
                source,
            })
        })?;
    }
    let needs_newline = std::fs::read(path).is_ok_and(|s| !s.is_empty() && !s.ends_with(b"\n"));
    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(write_error)?;
    let mut text = appended("", lines);
    if needs_newline {
        text.insert(0, '\n');
    }
    file.write_all(text.as_bytes()).map_err(write_error)
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

    /// `*.key` is left out: it is also how Keynote saves a presentation.
    #[test]
    fn defaults_keep_keynote_decks() {
        let (_dir, rules) = rules(None, None);
        for name in ["deck.key", "talks/Deck.KEY"] {
            assert!(rules.file_ignored_by(Path::new(name), false).is_none());
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

    fn lines(rules: &Rules, path: &str, is_dir: bool, scope: Scope) -> Option<Vec<String>> {
        lines_tracking(rules, path, is_dir, scope, &[])
    }

    fn lines_tracking(
        rules: &Rules,
        path: &str,
        is_dir: bool,
        scope: Scope,
        tracked: &[&str],
    ) -> Option<Vec<String>> {
        rules
            .stop_ignoring_lines(
                Path::new(path),
                is_dir,
                |p| tracked.iter().any(|t| Path::new(t) == p),
                scope,
            )
            .unwrap()
    }

    #[test]
    fn stop_ignoring_a_default_folder() {
        let (_dir, rules) = rules(None, None);
        assert_eq!(
            lines(&rules, ".snakemake", true, Scope::Package),
            Some(vec!["!/.snakemake/".to_string()])
        );
        assert_eq!(
            lines(&rules, "runs/.snakemake", true, Scope::Package),
            Some(vec!["!/runs/.snakemake/".to_string()])
        );
        assert_eq!(
            lines(&rules, "runs/.snakemake", true, Scope::AllPackages),
            Some(vec!["!.snakemake/".to_string()])
        );
        for scope in [Scope::Package, Scope::AllPackages] {
            let after = rules
                .with_lines(scope, &lines(&rules, ".snakemake", true, scope).unwrap())
                .unwrap();
            assert!(
                after
                    .dir_ignored_by(Path::new(".snakemake"), false)
                    .is_none()
            );
            assert!(
                after
                    .file_ignored_by(Path::new(".snakemake/log/a.txt"), false)
                    .is_none()
            );
            assert!(after.dir_ignored_by(Path::new(".git"), false).is_some());
        }
    }

    #[test]
    fn stop_ignoring_a_path_nothing_ignores_writes_nothing() {
        let (_dir, rules) = rules(None, None);
        assert_eq!(
            lines(&rules, "data.csv", false, Scope::Package),
            Some(vec![])
        );
    }

    /// The walk enters a folder that holds a tracked file, so its new file
    /// needs only its own line.
    #[test]
    fn stop_ignoring_a_file_in_a_tracked_folder() {
        let (_dir, rules) = rules(None, None);
        assert_eq!(
            lines_tracking(&rules, ".git/HEAD", false, Scope::Package, &[".git"]),
            Some(vec!["!/.git/HEAD".to_string()])
        );
    }

    #[test]
    fn stop_ignoring_under_an_ignored_parent_writes_a_line_for_it() {
        let (_dir, rules) = rules(Some("cache/\n*.log\n"), Some("*.tmp\n"));
        assert_eq!(
            lines(&rules, "cache/a.log", false, Scope::Package),
            Some(vec!["!/cache/".to_string(), "!/cache/a.log".to_string()])
        );
        // A folder brought back brings back what is in it, unless a rule
        // in the same file names the file itself.
        assert_eq!(
            lines(&rules, "cache/a.tmp", false, Scope::Package),
            Some(vec!["!/cache/".to_string()])
        );
        let (_dir, user) = super::tests::rules(None, Some("cache/\n*.log\n"));
        assert_eq!(
            lines(&user, "cache/a.log", false, Scope::AllPackages),
            Some(vec!["!cache/".to_string(), "!a.log".to_string()])
        );
    }

    /// The package's own rules are stronger than the user-wide file.
    #[test]
    fn all_packages_cannot_undo_a_package_rule() {
        let (_dir, rules) = rules(Some("*.csv\nraw/\n"), None);
        assert_eq!(lines(&rules, "a.csv", false, Scope::AllPackages), None);
        assert_eq!(lines(&rules, "raw/b.txt", false, Scope::AllPackages), None);
        assert_eq!(
            lines(&rules, "a.csv", false, Scope::Package),
            Some(vec!["!/a.csv".to_string()])
        );
    }

    #[test]
    fn stop_ignoring_escapes_glob_characters() {
        let (_dir, rules) = rules(Some("*.txt\n"), None);
        for name in [
            "a*b[1].txt",
            "x?{y,z}.txt",
            "back\\slash.txt",
            "trailing .txt ",
            "tab.txt\t",
        ] {
            let found = lines(&rules, name, false, Scope::Package).unwrap();
            let after = rules.with_lines(Scope::Package, &found).unwrap();
            assert!(
                after.file_ignored_by(Path::new(name), false).is_none(),
                "{name:?} as {found:?}"
            );
        }
        let found = lines(&rules, "a*b[1].txt", false, Scope::Package).unwrap();
        assert_eq!(found, vec!["!/a\\*b\\[1\\].txt".to_string()]);
        let after = rules.with_lines(Scope::Package, &found).unwrap();
        assert!(
            after
                .file_ignored_by(Path::new("aZb1.txt"), false)
                .is_some()
        );
    }

    #[test]
    fn stop_ignoring_keeps_trailing_whitespace_and_backslashes() {
        let (_dir, rules) = rules(Some(".cache*/\n"), None);
        for scope in [Scope::Package, Scope::AllPackages] {
            for (name, is_dir) in [
                ("Icon\r", false),
                (".cache\\", true),
                ("sub/.cache\\", true),
            ] {
                let found = lines(&rules, name, is_dir, scope);
                if scope == Scope::AllPackages && is_dir {
                    // A package rule hides it.
                    assert_eq!(found, None, "{name:?}");
                    continue;
                }
                let found = found.unwrap_or_else(|| panic!("{name:?} in {scope:?}"));
                let after = rules.with_lines(scope, &found).unwrap();
                assert!(
                    after.ignored_by(Path::new(name), is_dir, false).is_none(),
                    "{name:?} as {found:?}"
                );
            }
        }
    }

    #[test]
    fn a_name_with_a_line_break_is_an_error() {
        let (_dir, rules) = rules(Some("*.log\n"), None);
        assert!(
            rules
                .stop_ignoring_lines(Path::new("a\nb.log"), false, |_| false, Scope::Package)
                .is_err()
        );
    }

    #[test]
    fn append_lines_starts_each_on_a_line_of_its_own() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".quiltignore");
        append_lines(&path, &["!/a/".to_string()]).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "!/a/\n");
        std::fs::write(&path, "*.log").unwrap();
        append_lines(&path, &["!/a/".to_string(), "!/a/b".to_string()]).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "*.log\n!/a/\n!/a/b\n"
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
