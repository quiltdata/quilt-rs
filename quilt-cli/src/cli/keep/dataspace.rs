//! `quilt keep --dataspace`: what a dataspace keeps of a package, its
//! paths, its pin and its list, changed in `data.toml`, with the paths it
//! newly reads read at the lock's revision.

use std::path::PathBuf;

use quilt_rs::dataspace::conflict::CONFLICT_FILE;
use quilt_rs::dataspace::contract::Access;
use quilt_rs::dataspace::contract::CONTRACT_FILE;
use quilt_rs::dataspace::contract::PackagePath;
use quilt_rs::dataspace::contract::Paths;
use quilt_rs::dataspace::keep::Keep;
use quilt_rs::dataspace::keep::KeepRead;
use quilt_rs::dataspace::keep::Kept;
use quilt_rs::dataspace::keep::Pin;
use quilt_rs::dataspace::keep::Scope;
use quilt_rs::dataspace::keep::keep;
use quilt_rs::dataspace::read::ReadOutcome;
use quilt_rs::domain::DomainRoot;
use quilt_rs::domain::LocalRegistry;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::paths::DomainPaths;
use quilt_uri::RevisionPointer;
use quilt_uri::Tag;

use crate::cli::Error;
use crate::cli::dataspace;
use crate::cli::dataspace::Package;
use crate::cli::output::Render;
use crate::cli::output::Std;
use crate::cli::text::printable;
use crate::cli::text::printable_line;

#[derive(Debug)]
pub struct Input {
    /// The dataspace: the folder holding `data.toml`.
    pub dataspace: PathBuf,
    pub package: Package,
    /// The list whose entry changes, where the package is in two.
    pub from: Option<Access>,
    pub paths: Option<Scope>,
    pub revision: Option<Pin>,
    pub access: Option<Access>,
}

/// The paths a set of `quilt keep` flags asks for: `--all`, or each
/// `--folder`, as a folder, and each `--path` as given; `None` for none.
pub fn scope_from(all: bool, folders: Vec<String>, paths: Vec<String>) -> Option<Scope> {
    if all {
        return Some(Scope::Whole);
    }
    let mut scope: Vec<String> = folders
        .into_iter()
        .map(|folder| format!("{}/", folder.trim_end_matches('/')))
        .collect();
    scope.extend(paths);
    (!scope.is_empty()).then_some(Scope::Only(scope))
}

/// The pin a `--pin` or `--unpin` asks for.
pub fn pin_from(pin: Option<String>, unpin: bool) -> Option<Pin> {
    match (pin, unpin) {
        (Some(pin), _) => Some(Pin::At(pin)),
        (None, true) => Some(Pin::Latest),
        (None, false) => None,
    }
}

pub struct Output {
    kept: Kept,
}

impl Output {
    /// The entry's paths, `None` for the whole package.
    fn paths(&self) -> Option<Vec<String>> {
        match &self.kept.entry.paths {
            Paths::Whole => None,
            Paths::Only(paths) => Some(paths.iter().map(PackagePath::spelled).collect()),
        }
    }

    /// The entry's pin, as `data.toml` spells it.
    fn pin(&self) -> Option<String> {
        match &self.kept.entry.revision {
            RevisionPointer::Tag(Tag::Latest) => None,
            RevisionPointer::Hash(hash) | RevisionPointer::HashPrefix(hash) => Some(hash.clone()),
            RevisionPointer::Tag(tag) => Some(format!(":{tag}")),
        }
    }

    /// How the read of the paths newly read ended, as `--json` names it.
    fn outcome(&self) -> &'static str {
        let read = &self.kept.read;
        match read {
            KeepRead::Nothing => "nothing",
            KeepRead::Ran(ReadOutcome::Read(_)) if read.completed() => "read",
            KeepRead::Ran(ReadOutcome::Read(_)) => "incomplete",
            KeepRead::Ran(ReadOutcome::Blocked { .. }) => "blocked",
            KeepRead::Unpushed => "unpushed",
            KeepRead::Failed(_) => "failed",
            _ => "unknown",
        }
    }
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entry = &self.kept.entry;
        let package = entry.key();
        let access = entry.access;
        let paths = match self.paths() {
            None => "every file".to_string(),
            Some(paths) => printable_line(&paths.join(", ")),
        };
        write!(f, "{package} keeps {paths} under `{access}`")?;
        if let Some(pin) = self.pin() {
            write!(f, ", pinned to {pin}")?;
        }
        let folder = printable_line(&entry.folder.display().to_string());
        write!(f, ", in {folder}")?;
        if !self.kept.declared {
            write!(f, "\n  {CONTRACT_FILE} already said so; nothing changed")?;
        }
        match &self.kept.read {
            KeepRead::Ran(ReadOutcome::Read(report)) if self.kept.read.completed() => write!(
                f,
                "\n  read {} newly kept file(s) at {}",
                report.placed.len(),
                report.revision
            )?,
            KeepRead::Ran(ReadOutcome::Read(report)) => write!(
                f,
                "\n  {} cannot be read whole under the new paths, so the lock was left",
                report.revision
            )?,
            KeepRead::Ran(ReadOutcome::Blocked { revision, .. }) => write!(
                f,
                "\n  blocked; local changes collide with {revision}. Decide each in {CONFLICT_FILE}, then read again"
            )?,
            KeepRead::Unpushed => write!(
                f,
                "\n  the new paths were not read; the lock names local commits not yet pushed"
            )?,
            KeepRead::Failed(err) => write!(
                f,
                "\n  the new paths were not read: {}",
                printable(&err.to_string())
            )?,
            _ => {}
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let entry = &self.kept.entry;
        let placed = match &self.kept.read {
            KeepRead::Ran(ReadOutcome::Read(report)) => report
                .placed
                .iter()
                .map(|path| path.display().to_string())
                .collect(),
            _ => Vec::new(),
        };
        serde_json::json!({
            "package": entry.key().to_string(),
            "access": entry.access.to_string(),
            "paths": self.paths(),
            "revision": self.pin(),
            "folder": entry.folder.display().to_string(),
            "declared": self.kept.declared,
            "outcome": self.outcome(),
            "completed": self.kept.read.completed(),
            "placed": placed,
        })
    }

    fn completed(&self) -> bool {
        self.kept.read.completed()
    }
}

