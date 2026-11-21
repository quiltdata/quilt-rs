//! The reads the package page, its file pane, the pull preview and the
//! commit page make, and the backlog download, on the dataspace engine.
//!
//! Each answers what its ref-path command answers, read from the entry's
//! status, the manifests of the revisions the lock names, and the folder.
//! The dataspace has no tracked set and no stored scope (25.16): an entry's
//! paths are its scope, and every file they cover is read (25.7 rule 4), so
//! a listed file is here, deleted here, or outside the paths.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use quilt_rs::LogicalKey;
use quilt_rs::dataspace::contract::Access;
use quilt_rs::dataspace::contract::CONTRACT_FILE;
use quilt_rs::dataspace::contract::Entry;
use quilt_rs::dataspace::contract::PackagePath;
use quilt_rs::dataspace::contract::Paths;
use quilt_rs::dataspace::contract::Registry;
use quilt_rs::dataspace::keep::Keep;
use quilt_rs::dataspace::keep::KeepRead;
use quilt_rs::dataspace::keep::Scope;
use quilt_rs::dataspace::read::ReadOutcome;
use quilt_rs::dataspace::status::EntryStatus;
use quilt_rs::dataspace::status::LatestSource;
use quilt_rs::io::remote::Remote;
use quilt_rs::lineage::Change;
use quilt_rs::lineage::UpstreamState;
use quilt_rs::manifest::Manifest;
use quilt_rs::quiltignore::IgnoredBy;
use quilt_rs::quiltignore::Rules;
use quilt_uri::Namespace;
use quilt_uri::RevisionPointer;

use crate::engine::commands::CommitData;
use crate::engine::commands::CommitWorkflowData;
use crate::engine::commands::CommitWorkflows;
use crate::engine::commands::CurrentRevisionData;
use crate::engine::commands::FilesData;
use crate::engine::commands::KeepingData;
use crate::engine::commands::KeepingScope;
use crate::engine::commands::LatestLoss;
use crate::engine::commands::NotDownloaded;
use crate::engine::commands::PackageContextData;
use crate::engine::commands::PackageHeaderData;
use crate::engine::commands::PackagePageData;
use crate::engine::commands::PackageSize;
use crate::engine::commands::PackageStateDto;
use crate::engine::commands::ResolveData;
use crate::engine::commands::Unreachable;
use crate::engine::commands::commit_data::catalog_object_url;
use crate::engine::commands::commit_data::changed_bytes;
use crate::engine::commands::commit_data::junk_summary;
use crate::engine::commands::package_entries::EntryList;
use crate::engine::commands::package_entries::IgnoredByData;
use crate::engine::commands::package_entries::InstalledPackageEntryData;
use crate::engine::commands::package_entries::RuleSourceData;
use crate::engine::commit_message;
use crate::engine::dataspace::Dataspace;
use crate::engine::dataspace::fail;
use crate::engine::error::Error;
use crate::engine::publish_settings::PublishSettings;
use crate::engine::quilt;

use super::namespace;

/// The resolve pane's refusal when the read did not reach the bucket, as
/// the ref path words it.
const UNREACHED: &str = "The bucket couldn't be reached to compare the two revisions.";

/// One read of a package: its status, and the manifest of the revision the
/// lock names as current, empty where it names none.
pub(super) struct PackageRead {
    pub status: EntryStatus,
    pub current: Manifest,
}

impl PackageRead {
    pub(super) async fn of<R: Remote + Sync>(
        ds: &Dataspace<R>,
        namespace: &Namespace,
        fetch: bool,
    ) -> Result<Self, Error> {
        let status = ds.entry_status(namespace, fetch).await?;
        let current = match &status.hashes.current {
            Some(hash) => ds.revision_manifest(&status.entry, hash).await?,
            None => Manifest::default(),
        };
        Ok(Self { status, current })
    }

    fn entry(&self) -> &Entry {
        &self.status.entry
    }

    /// Whether the read asked the registry for `latest` and was answered.
    fn reached(&self) -> bool {
        self.status.latest_source == LatestSource::Registry
    }

    /// The registry's `latest`, when the read asked it just now.
    fn newer(&self) -> Option<String> {
        self.status.hashes.latest.clone().filter(|_| self.reached())
    }

