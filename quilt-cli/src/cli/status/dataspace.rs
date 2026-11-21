//! `quilt status --dataspace`: each entry of a dataspace against its lock,
//! its registry and its working folder. Writes nothing.

use std::path::Path;
use std::path::PathBuf;

use quilt_rs::dataspace::contract::CONTRACT_FILE;
use quilt_rs::dataspace::lock::LOCK_FILE;
use quilt_rs::dataspace::status::DataspaceStatus;
use quilt_rs::dataspace::status::EntryStatus;
use quilt_rs::dataspace::status::LatestSource;
use quilt_rs::dataspace::status::status;
use quilt_rs::domain::DomainRoot;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::lineage::Change;
use quilt_rs::lineage::PackageState;
use quilt_rs::lineage::UpstreamState;
use quilt_rs::lineage::WorkingCopy;
use quilt_rs::paths::DomainPaths;
use quilt_rs::queue::KeptReason;
use quilt_rs::queue::Queue;

use crate::cli::Error;
use crate::cli::dataspace::reader;
use crate::cli::output::Render;
use crate::cli::output::Std;
use crate::cli::text::printable_line;

#[derive(Debug)]
pub struct Input {
    pub dataspace: PathBuf,
}

pub struct Output {
    dataspace: PathBuf,
    status: DataspaceStatus,
}

/// What `state` says, as a status line.
pub(crate) fn state_words(state: PackageState) -> String {
    match state {
        PackageState::Latest => "up to date".to_string(),
        PackageState::PendingCommit => "local commits not pushed".to_string(),
        PackageState::PendingChanges { files: 1 } => "1 file changed".to_string(),
        PackageState::PendingChanges { files } => format!("{files} files changed"),
        PackageState::Behind => "behind latest".to_string(),
        PackageState::Diverged => "diverged from latest".to_string(),
        PackageState::Unpublished | PackageState::NoRemote => "not published yet".to_string(),
        PackageState::Unavailable => "working folder unavailable".to_string(),
        PackageState::Unknown => "latest unknown".to_string(),
    }
}

/// What `--json` calls a state.
pub(crate) fn state_kind(state: PackageState) -> &'static str {
    match state {
        PackageState::Latest => "latest",
        PackageState::PendingCommit => "pending_commit",
        PackageState::PendingChanges { .. } => "pending_changes",
        PackageState::Behind => "behind",
        PackageState::Diverged => "diverged",
        PackageState::Unpublished => "unpublished",
        PackageState::NoRemote => "no_remote",
        PackageState::Unavailable => "unavailable",
        PackageState::Unknown => "unknown",
    }
}

fn source_kind(source: LatestSource) -> &'static str {
    match source {
        LatestSource::Registry => "registry",
        LatestSource::Index => "index",
        LatestSource::Lock => "lock",
        LatestSource::Unknown => "unknown",
    }
}

pub(crate) fn kept_kind(reason: KeptReason) -> &'static str {
    match reason {
        KeptReason::LatestMoved => "latest_moved",
        KeptReason::WorkflowBlocked => "workflow_blocked",
    }
}

fn change_kind(change: &Change) -> &'static str {
    match change {
        Change::Modified(_) => "modified",
        Change::Added(_) => "added",
        Change::Removed(_) => "removed",
    }
}

fn path_text(path: &Path) -> String {
    printable_line(&path.display().to_string())
}

