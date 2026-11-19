use std::path::PathBuf;

use quilt_rs::dataspace::contract::CONTRACT_FILE;
use quilt_rs::dataspace::lock::Writer;
use quilt_rs::dataspace::read::ReadOutcome;
use quilt_rs::dataspace::read_all::DataspaceRead;
use quilt_rs::dataspace::read_all::EntryRead;
use quilt_rs::dataspace::read_all::ReadAllError;
use quilt_rs::dataspace::read_all::read_all;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::paths::DomainPaths;

use crate::cli::Error;
use crate::cli::output::Render;
use crate::cli::output::Std;
use crate::cli::text::printable;
use crate::cli::text::printable_line;

#[derive(Debug)]
pub struct Input {
    pub dataspace: PathBuf,
}

pub struct Output {
    dataspace: PathBuf,
    read: DataspaceRead,
}

/// The JSON keys of the lists [`lists`] gives, each present on every entry.
const LIST_KEYS: [&str; 6] = [
    "placed",
    "removed",
    "kept",
    "skipped",
    "hash_mismatch",
    "conflicts",
];

/// The lists of files an entry's read reports, each with its JSON key and
/// its heading in the text.
fn lists(read: &EntryRead) -> Vec<(&'static str, &'static str, &[PathBuf])> {
    match read {
        EntryRead::Ran(ReadOutcome::Read(report)) => vec![
            ("placed", "placed", &report.placed),
            ("removed", "removed", &report.removed),
            ("kept", "changed here and kept", &report.kept),
            (
                "skipped",
                "not placed: the bucket no longer holds them",
                &report.skipped,
            ),
            (
                "hash_mismatch",
                "not placed: they do not match their hash",
                &report.hash_mismatch,
            ),
        ],
        EntryRead::Ran(ReadOutcome::Blocked { conflicts, .. }) => {
            vec![("conflicts", "in both", conflicts)]
        }
        _ => Vec::new(),
    }
}

/// The revision an entry's read was of, where it got that far.
fn revision(read: &EntryRead) -> Option<&str> {
    match read {
        EntryRead::Ran(ReadOutcome::Read(report)) => Some(&report.revision),
        EntryRead::Ran(ReadOutcome::Blocked { revision, .. }) => Some(revision),
        _ => None,
    }
}

/// A heading, then its paths one per line, under one package's line.
fn group(f: &mut std::fmt::Formatter<'_>, heading: &str, paths: &[PathBuf]) -> std::fmt::Result {
    if paths.is_empty() {
        return Ok(());
    }
    let plural = if paths.len() == 1 { "" } else { "s" };
    write!(f, "\n  {} file{plural} {heading}:", paths.len())?;
    for path in paths {
        write!(f, "\n    {}", printable_line(&path.display().to_string()))?;
    }
    Ok(())
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.read.entries.is_empty() {
            return write!(
                f,
                "No `read` entries in {}",
                printable_line(&self.dataspace.join(CONTRACT_FILE).display().to_string())
            );
        }
        for (index, (entry, read)) in self.read.entries.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            let package = entry.key();
            match read {
                EntryRead::Ran(ReadOutcome::Read(report)) if read.completed() => write!(
                    f,
                    "{package}: read {} into {}",
                    report.revision,
                    printable_line(&entry.folder.display().to_string())
                )?,
                EntryRead::Ran(ReadOutcome::Read(report)) => write!(
                    f,
                    "{package}: {} cannot be read whole, so nothing changed",
                    report.revision
                )?,
                EntryRead::Ran(ReadOutcome::Blocked { revision, .. }) => write!(
                    f,
                    "{package}: blocked, so nothing changed; local changes collide with {revision}"
                )?,
                EntryRead::Unpushed => write!(
                    f,
                    "{package}: not read; the lock names local commits not yet pushed"
                )?,
                EntryRead::Failed(err) => {
                    write!(f, "{package}: failed: {}", printable(&err.to_string()))?;
                }
                _ => write!(f, "{package}: not read")?,
            }
            for (_, heading, paths) in lists(read) {
                group(f, heading, paths)?;
            }
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let paths = |list: &[PathBuf]| {
            list.iter()
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
        };
        let entries: Vec<serde_json::Value> = self
            .read
            .entries
            .iter()
            .map(|(entry, read)| {
                let (outcome, error) = match read {
                    EntryRead::Ran(ReadOutcome::Read(_)) if read.completed() => ("read", None),
                    EntryRead::Ran(ReadOutcome::Read(_)) => ("incomplete", None),
                    EntryRead::Ran(ReadOutcome::Blocked { .. }) => ("blocked", None),
                    EntryRead::Unpushed => ("unpushed", None),
                    EntryRead::Failed(err) => ("failed", Some(err.to_string())),
                    _ => ("not_read", None),
                };
                let mut json = serde_json::json!({
                    "package": entry.key().to_string(),
                    "folder": entry.folder.display().to_string(),
                    "outcome": outcome,
                    "completed": read.completed(),
                    "revision": revision(read),
                    "error": error,
                });
                for key in LIST_KEYS {
                    json[key] = serde_json::json!([]);
                }
                for (key, _, list) in lists(read) {
                    json[key] = paths(list).into();
                }
                json
            })
            .collect();
        serde_json::json!({
            "dataspace": self.dataspace.display().to_string(),
            "completed": self.read.completed(),
            "entries": entries,
        })
    }

    fn completed(&self) -> bool {
        self.read.completed()
    }
}

