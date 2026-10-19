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
/// With `switches`, a line that turns a built-in rule off is left out: see
/// [`switched_off`].
fn load_file(root: &Path, path: &Path, switches: bool) -> Res<Option<RuleFile>> {
    if !path.is_file() {
        return Ok(None);
    }
    // `trace`: loaded once per status computation, which runs on a timer across
    // every package. It was 44% of an otherwise-quiet log. That the ignore file was
    // read is not news; what it *excluded* is, and the status line carries that count.
    trace!("Loading {}", path.display());
    let contents = std::fs::read_to_string(path).map_err(|err| read_error(path, err))?;
    parse_file(root, path, contents, switches).map(Some)
}

/// Build a rules file from its text, as `GitignoreBuilder::add` would, keeping line numbers.
fn parse_file(root: &Path, path: &Path, contents: String, switches: bool) -> Res<RuleFile> {
    let mut builder = GitignoreBuilder::new(root);
    let mut lines = Vec::new();
    for (number, line) in (1u64..).zip(contents.lines()) {
        let line = if number == 1 {
            line.trim_start_matches('\u{feff}')
        } else {
            line
        };
        if switches && glob_text(line).is_some_and(|text| switch_of(text).is_some()) {
            continue;
        }
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
    Ok(load_file(dir, &dir.join(QUILTIGNORE), false)?.map(|file| file.gitignore))
}

/// The built-in rule a line of the user-wide file turns off, if it is one:
/// `!` and the rule exactly. Such a line takes the rule out of the built-in
/// layer, case-insensitive like the rule, rather than bringing back all it
/// matches: `!.*/` must not bring back a `.env` in a dot-folder.
fn switch_of(text: &str) -> Option<&'static str> {
    let rule = text.strip_prefix('!')?;
    DEFAULT_RULES.iter().copied().find(|r| *r == rule)
}

/// The built-in rules `contents` turns off.
fn switched_off(contents: &str) -> Vec<&'static str> {
    contents
        .trim_start_matches('\u{feff}')
        .lines()
        .filter_map(glob_text)
        .filter_map(switch_of)
        .collect()
}

/// The built-in layer without the rules in `off`.
fn build_defaults(off: &[&str]) -> Gitignore {
    let mut builder = GitignoreBuilder::new("/");
    builder
        .case_insensitive(true)
        .expect("case_insensitive never fails");
    for line in DEFAULT_RULES.iter().filter(|rule| !off.contains(rule)) {
        builder
            .add_line(None, line)
            .expect("built-in ignore rule parses");
    }
    builder.build().expect("built-in ignore rules build")
}

fn defaults() -> &'static Gitignore {
    static INSTANCE: OnceLock<Gitignore> = OnceLock::new();
    INSTANCE.get_or_init(|| build_defaults(&[]))
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
    /// The built-in layer, when the user-wide file turns some of it off.
    defaults: Option<Gitignore>,
}

/// The built-in layer `user` leaves, unless it is the whole of it.
fn defaults_left(user: Option<&RuleFile>) -> Option<Gitignore> {
    let off = switched_off(user.map_or("", |file| file.contents.as_str()));
    (!off.is_empty()).then(|| build_defaults(&off))
}