    /// The bucket's catalog host, where the bucket did not answer.
    fn unreachable(&self) -> Option<Unreachable> {
        let bucket = matches!(self.entry().registry, Registry::S3(_));
        (bucket && !self.reached()).then(|| Unreachable {
            host: self.entry().catalog.as_ref().map(ToString::to_string),
        })
    }

    fn uri(&self) -> Option<quilt_uri::S3PackageUri> {
        let Registry::S3(bucket) = &self.entry().registry else {
            return None;
        };
        Some(quilt_uri::S3PackageUri {
            catalog: self.entry().catalog.clone(),
            bucket: bucket.clone(),
            namespace: self.entry().package.clone(),
            revision: self
                .status
                .hashes
                .base
                .clone()
                .map_or_else(|| self.entry().revision.clone(), RevisionPointer::Hash),
            path: None,
        })
    }

    /// Whether a file of the current revision is in the folder.
    fn here(&self, key: &LogicalKey) -> bool {
        self.entry().folder.join(key.as_path()).is_file()
    }
}

/// The page's state: a conflict in `conflict.json` first, then a bucket that
/// did not answer, then what the status resolves to.
fn page_state(read: &PackageRead) -> PackageStateDto {
    if !read.status.conflicts.is_empty() {
        return PackageStateDto::PullConflict {
            files: read
                .status
                .conflicts
                .keys()
                .map(|key| key.as_path().display().to_string())
                .collect(),
        };
    }
    if let Some(Unreachable { host }) = read.unreachable() {
        return PackageStateDto::Unreachable { host };
    }
    PackageStateDto::from(read.status.state())
}

fn entry_row(
    namespace: &Namespace,
    filename: String,
    size: u64,
    status: &str,
) -> InstalledPackageEntryData {
    InstalledPackageEntryData {
        filename,
        size,
        status: status.to_string(),
        junky_pattern: None,
        ignored_by: None,
        folder: false,
        namespace: namespace.clone(),
    }
}

/// The files a folder's rules leave out, with their sizes, and the folders.
type Ignored = (Vec<(PathBuf, IgnoredBy, u64)>, Vec<(PathBuf, IgnoredBy)>);

/// What the folder's rules leave out of it, files and folders, walked as
/// the tick's commit walks it; a folder holding a `data.toml` of its own is
/// not walked, since status reports it as left out.
fn ignored_in(folder: &Path, rules: &Rules, tracked: &BTreeSet<PathBuf>) -> Ignored {
    let tracked_dir = |dir: &Path| tracked.iter().any(|file| file.starts_with(dir));
    let (mut files, mut dirs) = (Vec::new(), Vec::new());
    let mut queue = vec![PathBuf::new()];
    while let Some(prefix) = queue.pop() {
        let Ok(listed) = std::fs::read_dir(folder.join(&prefix)) else {
            continue;
        };
        for item in listed.flatten() {
            let key = prefix.join(item.file_name());
            let Ok(kind) = item.file_type() else {
                continue;
            };
            if kind.is_dir() {
                if let Some(by) = rules.dir_ignored_by(&key, tracked_dir(&key)) {
                    dirs.push((key, by));
                } else if !item.path().join(CONTRACT_FILE).is_file() {
                    queue.push(key);
                }
            } else if kind.is_file()
                && let Some(by) = rules.file_ignored_by(&key, tracked.contains(&key))
            {
                let size = item.metadata().map_or(0, |meta| meta.len());
                files.push((key, by, size));
            }
        }
    }
    (files, dirs)
}

