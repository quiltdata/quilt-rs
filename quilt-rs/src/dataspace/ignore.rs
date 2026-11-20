//! A dataspace's `[ignore]` table in `data.toml`: what packaging any of its
//! folders leaves out, by commit or publish. It governs only what is
//! packaged, never what a read writes, and no `.quiltignore` is written for
//! it.

use serde::Deserialize;

use crate::quiltignore::line_problem;

/// The `[ignore]` table. It covers the whole dataspace: an entry cannot
/// override it. Any other key is refused.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Ignore {
    /// Leave out the folders of the packages `data.toml` declares. A folder
    /// with a `data.toml` of its own is left out either way: it is another
    /// dataspace.
    pub packages: bool,
    /// Leave out `data.lock.toml`.
    pub lockfile: bool,
    /// Keep quilt-rs's built-in rules on. Off, the
    /// [secrets](crate::quiltignore::SECRET_RULES) are still left out.
    pub standard_patterns: bool,
    /// Gitignore lines just above the built-in rules, for new files only, so
    /// the package's `.quiltignore` and the user-wide file still win.
    pub extend_patterns: Vec<String>,
}

impl Default for Ignore {
    fn default() -> Self {
        Self {
            packages: true,
            lockfile: true,
            standard_patterns: true,
            extend_patterns: Vec::new(),
        }
    }
}

impl Ignore {
    /// The place in `extend_patterns`, 0-based, and the reason, of the first
    /// line that is not a valid gitignore line.
    pub(crate) fn problem(&self) -> Option<(usize, String)> {
        self.extend_patterns
            .iter()
            .enumerate()
            .find_map(|(index, line)| line_problem(line).map(|reason| (index, reason)))
    }
}