impl Rules {
    /// Load the package's `.quiltignore` and the user-wide `user_file`.
    /// Either may be missing; an unreadable or invalid one is an error.
    pub fn load(package_dir: &Path, user_file: &Path) -> Res<Rules> {
        let package = load_file(package_dir, &package_dir.join(QUILTIGNORE), false)?;
        let user =
            load_file(package_dir, user_file, true)?.map(|file| (user_file.to_path_buf(), file));
        Ok(Rules {
            package_dir: package_dir.to_path_buf(),
            package,
            defaults: defaults_left(user.as_ref().map(|(_, file)| file)),
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
        let file = parse_file(
            &self.package_dir,
            &path,
            contents,
            scope == Scope::AllPackages,
        )?;
        let (package, user, defaults) = match scope {
            Scope::Package => (Some(file), self.user.clone(), self.defaults.clone()),
            Scope::AllPackages => {
                let defaults = defaults_left(Some(&file));
                (self.package.clone(), Some((path, file)), defaults)
            }
        };
        Ok(Rules {
            package_dir: self.package_dir.clone(),
            package,
            user,
            user_file: self.user_file.clone(),
            defaults,
        })
    }

    /// The `!` lines that bring back the file or folder at `relative_path` in
    /// `scope`, outermost first: one for each folder above it the walk would
    /// still skip, then one for the path itself. Empty when nothing ignores it.
    /// `None` when `scope`'s file cannot bring it back: a package rule hides
    /// it, and the user-wide file is weaker.
    /// `tracked(path, is_dir)` says whether the package has the file, or a
    /// file in the folder.
    pub fn stop_ignoring_lines(
        &self,
        relative_path: &Path,
        is_dir: bool,
        tracked: impl Fn(&Path, bool) -> bool,
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
            if current
                .ignored_by(&prefix, dir, tracked(&prefix, dir))
                .is_none()
            {
                continue;
            }
            lines.push(stop_line(&prefix, dir, scope)?);
            let next = self.with_lines(scope, &lines)?;
            if next
                .ignored_by(&prefix, dir, tracked(&prefix, dir))
                .is_some()
            {
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
        let defaults = self.defaults.as_ref().unwrap_or_else(|| defaults());
        match defaults.matched_path_or_any_parents(relative_path, is_dir) {
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
        // folder's `/`, so those go in a class, which it leaves alone. A class
        // matches one byte, so a wider character goes in an alternation.
        if c == '\\' || (i == last && c.is_whitespace()) {
            let (open, close) = if c.is_ascii() { ('[', ']') } else { ('{', '}') };
            glob.push(open);
            glob.push(c);
            glob.push(close);
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

/// Take every line that reads as one of `lines` out of the rules file at
/// `path`, leaving the rest as it is. A missing file has none to take.
pub fn remove_lines(path: &Path, lines: &[String]) -> Res {
    let contents = match std::fs::read_to_string(path) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(read_error(path, err)),
    };
    let kept: String = contents
        .split_inclusive('\n')
        .enumerate()
        .filter(|(number, line)| {
            let line = line.trim_end_matches(['\n', '\r']);
            let line = if *number == 0 {
                line.trim_start_matches('\u{feff}')
            } else {
                line
            };
            !glob_text(line).is_some_and(|text| lines.iter().any(|l| l == text))
        })
        .map(|(_, line)| line)
        .collect();
    if kept == contents {
        return Ok(());
    }
    // Written beside the file and moved over it, so a reader never sees half of it.
    let write_error = |source| {
        Error::Fs(FsError::Write {
            path: path.to_path_buf(),
            source,
        })
    };
    let dir = path.parent().unwrap_or(Path::new("."));
    let mut temp = tempfile::NamedTempFile::new_in(dir).map_err(write_error)?;
    std::io::Write::write_all(&mut temp, kept.as_bytes()).map_err(write_error)?;
    if let Ok(metadata) = std::fs::metadata(path) {
        std::fs::set_permissions(temp.path(), metadata.permissions()).map_err(write_error)?;
    }
    temp.persist(path).map_err(|err| write_error(err.error))?;
    Ok(())
}

/// One of [`DEFAULT_RULES`], and whether it is on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DefaultRule {
    pub rule: &'static str,
    /// Off while the user-wide file has the rule's `!` line.
    pub on: bool,
}

/// The line in the user-wide file that turns a built-in rule off.
fn off_line(rule: &str) -> String {
    format!("!{rule}")
}

/// Each of [`DEFAULT_RULES`], in order, and whether the user-wide file at
/// `user_file` turns it off. A missing file turns none off.
pub fn default_rules(user_file: &Path) -> Res<Vec<DefaultRule>> {
    let contents = match std::fs::read_to_string(user_file) {
        Ok(contents) => contents,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(err) => return Err(read_error(user_file, err)),
    };
    let off = switched_off(&contents);
    Ok(DEFAULT_RULES
        .iter()
        .map(|rule| DefaultRule {
            rule,
            on: !off.contains(rule),
        })
        .collect())
}

/// Turn the built-in `rule` on or off for every package: off adds its `!`
/// line to the user-wide file at `user_file`, on takes every such line out.
pub fn set_default_rule(user_file: &Path, rule: &str, on: bool) -> Res {
    if !DEFAULT_RULES.contains(&rule) {
        return Err(Error::Fs(FsError::Write {
            path: user_file.to_path_buf(),
            source: std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                format!("{rule:?} is not a built-in rule"),
            ),
        }));
    }
    let line = off_line(rule);
    if on {
        return remove_lines(user_file, &[line]);
    }
    let off = default_rules(user_file)?
        .iter()
        .any(|default| default.rule == rule && !default.on);
    if off {
        return Ok(());
    }
    append_lines(user_file, &[line])
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
                |p, _| tracked.iter().any(|t| Path::new(t) == p),
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
        let (_dir, rules) = rules(Some("*.txt*\n"), None);
        for name in [
            "a*b[1].txt",
            "x?{y,z}.txt",
            "back\\slash.txt",
            "trailing .txt ",
            "tab.txt\t",
            "nbsp.txt\u{a0}",
            "ideographic.txt\u{3000}",
        ] {
            assert!(rules.file_ignored_by(Path::new(name), false).is_some());
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
                .stop_ignoring_lines(Path::new("a\nb.log"), false, |_, _| false, Scope::Package)
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

    #[test]
    fn remove_lines_keeps_the_rest_as_written() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join(".quiltignore");
        std::fs::write(
            &path,
            "\u{feff}!.env\r\n# note\r\n*.log\r\n!.env  \r\n!.envy",
        )
        .unwrap();
        remove_lines(&path, &["!.env".to_string()]).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "# note\r\n*.log\r\n!.envy"
        );
        let missing = dir.path().join("missing");
        remove_lines(&missing, &["!.env".to_string()]).unwrap();
        assert!(!missing.exists());
    }

    #[test]
    fn a_default_rule_is_off_while_the_user_file_has_its_line() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("domain").join(".quiltignore");
        let off = |path: &Path| -> Vec<&str> {
            default_rules(path)
                .unwrap()
                .into_iter()
                .filter(|d| !d.on)
                .map(|d| d.rule)
                .collect()
        };
        assert_eq!(default_rules(&path).unwrap().len(), DEFAULT_RULES.len());
        assert_eq!(off(&path), Vec::<&str>::new());

        set_default_rule(&path, ".DS_Store", false).unwrap();
        set_default_rule(&path, ".DS_Store", false).unwrap();
        set_default_rule(&path, ".*/", false).unwrap();
        assert_eq!(off(&path), vec![".*/", ".DS_Store"]);
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "!.DS_Store\n!.*/\n"
        );

        set_default_rule(&path, ".DS_Store", true).unwrap();
        assert_eq!(off(&path), vec![".*/"]);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "!.*/\n");

        assert!(set_default_rule(&path, "*.csv", false).is_err());
    }

