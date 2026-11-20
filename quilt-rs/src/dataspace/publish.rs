//! `write` and `replace`: a contract entry's working folder published as a
//! new revision with no Domain, against the `latest` read just before.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::path::Path;
use std::path::PathBuf;

use tracing::info;
use tracing::warn;

use crate::Error;
use crate::LogicalKey;
use crate::Res;
use crate::dataspace::EntryError;
use crate::dataspace::METADATA_FILE;
use crate::dataspace::contract::Access;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::Registry;
use crate::dataspace::metadata::MetadataFile;
use crate::dataspace::metadata::at_metadata_file;
use crate::dataspace::metadata::holds_metadata_file;
use crate::dataspace::read::OnDisk;
use crate::dataspace::read::bytes_at;
use crate::dataspace::read::left_by_read;
use crate::dataspace::read::metadata_place_free;
use crate::error::PackageOpError;
use crate::flow::PushTarget;
use crate::flow::push_files;
use crate::installed_package::PushOutcome;
use crate::io::manifest::LatestSeen;
use crate::io::manifest::RowContent;
use crate::io::manifest::fetch_jsonl;
use crate::io::manifest::read_latest;
use crate::io::remote::Remote;
use crate::io::remote::WORKFLOWS_CONFIG_KEY;
use crate::io::remote::WorkflowIntent;
use crate::io::remote::resolve_workflow;
use crate::io::storage::Storage;
use crate::manifest::HeaderMessage;
use crate::manifest::Manifest;
use crate::manifest::ManifestHeader;
use crate::manifest::ManifestRow;
use crate::manifest::RawNumbers;
use crate::object_hash::ObjectHash;
use quilt_uri::S3PackageHandle;
use quilt_uri::S3Uri;

/// How many times a `write` is composed on a `latest` that moved while it
/// published, before its revision is left stored and not `latest`.
const ATTEMPTS: usize = 3;

/// How a [`publish`] ended.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Published {
    /// `latest` names the new revision.
    Advanced {
        revision: String,
        /// `latest` may have been written without the base check: the check
        /// could not run, or a failed write left it unknown whether it did.
        latest_unchecked: bool,
    },
    /// The revision is stored under its hash, and `latest` names another.
    Stored {
        revision: String,
        /// What `latest` named instead, where that is known.
        latest: Option<String>,
    },
    /// The folder holds nothing `latest` lacks, so nothing was published.
    Unchanged {
        /// `latest`, `None` where the package has none.
        latest: Option<String>,
    },
}

impl Published {
    /// The base the lock takes for a package listed only under `write` or
    /// `replace`: the new revision, once it is `latest`. `None` leaves the
    /// lock as it is. A package also listed under `read` shares its base with
    /// the read paths, which this revision may not have been read into.
    #[must_use]
    pub fn lock_base(&self) -> Option<&str> {
        match self {
            Self::Advanced { revision, .. } => Some(revision),
            Self::Stored { .. } | Self::Unchanged { .. } => None,
        }
    }
}

