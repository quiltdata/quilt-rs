//! `quilt sync`: one tick of a dataspace's `sync` entries, as autosync runs
//! it. It reads each entry's `latest`, commits what its folder changed and
//! pushes the lock's commits; after a `data.toml` edit it only reads, which
//! is what lets `quilt commit` run again.

use std::path::Path;
use std::path::PathBuf;

use quilt_rs::dataspace::contract::CONTRACT_FILE;
use quilt_rs::dataspace::publish::Published;
use quilt_rs::dataspace::read::ReadReport;
use quilt_rs::dataspace::sync::DataspaceSync;
use quilt_rs::dataspace::sync::EntrySync;
use quilt_rs::dataspace::sync::sync;
use quilt_rs::domain::DomainRoot;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::paths::DomainPaths;
use quilt_rs::queue::Queue;

use crate::cli::Error;
use crate::cli::dataspace;
use crate::cli::output::Render;
use crate::cli::output::Std;
use crate::cli::text::printable_line;

/// The message of each commit the tick makes, where `--message` gives none.
pub const DEFAULT_MESSAGE: &str = "quilt sync";

#[derive(Debug)]
pub struct Input {
    /// The dataspace: the folder holding `data.toml`.
    pub dataspace: PathBuf,
    pub message: String,
}

pub struct Output {
    dataspace: PathBuf,
    synced: DataspaceSync,
}

/// One word for how an entry's tick ended, as `--json` gives it.
fn outcome_word(outcome: &EntrySync) -> &'static str {
    match outcome {
        EntrySync::Ran { read, published } => {
            let read_whole = read.as_ref().is_none_or(ReadReport::complete);
            let pushed = matches!(
                published,
                None | Some(Published::Advanced { .. } | Published::Unchanged { .. })
            );
            if read_whole && pushed {
                "synced"
            } else {
                "incomplete"
            }
        }
        EntrySync::Blocked { .. } => "conflict",
        EntrySync::Diverged { .. } => "diverged",
        EntrySync::ReadBack(_) => "read_back",
        EntrySync::Busy => "busy",
        EntrySync::Failed(_) => "failed",
        _ => "other",
    }
}

/// What an entry's tick did, in words, after the package's name.
fn outcome_text(outcome: &EntrySync) -> String {
    match outcome {
        EntrySync::Ran { read, published } => {
            let mut said = Vec::new();
            if let Some(read) = read {
                said.push(format!("read {}", read.revision));
            }
            match published {
                Some(Published::Advanced { revision, .. }) => {
                    said.push(format!("published {revision}"));
                }
                Some(Published::Stored { revision, .. }) => {
                    said.push(format!("stored {revision}, but latest names another"));
                }
                Some(Published::WorkflowBlocked { revision, reason }) => {
                    said.push(format!(
                        "stored {revision}; the workflow refused it: {reason}"
                    ));
                }
                Some(Published::Behind { .. }) => {
                    said.push("latest moved past the lock's base".to_string());
                }
                Some(Published::Unchanged { .. }) | None => {}
                Some(_) => said.push("published nothing".to_string()),
            }
            if said.is_empty() {
                "up to date".to_string()
            } else {
                said.join("; ")
            }
        }
        EntrySync::Blocked { revision, .. } => format!(
            "local changes collide with {revision}; decide them in conflict.json, then sync"
        ),
        EntrySync::Diverged { .. } => "latest diverged from the lock; nothing done".to_string(),
        EntrySync::ReadBack(report) => {
            format!("the folder was empty; read {} back", report.revision)
        }
        EntrySync::Busy => "another process holds it; left for the next sync".to_string(),
        EntrySync::Failed(err) => format!("failed: {err}"),
        _ => "not synced".to_string(),
    }
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.synced.entries.is_empty() {
            return write!(
                f,
                "No `sync` entries in {}",
                printable_line(&self.dataspace.join(CONTRACT_FILE).display().to_string())
            );
        }
        for (index, (entry, outcome)) in self.synced.entries.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            write!(f, "{}: {}", entry.key(), outcome_text(outcome))?;
        }
        if self.synced.read_only {
            write!(
                f,
                "\nOnly read: the dataspace is not filled yet, or data.toml was edited"
            )?;
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let entries: Vec<serde_json::Value> = self
            .synced
            .entries
            .iter()
            .map(|(entry, outcome)| {
                serde_json::json!({
                    "package": entry.key().to_string(),
                    "folder": entry.folder.display().to_string(),
                    "outcome": outcome_word(outcome),
                    "message": outcome_text(outcome),
                })
            })
            .collect();
        serde_json::json!({
            "dataspace": self.dataspace.display().to_string(),
            "read_only": self.synced.read_only,
            "completed": self.completed(),
            "entries": entries,
        })
    }

    /// Whether every entry ran to the end: read whole where it read, and
    /// pushed where it published.
    fn completed(&self) -> bool {
        self.synced
            .entries
            .iter()
            .all(|(_, outcome)| outcome_word(outcome) == "synced")
    }
}

