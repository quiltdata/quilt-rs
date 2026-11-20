//! `quilt install --dataspace`: the package added to a dataspace's
//! `data.toml`, then read into its working folder.

use std::path::PathBuf;

use quilt_rs::dataspace::conflict::CONFLICT_FILE;
use quilt_rs::dataspace::contract::Access;
use quilt_rs::dataspace::contract::CONTRACT_FILE;
use quilt_rs::dataspace::install::Install;
use quilt_rs::dataspace::install::InstallRead;
use quilt_rs::dataspace::install::Installed;
use quilt_rs::dataspace::install::install;
use quilt_rs::dataspace::read::ReadOutcome;
use quilt_rs::dataspace::read_all::EntryRead;
use quilt_rs::domain::DomainRoot;
use quilt_rs::domain::LocalRegistry;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::paths::DomainPaths;

use crate::cli::Error;
use crate::cli::dataspace;
use crate::cli::output::Render;
use crate::cli::output::Std;
use crate::cli::read;
use crate::cli::text::printable;
use crate::cli::text::printable_line;

#[derive(Debug)]
pub struct Input {
    /// The dataspace: the folder holding `data.toml`.
    pub dataspace: PathBuf,
    pub uri: String,
    pub access: Access,
    /// Each a logical key, or a folder of them ending in `/`.
    pub paths: Vec<PathBuf>,
    pub folder: Option<PathBuf>,
}

pub struct Output {
    installed: Installed,
}

/// How the install's read ended, as `--json` names it.
fn outcome(read: &InstallRead) -> &'static str {
    match read {
        InstallRead::NotRead => "not_read",
        InstallRead::Unchanged => "unchanged",
        InstallRead::Ran(ReadOutcome::Read(_)) if read.completed() => "read",
        InstallRead::Ran(ReadOutcome::Read(_)) => "incomplete",
        InstallRead::Ran(ReadOutcome::Blocked { .. }) => "blocked",
        InstallRead::Unpushed => "unpushed",
        InstallRead::Failed(_) => "failed",
        _ => "unknown",
    }
}

/// The read as [`read`] reports one, where it ran.
fn as_entry_read(read: &InstallRead) -> Option<EntryRead> {
    match read {
        InstallRead::Ran(outcome) => Some(EntryRead::Ran(outcome.clone())),
        InstallRead::Unpushed => Some(EntryRead::Unpushed),
        _ => None,
    }
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entry = &self.installed.entry;
        let package = entry.key();
        let access = entry.access;
        let folder = printable_line(&entry.folder.display().to_string());
        if self.installed.declared {
            write!(
                f,
                "{package}: added to {CONTRACT_FILE} under `{access}`, in {folder}"
            )?;
        } else {
            write!(
                f,
                "{package}: already in {CONTRACT_FILE} under `{access}`, in {folder}"
            )?;
        }
        let read = &self.installed.read;
        match read {
            InstallRead::NotRead => write!(f, "\n  not read: `{access}` entries are not read")?,
            InstallRead::Unchanged => {
                write!(f, "\n  already holds the revision the lock names")?;
            }
            InstallRead::Ran(ReadOutcome::Read(report)) if read.completed() => {
                write!(f, "\n  read {}", report.revision)?;
            }
            InstallRead::Ran(ReadOutcome::Read(report)) => write!(
                f,
                "\n  {} cannot be read whole, so nothing changed",
                report.revision
            )?,
            InstallRead::Ran(ReadOutcome::Blocked { revision, .. }) => write!(
                f,
                "\n  blocked, so nothing was read; local changes collide with {revision}. Decide each in {CONFLICT_FILE}, then read again"
            )?,
            InstallRead::Unpushed => write!(
                f,
                "\n  not read; the lock names local commits not yet pushed"
            )?,
            InstallRead::Failed(err) => {
                write!(f, "\n  read failed: {}", printable(&err.to_string()))?;
            }
            _ => write!(f, "\n  not read")?,
        }
        if let Some(read) = as_entry_read(read) {
            for (_, heading, paths) in read::lists(&read) {
                read::group(f, heading, paths)?;
            }
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let entry = &self.installed.entry;
        let read = &self.installed.read;
        let mut json = as_entry_read(read).map_or_else(
            || read::read_json(&EntryRead::Unpushed),
            |read| read::read_json(&read),
        );
        json["outcome"] = outcome(read).into();
        json["completed"] = read.completed().into();
        json["error"] = match read {
            InstallRead::Failed(err) => err.to_string().into(),
            _ => serde_json::Value::Null,
        };
        json["package"] = entry.key().to_string().into();
        json["access"] = entry.access.to_string().into();
        json["folder"] = entry.folder.display().to_string().into();
        json["declared"] = self.installed.declared.into();
        json
    }

    fn completed(&self) -> bool {
        self.installed.read.completed()
    }
}