/// Publishes `entry`'s working folder as a new revision, built on the
/// `latest` read just before. Under `write` it is an overlay: the files under
/// the entry's paths are added or replaced, and every other key of `latest`
/// is kept. Under `replace` it is a snapshot: a key the folder lacks is
/// removed. The revision takes the bucket's default workflow.
///
/// The [metadata file](METADATA_FILE) sets the revision's message, the
/// package's metadata, and that of each file and folder under the entry's
/// paths; what it leaves out is cleared, so an empty one clears them all. With
/// none, the metadata is kept. A `message` given here wins over the file's,
/// and the file's workflow is not taken. A `replace` whose role may not read
/// `latest` has no metadata to keep.
///
/// When `latest` moves while it publishes, a `write` is composed again on
/// the new `latest`, so another writer's added files are never dropped,
/// though a metadata file clears the metadata it leaves out under the entry's
/// paths, theirs too. A `replace` is left stored and not `latest`. Either way
/// a revision that did not become `latest` is reported with its hash.
pub async fn publish(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    entry: &Entry,
    message: Option<&str>,
) -> Result<Published, EntryError> {
    let whole = match entry.access {
        Access::Write => false,
        Access::Replace => true,
        access => return Err(EntryError::Access(access)),
    };
    let Registry::S3(bucket) = &entry.registry else {
        return Err(EntryError::LocalRegistry);
    };
    let folder = &entry.folder;
    if storage.read_dir(folder).await.is_err() {
        return Err(EntryError::FolderUnavailable(folder.clone()));
    }
    let files: Vec<(LogicalKey, PathBuf)> = walk(storage, folder)
        .await?
        .into_iter()
        .filter(|(key, _)| entry.paths.covers(key))
        .collect();
    if whole && files.is_empty() {
        return Err(EntryError::NothingToReplace(folder.clone()));
    }
    let metadata = metadata_file(storage, folder).await?;

    let mut target = PushTarget {
        handle: S3PackageHandle {
            bucket: bucket.clone(),
            namespace: entry.package.clone(),
        },
        origin: entry.catalog.clone(),
        parent: None,
    };
    let workflows = S3Uri {
        bucket: bucket.clone(),
        key: WORKFLOWS_CONFIG_KEY.to_string(),
        version: None,
    };
    let header = ManifestHeader {
        message: match (message, metadata.message()) {
            (Some(text), _) => HeaderMessage::Text(text.to_string()),
            (None, Some(message)) => message.clone(),
            (None, None) => HeaderMessage::Missing,
        },
        // Taken from `latest`, or from the metadata file; a default `{}` would
        // make an empty first publish look like a change.
        user_meta: None,
        workflow: resolve_workflow(
            remote,
            target.origin.as_ref(),
            WorkflowIntent::BucketDefault,
            &workflows,
        )
        .await?,
        ..ManifestHeader::default()
    };
    let mut attempt = 1;
    let mut lost = None;
    loop {
        let outcome = Box::pin(attempt_on_latest(
            storage,
            remote,
            entry,
            &files,
            &metadata,
            header.clone(),
            message.is_none() && metadata.message().is_some(),
            &mut target,
        ))
        .await;
        // A revision an earlier attempt stored stands where this one found
        // nothing more to publish or failed, so its hash is not lost.
        if let Some(stored) = lost.take()
            && matches!(outcome, Ok(Published::Unchanged { .. }) | Err(_))
        {
            if let Err(err) = &outcome {
                warn!("Publishing again failed: {err}");
            }
            return Ok(stored);
        }
        match outcome {
            Ok(stored @ Published::Stored { .. }) if !whole && attempt < ATTEMPTS => {
                lost = Some(stored);
                attempt += 1;
            }
            outcome => return outcome,
        }
    }
}

/// One publish of `files` on the `latest` read now. The message counts as a
/// change only where the metadata file gave it, `with_message`.
#[allow(clippy::too_many_arguments)]
async fn attempt_on_latest(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    entry: &Entry,
    files: &[(LogicalKey, PathBuf)],
    metadata: &MetadataFile,
    header: ManifestHeader,
    with_message: bool,
    target: &mut PushTarget,
) -> Result<Published, EntryError> {
    let whole = entry.access == Access::Replace;
    let seen = read_latest(remote, target.origin.as_ref(), target.handle.clone()).await?;
    let base = match &seen {
        LatestSeen::Tagged { uri, .. } => Some(fetch_jsonl(remote, uri).await?),
        LatestSeen::Absent => None,
        LatestSeen::Unreadable if whole => None,
        LatestSeen::Unreadable => return Err(EntryError::LatestUnreadable),
    };
    if base.as_ref().is_some_and(holds_metadata_file) {
        return Err(EntryError::HoldsMetadataFile);
    }
    let (mut manifest, content) = compose(storage, files, base.as_ref(), whole, header).await?;
    // `push_files` uploads the file a `file://` key names, so a row of
    // `latest` kept as it is must already be in S3.
    if let Some(row) = manifest
        .rows
        .iter()
        .filter(|row| !content.contains_key(&row.logical_key))
        .find(|row| !row.physical_key.starts_with("s3://"))
    {
        return Err(Error::from(PackageOpError::Push(format!(
            "`latest` names {} at {}, which is not in S3",
            row.logical_key, row.physical_key
        )))
        .into());
    }
    metadata.apply(&mut manifest, &entry.paths);
    target.parent = seen.hash().map(str::to_string);
    if same_as(&manifest, base.as_ref(), with_message) {
        info!("Nothing to publish from {}", entry.folder.display());
        return Ok(Published::Unchanged {
            latest: target.parent.clone(),
        });
    }
    let pushed = Box::pin(push_files(storage, remote, target, manifest, &content)).await;
    settle(remote, target, pushed).await
}