fn write_entry(f: &mut std::fmt::Formatter<'_>, status: &EntryStatus) -> std::fmt::Result {
    let entry = &status.entry;
    write!(
        f,
        "{} ({}) in {}",
        entry.key(),
        entry.access,
        path_text(&entry.folder)
    )?;
    if !status.installed {
        write!(f, "\n  not installed: no revision of it was read here")?;
    }
    write!(f, "\n  {}", state_words(status.state()))?;
    if status.upstream_state != UpstreamState::Error
        && status.latest_source != LatestSource::Registry
    {
        f.write_str(match status.latest_source {
            LatestSource::Index => ", against the last latest known",
            _ => ", against the revision last read or published here",
        })?;
    }
    if !status.unpushed.is_empty() {
        let plural = if status.unpushed.len() == 1 { "" } else { "s" };
        write!(
            f,
            "\n  {} local commit{plural} not pushed",
            status.unpushed.len()
        )?;
    }
    if let Some(kept) = &status.kept {
        let why = match kept.reason {
            KeptReason::LatestMoved => "latest had moved",
            KeptReason::WorkflowBlocked => "a workflow refused it",
        };
        write!(f, "\n  kept {}, not made latest: {why}", kept.hash)?;
    }
    if !status.conflicts.is_empty() {
        write!(f, "\n  conflicts in conflict.json:")?;
        for key in status.conflicts.keys() {
            write!(f, "\n    {}", printable_line(&key.to_string()))?;
        }
    }
    if let WorkingCopy::Unavailable(folder) = &status.working_copy {
        write!(
            f,
            "\n  {} is missing or empty, so no changes are listed",
            path_text(folder)
        )?;
    }
    for (path, change) in &status.changes {
        write!(f, "\n  {:<8}  {}", change_kind(change), path_text(path))?;
    }
    if status.metadata_changed {
        write!(f, "\n  metadata changed")?;
    }
    for problem in &status.problems {
        write!(
            f,
            "\n  could not read: {}",
            printable_line(&problem.to_string())
        )?;
    }
    Ok(())
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let status = &self.status;
        let mut lines = Vec::new();
        if !status.locked {
            lines.push(format!("No {LOCK_FILE}: nothing was read here yet"));
        } else if status.filled.is_none() {
            lines.push("Not filled yet: the first sync reads every entry whole".to_string());
        }
        if !status.mismatches.is_empty() {
            let what: Vec<String> = status.mismatches.iter().map(ToString::to_string).collect();
            lines.push(format!(
                "{LOCK_FILE} was written for another {}; a sync does not use it until it is fixed",
                what.join(", ")
            ));
        }
        for problem in &status.problems {
            lines.push(format!(
                "Could not read: {}",
                printable_line(&problem.to_string())
            ));
        }
        if status.entries.is_empty() {
            lines.push(format!(
                "No entries in {}",
                path_text(&self.dataspace.join(CONTRACT_FILE))
            ));
        }
        for line in &lines {
            writeln!(f, "{line}")?;
        }
        for (index, entry) in status.entries.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            write_entry(f, entry)?;
        }
        Ok(())
    }
}

fn entry_json(status: &EntryStatus) -> serde_json::Value {
    let entry = &status.entry;
    let changes: Vec<_> = status
        .changes
        .iter()
        .map(|(path, change)| {
            serde_json::json!({
                "path": path.display().to_string(),
                "status": change_kind(change),
            })
        })
        .collect();
    let conflicts: Vec<String> = status.conflicts.keys().map(ToString::to_string).collect();
    let unavailable = match &status.working_copy {
        WorkingCopy::Unavailable(folder) => Some(folder.display().to_string()),
        WorkingCopy::Available => None,
    };
    let kept = status.kept.as_ref().map(|kept| {
        serde_json::json!({
            "hash": kept.hash,
            "base": kept.base,
            "reason": kept_kind(kept.reason),
        })
    });
    let problems: Vec<String> = status.problems.iter().map(ToString::to_string).collect();
    serde_json::json!({
        "package": entry.key().to_string(),
        "access": entry.access.to_string(),
        "folder": entry.folder.display().to_string(),
        "installed": status.installed,
        "state": state_kind(status.state()),
        "upstream_state": status.upstream_state,
        "base": status.hashes.base,
        "current": status.hashes.current,
        "latest": status.hashes.latest,
        "remote": status.hashes.remote,
        "latest_source": source_kind(status.latest_source),
        "unpushed": status.unpushed,
        "changes": changes,
        "metadata_changed": status.metadata_changed,
        "working_copy_unavailable": unavailable,
        "conflicts": conflicts,
        "kept": kept,
        "problems": problems,
    })
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let status = &self.status;
        let mismatches: Vec<String> = status.mismatches.iter().map(ToString::to_string).collect();
        let problems: Vec<String> = status.problems.iter().map(ToString::to_string).collect();
        let entries: Vec<_> = status.entries.iter().map(entry_json).collect();
        serde_json::json!({
            "dataspace": self.dataspace.display().to_string(),
            "locked": status.locked,
            "filled": status.filled.map(|filled| filled.to_string()),
            "mismatches": mismatches,
            "problems": problems,
            "entries": entries,
        })
    }
}

pub async fn command(credentials: PathBuf, domain: Option<DomainRoot>, args: Input) -> Std {
    let paths = DomainPaths::new(credentials);
    let user_rules = paths.user_quiltignore();
    let remote = RemoteS3::new(paths, LocalStorage::new());
    Std::from_result(status_with(&remote, domain.as_ref(), args, &user_rules).await)
}

/// The queue of `domain`, where it has one; opening it makes none.
pub(crate) async fn existing_queue(domain: Option<&DomainRoot>) -> Option<Queue> {
    let path = domain?.queue_db();
    if !path.is_file() {
        return None;
    }
    Queue::open_read_only(&path).await.ok()
}