pub async fn command(credentials: PathBuf, domain: Option<DomainRoot>, args: Input) -> Std {
    let remote = RemoteS3::new(DomainPaths::new(credentials), LocalStorage::new());
    let local = domain.as_ref().map(DomainRoot::local);
    Std::from_result(keep_with(&remote, local.as_ref(), args).await)
}

async fn keep_with(
    remote: &impl Remote,
    local: Option<&LocalRegistry>,
    args: Input,
) -> Result<Output, Error> {
    let writer = dataspace::writer(local).await?;
    let package = args.package.key(&args.dataspace, &writer).await?;
    let mut request = Keep::new(package);
    request.from = args.from;
    request.paths = args.paths;
    request.revision = args.revision;
    request.access = args.access;
    let kept = Box::pin(keep(
        &LocalStorage::new(),
        remote,
        local,
        &args.dataspace,
        &writer,
        &request,
    ))
    .await?;
    Ok(Output { kept })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use quilt_rs::io::remote::mocks::MockRemote;
    use quilt_uri::Namespace;

    fn input(dataspace: &std::path::Path) -> Input {
        Input {
            dataspace: dataspace.to_path_buf(),
            package: Package::Namespace(Namespace::try_from("team/raw").expect("a name")),
            from: None,
            paths: None,
            revision: None,
            access: None,
        }
    }

    #[test]
    fn the_flags_pick_the_paths_and_the_pin() {
        assert_eq!(scope_from(false, Vec::new(), Vec::new()), None);
        assert_eq!(
            scope_from(true, vec!["a".to_string()], Vec::new()),
            Some(Scope::Whole)
        );
        assert_eq!(
            scope_from(
                false,
                vec!["data".to_string(), "docs/".to_string()],
                vec!["a.txt".to_string()]
            ),
            Some(Scope::Only(vec![
                "data/".to_string(),
                "docs/".to_string(),
                "a.txt".to_string()
            ]))
        );
        assert_eq!(pin_from(None, false), None);
        assert_eq!(pin_from(None, true), Some(Pin::Latest));
        assert_eq!(
            pin_from(Some("abc".to_string()), false),
            Some(Pin::At("abc".to_string()))
        );
    }

    /// A `write` entry's paths narrowed, by its name: `data.toml` says so,
    /// and nothing is read.
    #[test(tokio::test)]
    async fn keep_rewrites_the_entry_and_says_what_it_keeps() -> Result<(), Error> {
        let dataspace = tempfile::TempDir::new()?;
        std::fs::write(
            dataspace.path().join("data.toml"),
            "registry = \"s3://b\"\n[packages]\nwrite = [\"team/raw\"]\n",
        )?;
        let mut args = input(dataspace.path());
        args.paths = scope_from(false, vec!["out".to_string()], Vec::new());

        let output = keep_with(&MockRemote::default(), None, args).await?;

        let json = output.to_json();
        assert_eq!(json["paths"], serde_json::json!(["out/"]), "{json}");
        assert_eq!(json["access"], "write");
        assert_eq!(json["declared"], true);
        assert_eq!(json["outcome"], "nothing");
        assert!(
            output
                .to_string()
                .starts_with("quilt+s3://b#package=team/raw keeps out/ under `write`, in "),
            "{output}"
        );
        let text = std::fs::read_to_string(dataspace.path().join("data.toml"))?;
        assert!(text.contains("path = \"out/\""), "{text}");
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_pin_outside_read_is_a_contract_error() -> Result<(), Error> {
        let dataspace = tempfile::TempDir::new()?;
        std::fs::write(
            dataspace.path().join("data.toml"),
            "registry = \"s3://b\"\n[packages]\nsync = [\"team/raw\"]\n",
        )?;
        let mut args = input(dataspace.path());
        args.revision = pin_from(Some("abcdef1234".to_string()), false);

        let std = Std::from_result(keep_with(&MockRemote::default(), None, args).await);

        let Std::Err(err) = std else {
            panic!("expected an error, got {std:?}");
        };
        assert_eq!(err.kind(), "contract");
        Ok(())
    }
}
