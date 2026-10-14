//! A file row's `[⋯]`: the commands it can offer, their words and their
//! groups. The package page's file pane and the commit page's list each decide
//! which apply to a row; their pages run them.

use leptos::prelude::*;

use crate::kit::MenuAction;

/// One command a row's `[⋯]` offers, naming the row it acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowCommand {
    /// Open the local file in its default application.
    Open(String),
    /// Show the local file in the system's file browser.
    Reveal(String),
    OpenInCatalog(String),
    /// The file's `quilt+s3` address, as `util::file_uri` builds it.
    CopyUri(String),
    /// The file's full path on disk, for a file whose address would not name
    /// its bytes: no revision holds them. Carries its path inside the package;
    /// the page resolves where that is.
    CopyPath(String),
    /// Open the ignore popup with this path as the pattern.
    Ignore(String),
    /// Open the unignore popup for the pattern that matched this path.
    StopIgnoring {
        path: String,
        pattern: String,
    },
}

impl RowCommand {
    /// The item's words.
    #[must_use]
    pub const fn label(&self) -> &'static str {
        match self {
            Self::Open(_) => "Open file",
            Self::Reveal(_) => "Reveal in folder",
            Self::OpenInCatalog(_) => "Open in catalog",
            Self::CopyUri(_) => "Copy URI",
            Self::CopyPath(_) => "Copy path",
            Self::Ignore(_) => "Ignore",
            Self::StopIgnoring { .. } => "Stop ignoring",
        }
    }

    /// The row it acts on.
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            Self::Open(path)
            | Self::Reveal(path)
            | Self::OpenInCatalog(path)
            | Self::CopyUri(path)
            | Self::CopyPath(path)
            | Self::Ignore(path)
            | Self::StopIgnoring { path, .. } => path,
        }
    }

    /// Which of the menu's groups it sits in: the local file, its address, then
    /// `.quiltignore`. A rule goes between two groups.
    const fn group(&self) -> u8 {
        match self {
            Self::Open(_) | Self::Reveal(_) => 0,
            Self::OpenInCatalog(_) | Self::CopyUri(_) | Self::CopyPath(_) => 1,
            Self::Ignore(_) | Self::StopIgnoring { .. } => 2,
        }
    }

    /// Whether it leads to an edit of `.quiltignore`, and so waits for the
    /// page's lock. The others write nothing, as the row's own open does not.
    pub(crate) const fn edits_ignore(&self) -> bool {
        matches!(self, Self::Ignore(_) | Self::StopIgnoring { .. })
    }
}

/// Whether each command starts a group after another, and so sits below a
/// rule. The first item never does, and a group a row lacks draws no rule.
pub(crate) fn group_starts(commands: &[RowCommand]) -> Vec<bool> {
    commands
        .iter()
        .enumerate()
        .map(|(i, command)| i > 0 && commands[i - 1].group() != command.group())
        .collect()
}

/// One item per command, handing it to `on_command`, grouped as
/// [`group_starts`] says.
pub(crate) fn menu_actions(
    commands: &[RowCommand],
    on_command: Callback<RowCommand>,
) -> Vec<MenuAction> {
    commands
        .iter()
        .zip(group_starts(commands))
        .map(|(command, starts)| {
            let run = command.clone();
            let action = MenuAction::new(
                command.label(),
                Callback::new(move |()| on_command.run(run.clone())),
            );
            if starts { action.separated() } else { action }
        })
        .collect()
}