/// What a push came to. Where it cannot say whether `latest` moved, `latest`
/// is read again.
async fn settle(
    remote: &impl Remote,
    target: &PushTarget,
    pushed: Res<PushOutcome>,
) -> Result<Published, EntryError> {
    let err = match pushed {
        Ok(pushed) => {
            return Ok(Published::Advanced {
                revision: pushed.manifest_uri.hash,
                latest_unchecked: pushed.latest_unchecked,
            });
        }
        Err(Error::PackageOp(PackageOpError::PushedNotLatest {
            top_hash, latest, ..
        })) => {
            return Ok(Published::Stored {
                revision: top_hash,
                latest: Some(latest),
            });
        }
        Err(err) => err,
    };
    let Error::PackageOp(PackageOpError::PushTagFailed { top_hash, reason }) = &err else {
        return Err(err.into());
    };
    let Ok(now @ (LatestSeen::Tagged { .. } | LatestSeen::Absent)) =
        read_latest(remote, target.origin.as_ref(), target.handle.clone()).await
    else {
        return Err(err.into());
    };
    Ok(match now.hash() {
        Some(hash) if hash == top_hash => Published::Advanced {
            revision: top_hash.clone(),
            latest_unchecked: true,
        },
        // `latest` still names the base, so where the refused write went
        // cannot be told: the failure stands rather than a guessed race.
        latest if latest == target.parent.as_deref() => return Err(err.into()),
        latest => {
            warn!("Stored {top_hash}, which did not become latest: {reason}");
            Published::Stored {
                revision: top_hash.clone(),
                latest: latest.map(str::to_string),
            }
        }
    })
}

async fn metadata_file(
    storage: &(impl Storage + Sync),
    folder: &Path,
) -> Result<MetadataFile, EntryError> {
    let path = folder.join(METADATA_FILE);
    // As a read has it: a folder or a link there is not the metadata file.
    if !metadata_place_free(storage, &path).await {
        return Err(EntryError::MetadataFile("it is not a file".to_string()));
    }
    let text = bytes_at(storage, &path)
        .await?
        .map(String::from_utf8)
        .transpose()
        .map_err(|err| EntryError::MetadataFile(err.to_string()))?;
    MetadataFile::parse(text.as_deref()).map_err(EntryError::MetadataFile)
}