/// Syncs with `domain`, which keeps the commits and the queue, and the
/// credentials and user-wide ignore rules the `credentials` folder keeps.
pub async fn command(credentials: PathBuf, domain: DomainRoot, args: Input) -> Std {
    let paths = DomainPaths::new(credentials);
    let user_rules = paths.user_quiltignore();
    let remote = RemoteS3::new(paths, LocalStorage::new());
    Std::from_result(Box::pin(sync_with(&remote, &domain, args, &user_rules)).await)
}

pub(super) async fn sync_with(
    remote: &impl Remote,
    domain: &DomainRoot,
    Input { dataspace, message }: Input,
    user_rules: &Path,
) -> Result<Output, Error> {
    let storage = LocalStorage::new();
    let writer = dataspace::writer(Some(&domain.local())).await?;
    std::fs::create_dir_all(domain.root())?;
    let queue = Queue::open(&domain.queue_db()).await?;
    let synced = Box::pin(sync(
        &storage, remote, domain, &queue, &dataspace, &writer, &message, user_rules,
    ))
    .await?;
    Ok(Output { dataspace, synced })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use quilt_rs::dataspace::commit::CommitError;
    use quilt_rs::dataspace::commit::commit;
    use quilt_rs::dataspace::install::Install;
    use quilt_rs::dataspace::install::install;
    use quilt_rs::io::remote::mocks::MockRemote;
    use quilt_rs::manifest::Manifest;
    use quilt_rs::manifest::ManifestRow;
    use quilt_uri::S3Uri;

    const FIRST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const KEY: &str = "quilt+s3://b#package=team/raw";

    /// Stores `a.txt` holding `alpha` as revision [`FIRST`] of `team/raw` in
    /// bucket `b`, and tags it `latest`.
    async fn publish(remote: &MockRemote) -> Result<(), Error> {
        let body = b"alpha";
        let physical_key = format!("s3://b/team/raw/a.txt?versionId={FIRST}");
        let uri = |text: &str| S3Uri::try_from(text);
        remote
            .put_object(None, &uri(&physical_key)?, body.to_vec())
            .await?;
        let manifest = Manifest {
            rows: vec![ManifestRow {
                logical_key: quilt_rs::LogicalKey::new("a.txt")?,
                physical_key: physical_key.clone(),
                hash: quilt_rs::checksum::hash_reader(&body[..], body.len() as u64, 0x12).await?,
                size: body.len() as u64,
                ..ManifestRow::default()
            }],
            ..Manifest::default()
        };
        let packages = format!("s3://b/.quilt/packages/{FIRST}");
        remote
            .put_object(None, &uri(&packages)?, manifest.to_jsonlines().into_bytes())
            .await?;
        let latest = "s3://b/.quilt/named_packages/team/raw/latest";
        remote
            .put_object(None, &uri(latest)?, FIRST.as_bytes().to_vec())
            .await?;
        Ok(())
    }

    /// The refusal 25s left: after `data.toml` is edited, a commit waits for
    /// a sync to read the dataspace again, which `quilt sync` now runs.
    #[test(tokio::test)]
    async fn a_sync_clears_the_commit_refused_after_data_toml_is_edited() -> Result<(), Error> {
        let remote = MockRemote::default();
        publish(&remote).await?;
        let dir = tempfile::TempDir::new()?;
        let space = dir.path().join("space");
        std::fs::create_dir_all(&space)?;
        std::fs::write(space.join("data.toml"), "registry = \"s3://b\"\n")?;
        let domain = DomainRoot::new(dir.path().join("domain"));
        let rules = dir.path().join("no-rules");
        let storage = LocalStorage::new();
        let writer = dataspace::writer(Some(&domain.local())).await?;
        install(&storage, &remote, None, &space, &writer, &Install::new(KEY)).await?;
        let contract = std::fs::read_to_string(space.join("data.toml"))?;
        std::fs::write(space.join("data.toml"), format!("# edited\n{contract}"))?;
        std::fs::write(space.join("team/raw/a.txt"), b"beta")?;
        let key: quilt_rs::dataspace::contract::PackageKey = KEY.parse()?;
        let committing = || {
            Box::pin(commit(
                &storage, &remote, &domain, &space, &writer, &key, "edit", &rules,
            ))
        };
        let refused = committing().await;
        assert!(matches!(refused, Err(CommitError::Edited)), "{refused:?}");

        let input = Input {
            dataspace: space.clone(),
            message: DEFAULT_MESSAGE.to_string(),
        };
        let synced = sync_with(&remote, &domain, input, &rules).await?;

        assert!(synced.synced.read_only, "an edit is read first");
        assert_eq!(synced.to_json()["entries"][0]["package"], KEY);
        assert!(committing().await?.commit.is_some());
        Ok(())
    }
}