pub async fn command(domain: PathBuf, args: Input) -> Std {
    Std::from_result(model(domain, args).await)
}

/// Reads with no Domain: the domain folder supplies only the credentials a
/// catalog's login keeps there.
pub async fn model(domain: PathBuf, args: Input) -> Result<Output, Error> {
    let remote = RemoteS3::new(DomainPaths::new(domain), LocalStorage::new());
    read_with(&remote, args).await
}

async fn read_with(remote: &impl Remote, Input { dataspace }: Input) -> Result<Output, Error> {
    let writer = Writer::without_domain().map_err(ReadAllError::from)?;
    let read = read_all(&LocalStorage::new(), remote, &dataspace, &writer).await?;
    Ok(Output { dataspace, read })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;

    use test_log::test;

    use quilt_rs::io::remote::mocks::MockRemote;
    use quilt_rs::manifest::Manifest;
    use quilt_rs::manifest::ManifestRow;
    use quilt_uri::S3Uri;

    const FIRST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const SECOND: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    /// Stores `files` as revision `hash` of `team/raw` in bucket `b`, and
    /// tags it `latest`.
    async fn publish(
        remote: &MockRemote,
        hash: &str,
        files: &[(&str, &[u8])],
    ) -> Result<(), Error> {
        publish_bytes(remote, hash, files, false).await
    }

    /// [`publish`]; where `replaced`, each file sits under an unversioned key
    /// whose bytes have since changed.
    async fn publish_bytes(
        remote: &MockRemote,
        hash: &str,
        files: &[(&str, &[u8])],
        replaced: bool,
    ) -> Result<(), Error> {
        let mut rows = Vec::new();
        for (key, body) in files {
            let (physical_key, stored) = if replaced {
                (format!("s3://b/team/raw/{key}"), b"newer".to_vec())
            } else {
                let versioned = format!("s3://b/team/raw/{key}?versionId={hash}");
                (versioned, body.to_vec())
            };
            remote
                .put_object(None, &S3Uri::try_from(physical_key.as_str())?, stored)
                .await?;
            rows.push(ManifestRow {
                logical_key: quilt_rs::LogicalKey::new(*key)?,
                physical_key,
                hash: quilt_rs::checksum::hash_reader(*body, body.len() as u64, 0x12).await?,
                size: body.len() as u64,
                ..ManifestRow::default()
            });
        }
        let manifest = Manifest {
            rows,
            ..Manifest::default()
        };
        let manifest_uri = format!("s3://b/.quilt/packages/{hash}");
        remote
            .put_object(
                None,
                &S3Uri::try_from(manifest_uri.as_str())?,
                manifest.to_jsonlines().into_bytes(),
            )
            .await?;
        remote
            .put_object(
                None,
                &S3Uri::try_from("s3://b/.quilt/named_packages/team/raw/latest")?,
                hash.as_bytes().to_vec(),
            )
            .await?;
        Ok(())
    }

    fn dataspace() -> Result<tempfile::TempDir, Error> {
        let dataspace = tempfile::TempDir::new()?;
        std::fs::write(
            dataspace.path().join("data.toml"),
            "registry = \"s3://b\"\n\n[packages]\nread = [\"team/raw\"]\n",
        )?;
        Ok(dataspace)
    }

    async fn read(remote: &MockRemote, dataspace: &Path) -> Std {
        Std::from_result(
            read_with(
                remote,
                Input {
                    dataspace: dataspace.to_path_buf(),
                },
            )
            .await,
        )
    }

    #[test(tokio::test)]
    async fn a_dataspace_read_whole_reports_each_package_and_succeeds() -> Result<(), Error> {
        let remote = MockRemote::default();
        publish(&remote, FIRST, &[("a.txt", b"alpha")]).await?;
        let dataspace = dataspace()?;

        let std = read(&remote, dataspace.path()).await;

        assert!(!std.failed(), "{std:?}");
        let Std::Out(output) = std else {
            panic!("expected a success, got {std:?}");
        };
        let json = output.to_json();
        assert_eq!(json["completed"], true);
        let entry = &json["entries"][0];
        assert_eq!(entry["package"], "quilt+s3://b#package=team/raw");
        assert_eq!(entry["outcome"], "read");
        assert_eq!(entry["revision"], FIRST);
        assert_eq!(
            entry["placed"],
            serde_json::json!(["a.txt", "package-metadata.json"])
        );
        assert!(output.to_string().starts_with(&format!(
            "quilt+s3://b#package=team/raw: read {FIRST} into "
        )));
        assert!(dataspace.path().join("data.lock.toml").exists());
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_package_blocked_by_local_changes_is_reported_and_fails() -> Result<(), Error> {
        let remote = MockRemote::default();
        publish(&remote, FIRST, &[("a.txt", b"alpha")]).await?;
        let dataspace = dataspace()?;
        read(&remote, dataspace.path()).await;
        std::fs::write(dataspace.path().join("team/raw/a.txt"), b"edited")?;
        publish(&remote, SECOND, &[("a.txt", b"ALPHA")]).await?;

        let std = read(&remote, dataspace.path()).await;

        assert!(std.failed());
        let Std::Out(output) = std else {
            panic!("expected the report, got {std:?}");
        };
        let json = output.to_json();
        assert_eq!(json["completed"], false);
        assert_eq!(json["entries"][0]["outcome"], "blocked");
        assert_eq!(
            json["entries"][0]["conflicts"],
            serde_json::json!(["a.txt"])
        );
        assert_eq!(
            output.to_string(),
            format!(
                "quilt+s3://b#package=team/raw: blocked, so nothing changed; local changes collide with {SECOND}\n  1 file in both:\n    a.txt"
            )
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_revision_whose_bytes_are_gone_is_reported_as_changing_nothing() -> Result<(), Error>
    {
        let remote = MockRemote::default();
        publish_bytes(&remote, FIRST, &[("a.txt", b"alpha")], true).await?;
        let dataspace = dataspace()?;

        let std = read(&remote, dataspace.path()).await;

        assert!(std.failed());
        let Std::Out(output) = std else {
            panic!("expected the report, got {std:?}");
        };
        assert_eq!(output.to_json()["entries"][0]["outcome"], "incomplete");
        assert_eq!(
            output.to_string(),
            format!(
                "quilt+s3://b#package=team/raw: {FIRST} cannot be read whole, so nothing changed\n  1 file not placed: the bucket no longer holds them:\n    a.txt"
            )
        );
        assert!(!dataspace.path().join("team/raw/a.txt").exists());
        Ok(())
    }

    #[test(tokio::test)]
    async fn live_read_fills_a_dataspace_from_a_bucket() -> Result<(), Error> {
        use crate::cli::fixtures::packages::default as pkg;
        let domain = tempfile::TempDir::new()?;
        let dataspace = tempfile::TempDir::new()?;
        std::fs::write(
            dataspace.path().join("data.toml"),
            format!(
                "registry = \"s3://{}\"\n\n[packages]\nread = [{{ package = \"{}\", revision = \"{}\", path = \"{}\" }}]\n",
                pkg::BUCKET,
                pkg::NAMESPACE_STR,
                pkg::TOP_HASH,
                pkg::TIMESTAMP_LK,
            ),
        )?;
        let input = || Input {
            dataspace: dataspace.path().to_path_buf(),
        };

        let first = Std::from_result(model(domain.path().to_path_buf(), input()).await);
        let again = Std::from_result(model(domain.path().to_path_buf(), input()).await);

        let Std::Out(first) = first else {
            panic!("expected a success, got {first:?}");
        };
        let json = first.to_json();
        assert_eq!(json["entries"][0]["revision"], pkg::TOP_HASH);
        assert!(
            dataspace
                .path()
                .join(pkg::NAMESPACE_STR)
                .join(pkg::TIMESTAMP_LK)
                .is_file()
        );
        let lock = std::fs::read_to_string(dataspace.path().join("data.lock.toml"))?;
        assert!(lock.contains(pkg::TOP_HASH), "{lock}");
        let Std::Out(again) = again else {
            panic!("expected a success, got {again:?}");
        };
        assert_eq!(
            again.to_json()["entries"][0]["placed"],
            serde_json::json!([])
        );
        assert!(
            !domain.path().join(".quilt").exists(),
            "nothing is kept in the domain folder"
        );
        Ok(())
    }
}