/// The manifest to publish and the files to upload for it: the folder's
/// `files` over `base`'s rows, or alone where `whole`, with `base`'s
/// metadata. A file that holds its `base` row's bytes keeps that row and is
/// not uploaded.
async fn compose(
    storage: &(impl Storage + Sync),
    files: &[(LogicalKey, PathBuf)],
    base: Option<&Manifest>,
    whole: bool,
    mut header: ManifestHeader,
) -> Res<(Manifest, BTreeMap<LogicalKey, RowContent>)> {
    let base_rows: BTreeMap<&LogicalKey, &ManifestRow> = base
        .iter()
        .flat_map(|base| &base.rows)
        .map(|row| (&row.logical_key, row))
        .collect();
    let mut rows: BTreeMap<LogicalKey, ManifestRow> = if whole {
        BTreeMap::new()
    } else {
        base_rows
            .iter()
            .map(|(key, row)| ((*key).clone(), (*row).clone()))
            .collect()
    };
    let mut content = BTreeMap::new();
    for (key, path) in files {
        let base_row = base_rows.get(key).copied();
        let row = match base_row {
            Some(row)
                if OnDisk::read(storage, path, row.hash.algorithm())
                    .await?
                    .holds(row) =>
            {
                row.clone()
            }
            _ => {
                content.insert(key.clone(), RowContent::Path(path.clone()));
                // `push_files` takes the size and hash from the content, and
                // ignores the physical key.
                ManifestRow {
                    logical_key: key.clone(),
                    physical_key: String::new(),
                    hash: ObjectHash::default(),
                    size: 0,
                    meta: base_row.and_then(|row| row.meta.clone()),
                    meta_numbers: base_row
                        .map_or_else(RawNumbers::default, |row| row.meta_numbers.clone()),
                }
            }
        };
        rows.insert(key.clone(), row);
    }
    if let Some(base) = base {
        header.user_meta.clone_from(&base.header.user_meta);
        header
            .user_meta_numbers
            .clone_from(&base.header.user_meta_numbers);
    }
    let rows: Vec<ManifestRow> = rows.into_values().collect();
    let mut dir_meta = base.map_or_else(Vec::new, |base| base.dir_meta.clone());
    if whole {
        // A snapshot keeps a folder's metadata only while the folder holds a
        // file: one it dropped, or turned into a file, has none.
        let folders: BTreeSet<&str> = rows
            .iter()
            .filter_map(|row| row.logical_key.to_str())
            .flat_map(|key| key.match_indices('/').map(move |(at, _)| &key[..at]))
            .collect();
        dir_meta.retain(|dir| {
            dir.logical_key
                .to_str()
                .is_some_and(|key| folders.contains(key))
        });
    }
    let manifest = Manifest {
        header,
        rows,
        dir_meta,
    };
    Ok((manifest, content))
}

/// Whether `manifest`, its rows sorted, writes the same files, metadata and
/// folder metadata as `base`, and the same message where `with_message`.
fn same_as(manifest: &Manifest, base: Option<&Manifest>, with_message: bool) -> bool {
    let Some(base) = base else {
        return manifest.rows.is_empty() && manifest.header.user_meta.is_none();
    };
    let mut theirs: Vec<&ManifestRow> = base.rows.iter().collect();
    theirs.sort_by(|a, b| a.logical_key.cmp(&b.logical_key));
    (!with_message || manifest.header.message == base.header.message)
        && manifest.header.same_user_meta(&base.header)
        && manifest.dir_meta == base.dir_meta
        && manifest.rows.len() == theirs.len()
        && manifest
            .rows
            .iter()
            .zip(&theirs)
            .all(|(ours, theirs)| ours.matches_content(theirs) && ours.same_meta(theirs))
}

/// Whether `name`, at a working folder's root, is Quilt's rather than a file
/// of the package: [at the metadata file's place](at_metadata_file), or what
/// a read leaves.
fn quilts_own(name: &str) -> bool {
    if at_metadata_file(name) {
        if name != METADATA_FILE {
            warn!("Left out of the package: {name} takes the name of {METADATA_FILE}");
        }
        return true;
    }
    left_by_read(name)
}

/// The files in `folder` by logical key, leaving out what [`quilts_own`]
/// names and anything that is not a file or a folder.
async fn walk(storage: &(impl Storage + Sync), folder: &Path) -> Res<Vec<(LogicalKey, PathBuf)>> {
    let mut files = Vec::new();
    let mut queue = VecDeque::from([(folder.to_path_buf(), PathBuf::new())]);
    while let Some((dir, prefix)) = queue.pop_front() {
        let mut entries = storage.read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name();
            if prefix.as_os_str().is_empty() && quilts_own(&name.to_string_lossy()) {
                continue;
            }
            let path = dir.join(&name);
            let relative = prefix.join(&name);
            let file_type = entry.file_type().await?;
            if file_type.is_dir() {
                queue.push_back((path, relative));
            } else if file_type.is_file() {
                match LogicalKey::new(crate::paths::logical_key(&relative)) {
                    Ok(key) => files.push((key, path)),
                    Err(err) => warn!("Left out of the package: {err}"),
                }
            } else {
                warn!(
                    "Left out of the package: {} is not a file",
                    relative.display()
                );
            }
        }
    }
    Ok(files)
}

#[cfg(test)]
mod tests;