/// Every entry of the file pane: the folder's changes, the current
/// revision's other files, here or not, what the rules leave out, and the
/// nested dataspaces status left out (25o item 10).
async fn entries<R: Remote + Sync>(
    ds: &Dataspace<R>,
    read: &PackageRead,
) -> Result<Vec<InstalledPackageEntryData>, Error> {
    let namespace = &read.entry().package;
    let status = &read.status;
    let mut entries = Vec::new();
    for (path, change) in &status.changes {
        let (word, size) = match change {
            Change::Added(row) => ("added", row.size),
            Change::Modified(row) => ("modified", row.size),
            Change::Removed(row) => ("deleted", row.size),
        };
        entries.push(InstalledPackageEntryData {
            junky_pattern: quilt::junk::check(path).map(|found| found.pattern),
            ..entry_row(namespace, path.display().to_string(), size, word)
        });
    }
    for row in &read.current.rows {
        let path = row.logical_key.as_path();
        if status.changes.contains_key(path) {
            continue;
        }
        let word = if read.entry().paths.covers(&row.logical_key) && read.here(&row.logical_key) {
            "pristine"
        } else {
            "remote"
        };
        entries.push(entry_row(
            namespace,
            path.display().to_string(),
            row.size,
            word,
        ));
    }
    let folder = read.entry().folder.clone();
    let rules = ds.ignore_rules(&folder).await?;
    let tracked: BTreeSet<PathBuf> = read
        .current
        .rows
        .iter()
        .map(|row| row.logical_key.as_path().to_path_buf())
        .collect();
    let (files, dirs) = tokio::task::spawn_blocking(move || ignored_in(&folder, &rules, &tracked))
        .await
        .map_err(fail)?;
    for (path, by, size) in files {
        entries.push(InstalledPackageEntryData {
            ignored_by: Some((&by).into()),
            ..entry_row(namespace, path.display().to_string(), size, "pristine")
        });
    }
    let nested = status.left_out.iter().map(|left| {
        (
            left.path.clone(),
            IgnoredByData {
                pattern: CONTRACT_FILE.to_string(),
                source: RuleSourceData::NestedDataspace,
            },
        )
    });
    let dirs = dirs
        .into_iter()
        .map(|(path, by)| (path, IgnoredByData::from(&by)));
    for (path, by) in dirs.chain(nested) {
        entries.push(InstalledPackageEntryData {
            ignored_by: Some(by),
            folder: true,
            ..entry_row(namespace, path.display().to_string(), 0, "pristine")
        });
    }
    Ok(entries)
}

/// The Keeping section. The entry's paths are its scope: the whole package,
/// its folders, or else its files. A listed file not here is outstanding
/// unless it is a deletion, and none is a backlog, since every file the
/// paths cover is read; Download keeps the files it names instead.
fn keeping(read: &PackageRead) -> (KeepingData, Option<PackageSize>) {
    let (scope, folders) = match &read.entry().paths {
        Paths::Whole => (KeepingScope::EntirePackage, Vec::new()),
        Paths::Only(paths) => {
            let folders: Option<Vec<String>> = paths
                .iter()
                .map(|path| match path {
                    PackagePath::Folder(key) => Some(key.as_path().display().to_string()),
                    PackagePath::File(_) => None,
                })
                .collect();
            match folders {
                Some(folders) => (KeepingScope::Folders, folders),
                None => (KeepingScope::IndividualFiles, Vec::new()),
            }
        }
    };
    let deleted_here = read
        .status
        .changes
        .values()
        .filter(|change| matches!(change, Change::Removed(_)))
        .count();
    let (mut total, mut outstanding_bytes, mut outstanding) = (Some(0u64), Some(0u64), 0);
    for row in &read.current.rows {
        total = total.and_then(|sum| sum.checked_add(row.size));
        let deleted = matches!(
            read.status.changes.get(row.logical_key.as_path()),
            Some(Change::Removed(_))
        );
        if !deleted && !read.here(&row.logical_key) {
            outstanding += 1;
            outstanding_bytes = outstanding_bytes.and_then(|sum| sum.checked_add(row.size));
        }
    }
    let size = total
        .zip(outstanding_bytes)
        .map(|(total, out)| PackageSize {
            total,
            downloaded: total - out,
            backlog: 0,
        });
    (
        KeepingData {
            scope,
            folders,
            total: read.current.rows.len(),
            outstanding,
            remote_only: Vec::new(),
            deleted_here,
        },
        size,
    )
}

