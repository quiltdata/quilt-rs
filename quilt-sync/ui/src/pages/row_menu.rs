//! A file row's `[⋯]`: the commands it can offer, their words and their
//! groups, and [`carry_out`] for the ones that write nothing. The package
//! page's file pane, the commit page's list and the main page's recent files
//! each decide which apply to a row; their pages run them and report how it
//! went in their own way.

use std::future::Future;
use std::pin::Pin;

use leptos::prelude::*;

use crate::commands;
use crate::kit::MenuAction;

/// One command a row's `[⋯]` offers, naming the row it acts on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowCommand {
    /// Open the local file in its default application.
    Open(String),
    /// Show the local file in the system's file browser.
    Reveal(String),
    OpenInCatalog(String),
    /// The file's `quilt+s3` address at the revision the package holds, as
    /// `util::file_uri` builds it.
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
        folder: bool,
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
pub fn menu_actions(commands: &[RowCommand], on_command: Callback<RowCommand>) -> Vec<MenuAction> {
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

/// How a command that writes nothing went, for the page to report.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Ran {
    /// It ran, and shows itself: an application, a folder or the browser
    /// opens. Also a command with nothing to act on, such as a catalog link
    /// with no catalog host.
    Done,
    /// A copy landed, with what it put on the clipboard.
    Copied(String),
    /// It failed: the page's words, and the engine's.
    Failed { lead: &'static str, detail: String },
}

/// The page's words for a copy that failed.
pub(crate) const COPY_FAILED: &str = "Could not copy.";

/// Runs a row's command that writes nothing: opening, revealing, the catalog
/// and the copies. None needs a page's lock. `uri` is the package's remote, if
/// it has one; the `.quiltignore` items are each page's own and come back
/// [`Ran::Done`].
///
/// A copy asks `fresh` once its text is ready, and writes only while it says
/// yes: a copy clicked later must not lose the clipboard to a slower earlier
/// one.
pub(crate) async fn carry_out(
    command: &RowCommand,
    namespace: &str,
    uri: Option<&quilt_uri::S3PackageUri>,
    local: LocalPath,
    fresh: impl Fn() -> bool,
) -> Ran {
    let failed = |lead| move |detail| Ran::Failed { lead, detail };
    match command {
        RowCommand::Open(path) => {
            commands::open_in_default_application(namespace.to_string(), path.clone(), uri.cloned())
                .await
                .map_or_else(failed("Could not open this file."), |_| Ran::Done)
        }
        RowCommand::Reveal(path) => {
            commands::reveal_in_file_browser(namespace.to_string(), path.clone(), uri.cloned())
                .await
                .map_or_else(failed("Could not show this file in its folder."), |_| {
                    Ran::Done
                })
        }
        RowCommand::OpenInCatalog(path) => {
            let Some(url) = uri.and_then(|u| crate::util::entry_catalog_url(u, path)) else {
                return Ran::Done;
            };
            commands::open_in_web_browser(url)
                .await
                .map_or_else(failed("Could not open this file in the catalog."), |_| {
                    Ran::Done
                })
        }
        RowCommand::CopyUri(_) | RowCommand::CopyPath(_) => {
            let copied = match clipped(command, namespace, uri, local).await {
                Ok(None) => return Ran::Done,
                Ok(Some(_)) if !fresh() => return Ran::Done,
                Ok(Some((text, event))) => commands::copy_to_clipboard(text.clone(), event)
                    .await
                    .map(|_| text),
                Err(detail) => Err(detail),
            };
            copied.map_or_else(failed(COPY_FAILED), Ran::Copied)
        }
        RowCommand::Ignore(_) | RowCommand::StopIgnoring { .. } => Ran::Done,
    }
}

/// Where a package's file is on disk: `(namespace, path)` to the full path.
/// A seam, so a test can answer without a Tauri host.
pub(crate) type LocalPath =
    fn(String, String) -> Pin<Box<dyn Future<Output = Result<String, String>>>>;

pub(crate) fn local_path(
    namespace: String,
    path: String,
) -> Pin<Box<dyn Future<Output = Result<String, String>>>> {
    Box::pin(commands::package_file_path(namespace, path))
}

/// What a copy puts on the clipboard, and the address its event names:
/// `util::file_uri` for *Copy URI*, and for *Copy path* the file's full path
/// on disk, which a `New` file has though no revision holds it. `Ok(None)` for
/// a command that copies nothing, or an address with no remote to build it
/// from; an error when the path cannot be found.
///
/// The address names the revision `uri` names, with the whole hash: each list
/// passes the revision the package holds, never `Latest`.
pub(crate) async fn clipped(
    command: &RowCommand,
    namespace: &str,
    uri: Option<&quilt_uri::S3PackageUri>,
    local: LocalPath,
) -> Result<Option<(String, Option<quilt_uri::S3PackageUri>)>, String> {
    let file = |path: &str| uri.map(|u| crate::util::file_uri(u, path));
    match command {
        RowCommand::CopyUri(path) => Ok(file(path).map(|f| (f.to_copy_string(), Some(f)))),
        RowCommand::CopyPath(path) => {
            let on_disk = local(namespace.to_string(), path.clone()).await?;
            Ok(Some((on_disk, file(path))))
        }
        _ => Ok(None),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::*;

    /// Where this test's package is on disk, as the backend would answer.
    fn at_home(
        namespace: String,
        path: String,
    ) -> Pin<Box<dyn Future<Output = Result<String, String>>>> {
        Box::pin(async move { Ok(format!("/Users/me/QuiltSync/{namespace}/{path}")) })
    }

    fn not_there(_: String, path: String) -> Pin<Box<dyn Future<Output = Result<String, String>>>> {
        Box::pin(async move { Err(format!("{path} is not there")) })
    }

    /// Copy URI puts the file's address at the held revision on the clipboard,
    /// with the whole hash and the path escaped; Copy path puts the file's full
    /// path on disk, which needs no remote.
    #[wasm_bindgen_test]
    async fn copy_uri_copies_the_address_and_copy_path_the_full_path_on_disk() {
        let hash = "0123456789abcdef".repeat(4);
        let package = quilt_uri::S3PackageUri {
            revision: quilt_uri::RevisionPointer::Hash(hash.clone()),
            ..crate::util::package_uri(
                "team-bucket",
                &"user/plate-07".try_into().unwrap(),
                Some("quilt.test"),
            )
        };
        let copy_uri = RowCommand::CopyUri("runs/one two.csv".to_string());
        let (text, event) = clipped(&copy_uri, "user/plate-07", Some(&package), at_home)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            text,
            format!(
                "quilt+s3://team-bucket#package=user/plate-07@{hash}\
                 &path=runs/one%20two.csv&catalog=quilt.test"
            )
        );
        assert_eq!(event.map(|u| u.to_copy_string()), Some(text));

        // A New row: the full path on disk, as opening the file resolves it.
        let copy_path = RowCommand::CopyPath("runs/new.csv".to_string());
        let (text, _) = clipped(&copy_path, "user/plate-07", None, at_home)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(text, "/Users/me/QuiltSync/user/plate-07/runs/new.csv");
        assert_eq!(
            clipped(&copy_path, "user/plate-07", None, not_there).await,
            Err("runs/new.csv is not there".to_string()),
            "a path that cannot be found is a failed copy, not an empty one",
        );

        assert_eq!(
            clipped(
                &RowCommand::CopyUri("a.csv".to_string()),
                "user/plate-07",
                None,
                at_home
            )
            .await,
            Ok(None),
            "no remote, no address",
        );
        assert_eq!(
            clipped(
                &RowCommand::Ignore("a.csv".to_string()),
                "user/plate-07",
                Some(&package),
                at_home
            )
            .await,
            Ok(None),
        );
    }

    /// A command with nothing to act on is done without asking the host; one
    /// the host refuses fails in the page-facing words. There is no Tauri host
    /// in this harness, so every call it makes is refused.
    #[wasm_bindgen_test]
    async fn carry_out_says_how_a_command_went() {
        let ns = "user/plate-07";
        let no_catalog = crate::util::package_uri("team-bucket", &ns.try_into().unwrap(), None);
        let catalog = RowCommand::OpenInCatalog("a.csv".to_string());
        assert_eq!(
            carry_out(&catalog, ns, Some(&no_catalog), at_home, || true).await,
            Ran::Done,
            "no catalog host, no link to open",
        );
        assert_eq!(
            carry_out(
                &RowCommand::CopyUri("a.csv".to_string()),
                ns,
                None,
                at_home,
                || true
            )
            .await,
            Ran::Done,
            "no remote, no address to copy",
        );
        assert_eq!(
            carry_out(
                &RowCommand::Ignore("a.csv".to_string()),
                ns,
                None,
                at_home,
                || true
            )
            .await,
            Ran::Done,
        );
        let refused = carry_out(
            &RowCommand::CopyUri("a.csv".to_string()),
            ns,
            Some(&no_catalog),
            at_home,
            || true,
        )
        .await;
        assert!(
            matches!(
                refused,
                Ran::Failed {
                    lead: "Could not copy.",
                    ..
                }
            ),
            "got {refused:?}"
        );
    }
}