    /// A rule turned off is out of the built-in layer, whatever the case of
    /// the name, and the other rules still apply inside what it matched.
    #[test]
    fn a_default_rule_turned_off_leaves_the_others_on() {
        let dir = TempDir::new().unwrap();
        let user_file = dir.path().join("user.quiltignore");
        std::fs::write(&user_file, "# mine\n.venv/\n").unwrap();
        set_default_rule(&user_file, ".*/", false).unwrap();
        set_default_rule(&user_file, "*.pem", false).unwrap();
        let rules = Rules::load(dir.path(), &user_file).unwrap();
        let ignored = |path: &str, is_dir| rules.ignored_by(Path::new(path), is_dir, false);
        assert_eq!(ignored(".mydata", true), None);
        assert_eq!(ignored(".mydata/a.csv", false), None);
        assert_eq!(ignored("key.PEM", false), None);
        assert_eq!(
            ignored(".mydata/.env", false).map(|by| by.source),
            Some(RuleSource::Default)
        );
        assert!(
            ignored(".venv", true).is_some(),
            "the user's own rule stays"
        );
    }

    /// The line a switch writes is one the layers read as bringing the files back.
    #[test]
    fn a_default_rule_turned_off_lets_its_files_in() {
        let dir = TempDir::new().unwrap();
        let user_file = dir.path().join("user.quiltignore");
        for rule in DEFAULT_RULES {
            set_default_rule(&user_file, rule, false).unwrap();
        }
        let rules = Rules::load(dir.path(), &user_file).unwrap();
        for (path, is_dir) in [
            (".snakemake", true),
            (".DS_Store", false),
            ("Icon\r", false),
            ("#draft#", false),
            ("a.pyc", false),
            ("node_modules", true),
        ] {
            assert_eq!(
                rules.ignored_by(Path::new(path), is_dir, false),
                None,
                "{path:?}"
            );
        }
    }
}