/// The resolve pane, for a diverged package only: this copy's revision
/// against the `latest` the registry named just now.
async fn resolve<R: Remote + Sync>(ds: &Dataspace<R>, read: &PackageRead) -> Option<ResolveData> {
    if read.status.upstream_state != UpstreamState::Diverged {
        return None;
    }
    let Some(latest) = read.newer() else {
        return Some(ResolveData::Refused {
            reason: UNREACHED.to_string(),
        });
    };
    let published = match ds.revision_manifest(read.entry(), &latest).await {
        Ok(published) => published,
        Err(err) => {
            return Some(ResolveData::Refused {
                reason: err.to_string(),
            });
        }
    };
    let comparison = quilt::flow::compare_for_resolve(
        &read.current,
        &published,
        None,
        &std::collections::HashSet::<String>::new(),
    );
    let mut differing: Vec<String> = comparison
        .differing
        .iter()
        .map(|key| key.display().to_string())
        .collect();
    differing.sort();
    let tracked: BTreeSet<&Path> = read
        .current
        .rows
        .iter()
        .map(|row| row.logical_key.as_path())
        .collect();
    Some(ResolveData::Compared {
        loss: LatestLoss::from(&comparison),
        published_message: comparison.published_message,
        differing,
        unpublished: read.status.unpushed.len(),
        uncommitted: read
            .status
            .changes
            .keys()
            .filter(|path| tracked.contains(path.as_path()))
            .count(),
        blocked: None,
    })
}

/// The package page's one read, as `get_package_page_data` answers it.
pub(in crate::engine::commands) async fn package_page<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
) -> Result<PackagePageData, Error> {
    let namespace = namespace(text)?;
    let read = PackageRead::of(ds, &namespace, true).await?;
    let current = read.status.hashes.current.clone().ok_or_else(|| {
        Error::General(format!(
            "Installed package {namespace} has no current revision"
        ))
    })?;
    let history = ds.history(&namespace).await?;
    let revision = history.iter().find(|revision| revision.hash == current);
    let message = revision
        .and_then(|revision| revision.message.clone())
        .or_else(|| read.current.header.message().map(str::to_owned));
    let obtained_at = revision
        .and_then(|revision| revision.time)
        .map_or(0.0, |time| {
            #[allow(clippy::cast_precision_loss, reason = "milliseconds since 1970 fit")]
            let millis = time.as_millisecond() as f64;
            millis
        });
    let header = quilt::flow::Revision::from_header(
        current.clone(),
        jiff::Timestamp::UNIX_EPOCH,
        read.current.header.clone(),
    );
    let (keeping, size) = keeping(&read);
    let files = match (read.status.compared, entries(ds, &read).await) {
        (true, Ok(entries)) => FilesData::Listed(EntryList::of(entries)),
        (false, _) => FilesData::Unlisted {
            reason: read.status.problems.first().map_or_else(
                || "The folder could not be compared".to_string(),
                ToString::to_string,
            ),
        },
        (true, Err(err)) => FilesData::Unlisted {
            reason: err.to_string(),
        },
    };
    let unpushed = &read.status.unpushed;
    Ok(PackagePageData {
        header: PackageHeaderData {
            namespace: namespace.clone(),
            uri: read.uri(),
            state: page_state(&read),
            remote_locked: read.status.hashes.base.is_some(),
            has_local_commit: !unpushed.is_empty(),
            // Undo's floor, as the ref path keeps it: a first commit with
            // nothing published behind it.
            commit_has_parent: unpushed.len() > 1
                || (!unpushed.is_empty() && read.status.hashes.base.is_some()),
            role_switch: None,
            newer: read.newer(),
        },
        context: PackageContextData {
            revision: CurrentRevisionData {
                hash: current,
                message,
                obtained_at,
                meta: header.meta.map(|meta| meta.to_string()),
                workflow: header.workflow,
            },
            bucket: match &read.entry().registry {
                Registry::S3(bucket) => Some(bucket.clone()),
                _ => None,
            },
            revision_count: history.len(),
            keeping,
            size,
            resolve: resolve(ds, &read).await,
        },
        sync_paused: None,
        files,
    })
}