pub async fn command(credentials: PathBuf, domain: Option<DomainRoot>, args: Input) -> Std {
    let remote = RemoteS3::new(DomainPaths::new(credentials), LocalStorage::new());
    let local = domain.as_ref().map(DomainRoot::local);
    Std::from_result(install_with(&remote, local.as_ref(), args).await)
}

async fn install_with(
    remote: &impl Remote,
    local: Option<&LocalRegistry>,
    args: Input,
) -> Result<Output, Error> {
    let writer = dataspace::writer(local).await?;
    let mut request = Install::new(args.uri);
    request.access = args.access;
    request.paths = args
        .paths
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    request.folder = args.folder;
    let installed = install(
        &LocalStorage::new(),
        remote,
        local,
        &args.dataspace,
        &writer,
        &request,
    )
    .await?;
    Ok(Output { installed })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use quilt_rs::io::remote::mocks::MockRemote;
    use quilt_rs::manifest::Manifest;
    use quilt_rs::manifest::ManifestRow;
    use quilt_uri::S3Uri;

    const FIRST: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    /// Stores `a.txt` holding `alpha` as revision [`FIRST`] of `team/raw` in
    /// bucket `b`, and tags it `latest`.
    async fn publish(remote: &MockRemote) -> Result<(), Error> {
        let body = b"alpha";
        let physical_key = format!("s3://b/team/raw/a.txt?versionId={FIRST}");
        remote
            .put_object(
                None,
                &S3Uri::try_from(physical_key.as_str())?,
                body.to_vec(),
            )
            .await?;
        let manifest = Manifest {
            rows: vec![ManifestRow {
                logical_key: quilt_rs::LogicalKey::new("a.txt")?,
                physical_key,
                hash: quilt_rs::checksum::hash_reader(&body[..], body.len() as u64, 0x12).await?,
                size: body.len() as u64,
                ..ManifestRow::default()
            }],
            ..Manifest::default()
        };
        remote
            .put_object(
                None,
                &S3Uri::try_from(format!("s3://b/.quilt/packages/{FIRST}").as_str())?,
                manifest.to_jsonlines().into_bytes(),
            )
            .await?;
        remote
            .put_object(
                None,
                &S3Uri::try_from("s3://b/.quilt/named_packages/team/raw/latest")?,
                FIRST.as_bytes().to_vec(),
            )
            .await?;
        Ok(())
    }

    fn input(dataspace: &std::path::Path, access: Access) -> Input {
        Input {
            dataspace: dataspace.to_path_buf(),
            uri: "quilt+s3://b#package=team/raw".to_string(),
            access,
            paths: Vec::new(),
            folder: None,
        }
    }

    #[test(tokio::test)]
    async fn an_install_reports_the_entry_added_and_the_revision_read() -> Result<(), Error> {
        let remote = MockRemote::default();
        publish(&remote).await?;
        let dataspace = tempfile::TempDir::new()?;
        std::fs::write(
            dataspace.path().join("data.toml"),
            "registry = \"s3://b\"\n",
        )?;

        let first = Std::from_result(
            install_with(&remote, None, input(dataspace.path(), Access::Sync)).await,
        );
        let again = Std::from_result(
            install_with(&remote, None, input(dataspace.path(), Access::Sync)).await,
        );

        let (Std::Out(first), Std::Out(again)) = (first, again) else {
            panic!("expected two installs");
        };
        let json = first.to_json();
        assert_eq!(json["outcome"], "read", "{json}");
        assert_eq!(json["revision"], FIRST);
        assert_eq!(json["declared"], true);
        assert_eq!(
            json["placed"],
            serde_json::json!(["a.txt", "metadata.toml"])
        );
        assert!(
            first
                .to_string()
                .starts_with("quilt+s3://b#package=team/raw: added to data.toml under `sync`, in "),
            "{first}"
        );
        assert_eq!(again.to_json()["outcome"], "unchanged");
        assert!(
            again
                .to_string()
                .ends_with("already holds the revision the lock names"),
            "{again}"
        );
        assert_eq!(
            std::fs::read(dataspace.path().join("team/raw/a.txt"))?,
            b"alpha"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn an_install_under_another_access_is_an_error() -> Result<(), Error> {
        let remote = MockRemote::default();
        publish(&remote).await?;
        let dataspace = tempfile::TempDir::new()?;
        std::fs::write(
            dataspace.path().join("data.toml"),
            "registry = \"s3://b\"\n[packages]\nread = [\"team/raw\"]\n",
        )?;

        let std = Std::from_result(
            install_with(&remote, None, input(dataspace.path(), Access::Sync)).await,
        );

        let Std::Err(err) = std else {
            panic!("expected an error, got {std:?}");
        };
        assert_eq!(err.kind(), "access_declared");
        Ok(())
    }
}