async fn status_with(
    remote: &impl Remote,
    domain: Option<&DomainRoot>,
    Input { dataspace }: Input,
    user_rules: &Path,
) -> Result<Output, Error> {
    let writer = reader(domain).await?;
    let queue = existing_queue(domain).await;
    let status = Box::pin(status(
        &LocalStorage::new(),
        remote,
        domain,
        queue.as_ref(),
        &dataspace,
        &writer,
        user_rules,
        true,
    ))
    .await?;
    Ok(Output { dataspace, status })
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    use quilt_rs::io::remote::mocks::MockRemote;
    use quilt_rs::manifest::Manifest;
    use quilt_rs::manifest::ManifestRow;
    use quilt_uri::S3Uri;
    use test_log::test;

    pub(crate) const FIRST: &str =
        "1111111111111111111111111111111111111111111111111111111111111111";

    /// Stores `files` as revision `hash` of `team/raw` in bucket `b`, lists
    /// it at `second`, and tags it `latest`.
    pub(crate) async fn seed(
        remote: &MockRemote,
        hash: &str,
        files: &[(&str, &[u8])],
        second: i64,
    ) -> Result<(), Error> {
        let mut rows = Vec::new();
        for (key, body) in files {
            let physical_key = format!("s3://b/team/raw/{key}?versionId={hash}");
            remote
                .put_object(
                    None,
                    &S3Uri::try_from(physical_key.as_str())?,
                    body.to_vec(),
                )
                .await?;
            rows.push(ManifestRow {
                logical_key: quilt_rs::LogicalKey::new(*key)?,
                physical_key,
                hash: quilt_rs::checksum::hash_reader(*body, body.len() as u64, 0x12).await?,
                size: body.len() as u64,
                ..ManifestRow::default()
            });
        }
        let mut manifest = Manifest {
            rows,
            ..Manifest::default()
        };
        manifest.header.message = quilt_rs::manifest::HeaderMessage::Text(format!("r{second}"));
        let put = |uri: String, body: Vec<u8>| async move {
            remote
                .put_object(None, &S3Uri::try_from(uri.as_str())?, body)
                .await
        };
        put(
            format!("s3://b/.quilt/packages/{hash}"),
            manifest.to_jsonlines().into_bytes(),
        )
        .await?;
        put(
            format!("s3://b/.quilt/named_packages/team/raw/{second}"),
            hash.as_bytes().to_vec(),
        )
        .await?;
        put(
            "s3://b/.quilt/named_packages/team/raw/latest".to_string(),
            hash.as_bytes().to_vec(),
        )
        .await?;
        Ok(())
    }

    /// A dataspace syncing `team/raw` from bucket `b`, filled at `FIRST`
    /// holding `a.txt`, with no Domain.
    pub(crate) async fn filled(remote: &MockRemote) -> Result<tempfile::TempDir, Error> {
        let dataspace = tempfile::TempDir::new()?;
        std::fs::write(
            dataspace.path().join(CONTRACT_FILE),
            "registry = \"s3://b\"\n\n[packages]\nread = [\"team/raw\"]\n",
        )?;
        seed(remote, FIRST, &[("a.txt", b"v1")], 1_700_000_000).await?;
        let writer = reader(None).await?;
        quilt_rs::dataspace::read_all::read_all(
            &LocalStorage::new(),
            remote,
            None,
            dataspace.path(),
            &writer,
        )
        .await
        .map_err(|err| Error::Test(err.to_string()))?;
        Ok(dataspace)
    }

    #[test(tokio::test)]
    async fn a_dataspace_status_shows_each_entry_and_its_changes() -> Result<(), Error> {
        let remote = MockRemote::default();
        let dataspace = filled(&remote).await?;
        std::fs::write(dataspace.path().join("team/raw/a.txt"), "mine")?;
        let input = Input {
            dataspace: dataspace.path().to_path_buf(),
        };
        let no_rules = dataspace.path().join("no-user-rules");

        let output = status_with(&remote, None, input, &no_rules).await?;

        let text = output.to_string();
        assert!(
            text.starts_with("quilt+s3://b#package=team/raw (read) in "),
            "{text}"
        );
        assert!(text.contains("\n  1 file changed"), "{text}");
        assert!(text.contains("\n  modified  a.txt"), "{text}");
        let json = output.to_json();
        let entry = &json["entries"][0];
        assert_eq!(entry["state"], "pending_changes");
        assert_eq!(entry["upstream_state"], "up_to_date");
        assert_eq!(entry["base"], FIRST);
        assert_eq!(entry["changes"][0]["status"], "modified");
        assert_eq!(json["locked"], true);
        Ok(())
    }
}