/// What a sync would bring now, as the Pull button's dry run: the paths
/// `latest` adds, changes and removes against the lock's base, and the
/// verdict. A path changed here and there to different bytes, or a row of
/// `conflict.json`, blocks.
pub(in crate::engine::commands) async fn pull_preview<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
) -> Result<quilt::flow::PullPreview, Error> {
    use quilt::flow::PullOutcome;

    let namespace = namespace(text)?;
    let status = ds.entry_status(&namespace, true).await?;
    let hashes = &status.hashes;
    let up_to_date = quilt::flow::PullPreview {
        outcome: PullOutcome::UpToDate,
        added: Vec::new(),
        changed: Vec::new(),
        removed: Vec::new(),
        latest_hash: None,
    };
    let Some(latest_hash) = hashes.latest.clone() else {
        return Ok(up_to_date);
    };
    if hashes.base.as_ref() == Some(&latest_hash) || hashes.current.as_ref() == Some(&latest_hash) {
        return Ok(up_to_date);
    }
    let base = match &hashes.base {
        Some(base) => ds.revision_manifest(&status.entry, base).await?,
        None => Manifest::default(),
    };
    let latest = ds.revision_manifest(&status.entry, &latest_hash).await?;
    let rows = |manifest: &Manifest| -> BTreeMap<PathBuf, quilt::manifest::ManifestRow> {
        manifest
            .rows
            .iter()
            .map(|row| (row.logical_key.as_path().to_path_buf(), row.clone()))
            .collect()
    };
    let (was, now) = (rows(&base), rows(&latest));
    let (mut added, mut changed, mut removed) = (Vec::new(), Vec::new(), Vec::new());
    // What each path is at `latest`: its bytes, or gone.
    let mut theirs: BTreeMap<PathBuf, Option<&quilt::manifest::ManifestRow>> = BTreeMap::new();
    for (path, row) in &now {
        match was.get(path) {
            None => added.push(path.clone()),
            Some(old) if !old.matches_content(row) => changed.push(path.clone()),
            Some(_) => continue,
        }
        theirs.insert(path.clone(), Some(row));
    }
    for path in was.keys().filter(|path| !now.contains_key(*path)) {
        removed.push(path.clone());
        theirs.insert(path.clone(), None);
    }
    let mut conflicts: Vec<PathBuf> = status
        .conflicts
        .keys()
        .map(|key| key.as_path().to_path_buf())
        .collect();
    for (path, change) in &status.changes {
        let Some(theirs) = theirs.get(path) else {
            continue;
        };
        let same = match (change, theirs) {
            (Change::Added(mine) | Change::Modified(mine), Some(theirs)) => {
                mine.matches_content(theirs)
            }
            (Change::Removed(_), None) => true,
            _ => false,
        };
        if !same && !conflicts.contains(path) {
            conflicts.push(path.clone());
        }
    }
    conflicts.sort();
    let outcome = if !conflicts.is_empty() {
        PullOutcome::Blocked { conflicts }
    } else if status.changes.is_empty() {
        PullOutcome::CleanUpdate
    } else {
        let of = |kind: fn(&Change) -> bool| -> Vec<PathBuf> {
            status
                .changes
                .iter()
                .filter(|(_, change)| kind(change))
                .map(|(path, _)| path.clone())
                .collect()
        };
        PullOutcome::KeepsLocalChanges {
            added: of(|change| matches!(change, Change::Added(_))),
            modified: of(|change| matches!(change, Change::Modified(_))),
            removed: of(|change| matches!(change, Change::Removed(_))),
        }
    };
    Ok(quilt::flow::PullPreview {
        outcome,
        added,
        changed,
        removed,
        latest_hash: Some(latest_hash),
    })
}

/// Download: the files named that the entry's paths do not cover become
/// its paths too, and are read at the lock's base (25t item 2), as Keeping
/// a folder is; a path the entry already covers is read already, so it is
/// left as it is. What the read left out is answered, as the ref path's.
pub(in crate::engine::commands) async fn download<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
    paths: &[String],
) -> Result<NotDownloaded, Error> {
    let entry = ds.entry(&namespace(text)?).await?;
    let mut wanted: Vec<String> = Vec::new();
    for path in paths {
        let key = LogicalKey::new(path.as_str())
            .map_err(|err| Error::Dataspace(format!("{path}: {err}")))?;
        if !entry.paths.covers(&key) {
            wanted.push(path.clone());
        }
    }
    let Paths::Only(kept) = &entry.paths else {
        return Ok(NotDownloaded::default());
    };
    if wanted.is_empty() {
        return Ok(NotDownloaded::default());
    }
    if !matches!(entry.access, Access::Read | Access::Sync) {
        return Err(Error::Dataspace(format!(
            "{} is under `{}`, which is not read; nothing is downloaded",
            entry.package, entry.access
        )));
    }
    let mut scope: Vec<String> = kept.iter().map(PackagePath::spelled).collect();
    scope.extend(wanted);
    let mut request = Keep::new(entry.key());
    request.from = Some(entry.access);
    request.paths = Some(Scope::Only(scope));
    let kept = ds.keep(&request).await?;
    match kept.read {
        KeepRead::Nothing => Ok(NotDownloaded::default()),
        KeepRead::Ran(ReadOutcome::Read(report)) => Ok(NotDownloaded {
            skipped: report
                .skipped
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
            hash_mismatch: report
                .hash_mismatch
                .iter()
                .map(|p| p.display().to_string())
                .collect(),
        }),
        KeepRead::Ran(ReadOutcome::Blocked { conflicts, .. }) => Err(Error::Dataspace(format!(
            "{} files collide with local changes; nothing was downloaded",
            conflicts.len()
        ))),
        KeepRead::Unpushed => Err(Error::Dataspace(format!(
            "{} has local commits not yet published; publish them, then download",
            entry.package
        ))),
        KeepRead::Failed(err) => Err(fail(err)),
        other => Err(Error::Dataspace(format!("{other:?}"))),
    }
}

