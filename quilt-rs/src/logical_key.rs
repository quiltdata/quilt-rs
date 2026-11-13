//!
//! A package-relative file path that is known to stay inside the package folder.

use std::cmp::Ordering;
use std::fmt;
use std::hash::Hash;
use std::hash::Hasher;
use std::ops::Deref;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
use thiserror::Error;

/// The path of a package entry, relative to the package folder, with its
/// folders separated by `/`.
///
/// It only holds a key that names a place inside the package folder on every
/// OS. A key is refused when:
///
/// - split on `/`, a part is empty, `.` or `..`, as quilt3 refuses them, so it
///   can't be empty, start or end with `/`, or hold `//`;
/// - split on `/` and `\`, a part is two or more dots, spaces allowed: Windows
///   reads `\` as a separator and drops a name's trailing dots and spaces, so
///   `a\..` and `.. ` both climb out there;
/// - it starts with `\` or a drive (`C:`), which Windows reads as a root;
/// - it holds a NUL.
///
/// Anywhere else a `\` is part of a name.
///
/// Keys compare and sort by their `/`-separated names on every OS, the order
/// of quilt3's top hash, so `a/c` sorts before `a-b`, and `a\b` after both.
///
/// [`LogicalKey::join_to`] is the way to turn a key into a local path.
/// `LogicalKey` derefs to [`Path`] for reading, but on purpose isn't
/// `AsRef<Path>`, so `folder.join(&key)` doesn't compile.
#[derive(Clone, Debug)]
pub struct LogicalKey(PathBuf);

impl LogicalKey {
    fn bytes(&self) -> &[u8] {
        self.0.as_os_str().as_encoded_bytes()
    }
}

impl PartialEq for LogicalKey {
    fn eq(&self, other: &Self) -> bool {
        self.bytes() == other.bytes()
    }
}

impl Eq for LogicalKey {}

impl Hash for LogicalKey {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.bytes().hash(state);
    }
}

impl Ord for LogicalKey {
    fn cmp(&self, other: &Self) -> Ordering {
        let slash = |b: &u8| *b == b'/';
        self.bytes().split(slash).cmp(other.bytes().split(slash))
    }
}

impl PartialOrd for LogicalKey {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

/// Why a path can't be a [`LogicalKey`].
#[derive(Clone, Debug, Error, PartialEq, Eq)]
#[error("{key:?} can't be a logical key: {reason}")]
pub struct LogicalKeyError {
    pub key: String,
    pub reason: &'static str,
}

fn refusal(key: &[u8]) -> Option<&'static str> {
    if key.is_empty() {
        return Some("it is empty");
    }
    if key.contains(&0) {
        return Some("it holds a NUL");
    }
    if key.starts_with(b"\\") {
        return Some("it starts with \\");
    }
    if key.len() >= 2 && key[0].is_ascii_alphabetic() && key[1] == b':' {
        return Some("it starts with a drive");
    }
    for part in key.split(|b| *b == b'/') {
        if part.is_empty() {
            return Some("it has an empty folder or file name");
        }
        if part == b"." || part == b".." {
            return Some("it has a . or .. folder");
        }
        for piece in part.split(|b| *b == b'\\') {
            let mut dots = piece.iter().filter(|b| **b == b'.');
            if dots.nth(1).is_some() && piece.iter().all(|b| *b == b'.' || *b == b' ') {
                return Some("it has a .. folder");
            }
        }
    }
    None
}

impl LogicalKey {
    /// Checks `key` and wraps it.
    ///
    /// # Errors
    ///
    /// When `key` names a place outside the package folder; see [`LogicalKey`].
    pub fn new(key: impl Into<PathBuf>) -> Result<Self, LogicalKeyError> {
        let key = key.into();
        match refusal(key.as_os_str().as_encoded_bytes()) {
            Some(reason) => Err(LogicalKeyError {
                key: key.to_string_lossy().into_owned(),
                reason,
            }),
            None => Ok(Self(key)),
        }
    }

    /// The local path of this key inside `folder`.
    #[must_use]
    pub fn join_to(&self, folder: &Path) -> PathBuf {
        let path = folder.join(&self.0);
        debug_assert!(path.starts_with(folder));
        path
    }

    /// Whether `other` is inside this key taken as a folder, by `/` names.
    pub(crate) fn is_folder_of(&self, other: &Self) -> bool {
        other
            .bytes()
            .strip_prefix(self.bytes())
            .is_some_and(|rest| rest.starts_with(b"/"))
    }