/// The commit page's data. A dataspace commit takes a message, and its
/// metadata from `metadata.toml` in the folder (26): it takes no workflow
/// and no metadata from the form, so none is offered.
pub(in crate::engine::commands) async fn commit_data<R: Remote + Sync>(
    ds: &Dataspace<R>,
    settings: &PublishSettings,
    text: &str,
) -> Result<CommitData, Error> {
    let namespace = namespace(text)?;
    let read = PackageRead::of(ds, &namespace, true).await?;
    let status = &read.status;
    let word = match status.upstream_state {
        UpstreamState::UpToDate => "up_to_date",
        UpstreamState::Ahead => "ahead",
        UpstreamState::Behind => "behind",
        UpstreamState::Diverged => "diverged",
        UpstreamState::Local => "local",
        UpstreamState::Error => "error",
    };
    let entries = entries(ds, &read).await?;
    let ignored_count = entries
        .iter()
        .filter(|entry| entry.ignored_by.is_some() && !entry.folder)
        .count();
    let unmodified_count = read
        .current
        .rows
        .iter()
        .filter(|row| !status.changes.contains_key(row.logical_key.as_path()))
        .count();
    let list = EntryList::of(entries);
    let junky: Vec<(PathBuf, String)> = status
        .changes
        .keys()
        .filter_map(|path| quilt::junk::check(path).map(|found| (path.clone(), found.pattern)))
        .collect();
    let message = commit_message::render_publish_message(
        settings.message_template.as_deref().unwrap_or_default(),
        &commit_message::PublishMessageContext {
            namespace: &namespace,
            changes_summary: commit_message::generate(&status.changes),
        },
    );
    let header = &read.current.header;
    let (user_meta, user_meta_error) = match &header.user_meta {
        Some(meta) => match serde_json::to_string(meta) {
            Ok(text) => (text, None),
            Err(_) => (String::new(), Some("Failed to stringify meta".to_string())),
        },
        None => (String::new(), None),
    };
    let uri = read.uri();
    let host = uri.as_ref().and_then(|uri| uri.catalog.as_ref());
    let workflow = header.workflow.as_ref().map(|w| CommitWorkflowData {
        id: w.id.as_ref().map(|id| id.id.clone()),
        url: catalog_object_url(&w.config, host),
        config_url: None,
    });
    Ok(CommitData {
        namespace: namespace.clone(),
        remote_locked: status.hashes.base.is_some(),
        newer: read.newer(),
        status: word.to_string(),
        message,
        user_meta,
        user_meta_error,
        settings_user_meta: None,
        settings_workflow: None,
        has_message_template: settings
            .message_template
            .as_deref()
            .is_some_and(|t| !t.trim().is_empty()),
        workflow,
        workflows: CommitWorkflows::NotConfigured,
        no_access_reason: None,
        no_session: false,
        no_session_host: None,
        unreachable: read.unreachable(),
        entries: list.entries,
        ignored_count,
        unmodified_count,
        counts: list.counts,
        changed_bytes: changed_bytes(&status.changes),
        junk: junk_summary(&junky),
        uri,
    })
}

#[cfg(test)]
mod tests;