    #[must_use]
    pub fn as_path(&self) -> &Path {
        &self.0
    }
}

/// For tests that need a row and don't care which file it names.
#[cfg(any(test, feature = "testing"))]
impl Default for LogicalKey {
    fn default() -> Self {
        Self(PathBuf::from("file"))
    }
}

impl Deref for LogicalKey {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl fmt::Display for LogicalKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.display().fmt(f)
    }
}

impl From<LogicalKey> for PathBuf {
    fn from(key: LogicalKey) -> Self {
        key.0
    }
}

impl TryFrom<PathBuf> for LogicalKey {
    type Error = LogicalKeyError;

    fn try_from(key: PathBuf) -> Result<Self, Self::Error> {
        Self::new(key)
    }
}

impl TryFrom<&Path> for LogicalKey {
    type Error = LogicalKeyError;

    fn try_from(key: &Path) -> Result<Self, Self::Error> {
        Self::new(key)
    }
}

impl TryFrom<&str> for LogicalKey {
    type Error = LogicalKeyError;

    fn try_from(key: &str) -> Result<Self, Self::Error> {
        Self::new(key)
    }
}

impl TryFrom<String> for LogicalKey {
    type Error = LogicalKeyError;

    fn try_from(key: String) -> Result<Self, Self::Error> {
        Self::new(key)
    }
}

impl FromStr for LogicalKey {
    type Err = LogicalKeyError;

    fn from_str(key: &str) -> Result<Self, Self::Err> {
        Self::new(key)
    }
}

impl Serialize for LogicalKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LogicalKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let key = PathBuf::deserialize(deserializer)?;
        Self::new(key).map_err(serde::de::Error::custom)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Res;

    #[test]
    fn keys_sort_by_their_slash_separated_names() -> Res {
        let mut keys = ["a\\b", "a-b", "a/c"]
            .into_iter()
            .map(LogicalKey::new)
            .collect::<Result<Vec<_>, _>>()?;
        keys.sort();
        let sorted: Vec<String> = keys.iter().map(ToString::to_string).collect();
        assert_eq!(sorted, ["a/c", "a-b", "a\\b"]);
        assert_ne!(LogicalKey::new("a/b")?, LogicalKey::new("a\\b")?);
        Ok(())
    }

    #[test]
    fn keys_inside_the_package_are_accepted() {
        for key in [
            "a",
            "a/b/c.txt",
            ".hidden",
            "a/.b/..c/c..",
            "...txt",
            "a b/c d.md",
            "data\\raw\\a.csv",
            "a/b:c.txt",
            "2024-01-01T10:00:00.csv",
            "a/C:x",
            "a/\\\\server\\share",
        ] {
            assert!(LogicalKey::new(key).is_ok(), "{key}");
        }
    }

    #[test]
    fn keys_leaving_the_package_are_refused() {
        for key in [
            "",
            "/",
            "/etc/passwd",
            "a/",
            "a//b",
            ".",
            "..",
            "./a",
            "a/./b",
            "../a",
            "a/../../b",
            "a/..",
            "..\\a",
            "a\\..\\..\\b",
            "a/b\\..",
            "...",
            ".. ",
            "a/. .",
            "\\a",
            "\\\\server\\share\\a",
            "\\\\?\\C:\\a",
            "//server/share/a",
            "\\\\.\\pipe\\a",
            "\\??\\C:\\a",
            "/??/C:/a",
            "C:/a",
            "C:..",
            "C:",
            "C:\\a",
            "c:a",
            "a\0b",
        ] {
            assert!(LogicalKey::new(key).is_err(), "{key:?}");
        }
    }

    #[test]
    fn a_key_joins_inside_its_folder() -> Res {
        let folder = Path::new("home").join("pkg");
        let key = LogicalKey::new("a/b.txt")?;
        assert_eq!(key.join_to(&folder), folder.join("a").join("b.txt"));
        Ok(())
    }

    #[test]
    fn a_manifest_key_is_checked_when_read() -> Res {
        let key: LogicalKey = serde_json::from_str("\"a/b.txt\"")?;
        assert_eq!(key.as_path(), Path::new("a/b.txt"));
        assert_eq!(serde_json::to_string(&key)?, "\"a/b.txt\"");

        let err = serde_json::from_str::<LogicalKey>("\"../b.txt\"").unwrap_err();
        assert!(err.to_string().contains("can't be a logical key"), "{err}");
        Ok(())
    }
}
