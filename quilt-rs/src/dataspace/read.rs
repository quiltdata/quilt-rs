//! `read`: a `read` or `sync` entry's files and metadata, put in its working
//! folder with no Domain.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use aws_sdk_s3::primitives::ByteStream;
use tracing::info;
use tracing::warn;

use crate::LogicalKey;
use crate::Res;
use crate::checksum::hash_file;
use crate::dataspace::EntryError;
use crate::dataspace::METADATA_FILE;
use crate::dataspace::contract::Access;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::Paths;
use crate::dataspace::contract::Registry;
use crate::dataspace::metadata::holds_metadata_file;
use crate::dataspace::metadata::render;
use crate::flow::blocks_a_folder;
use crate::flow::edited;
use crate::flow::install_to;
use crate::flow::refuse_linked_ancestors;
use crate::io::manifest::fetch_jsonl;
use crate::io::manifest::resolve_manifest_uri;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::object_hash::ObjectHash;
use quilt_uri::ManifestUri;
use quilt_uri::S3PackageUri;

/// The prefix of the folder a read stages its downloads in, inside the
/// working folder so that each file is renamed into place.
const STAGING_PREFIX: &str = ".quilt-read-";

/// Whether `name`, at a working folder's root, is a staging folder or a
/// temporary file that a read leaves while it runs: the metadata file's, or
/// the `.tmp-<uuid>` the storage writes it through.
pub(crate) fn left_by_read(name: &str) -> bool {
    name.starts_with(STAGING_PREFIX)
        || name
            .strip_prefix(METADATA_FILE)
            .is_some_and(|rest| rest.starts_with(".tmp-"))
        || name
            .strip_prefix(".tmp-")
            .is_some_and(|rest| uuid::Uuid::parse_str(rest).is_ok())
}

/// How a [`read`] ended.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum ReadOutcome {
    /// The folder holds the revision under the entry's paths, but for the
    /// local changes it kept and any file the report says was not placed.
    Read(ReadReport),
    /// Local changes collide with the revision's, so nothing was changed.
    Blocked {
        /// The revision that was to be read.
        revision: String,
        /// The logical keys, and the metadata file, that collide.
        conflicts: Vec<PathBuf>,
    },
}

impl ReadOutcome {
    /// The base the lock takes for this package: the revision, when every
    /// file of it was placed. `None` leaves the lock as it is.
    #[must_use]
    pub fn lock_base(&self) -> Option<&str> {
        match self {
            Self::Read(report) if report.skipped.is_empty() && report.hash_mismatch.is_empty() => {
                Some(&report.revision)
            }
            Self::Read(_) | Self::Blocked { .. } => None,
        }
    }
}

/// What a [`read`] changed in the working folder, by logical key; the
/// metadata file is listed by its name.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReadReport {
    /// The revision read, as a whole top hash.
    pub revision: String,
    /// Files written, new or replacing an unchanged one.
    pub placed: Vec<PathBuf>,
    /// Files the revision dropped, deleted because they were unchanged.
    pub removed: Vec<PathBuf>,
    /// Files changed, or under `sync` deleted, locally where the revision did
    /// not change them, left as they are.
    pub kept: Vec<PathBuf>,
    /// Rows whose bytes the remote no longer holds, not placed.
    pub skipped: Vec<PathBuf>,
    /// Rows whose bytes do not hash to the row, not placed.
    pub hash_mismatch: Vec<PathBuf>,
}

/// One file to change: placed from `want`, or removed where there is no
/// `want`, only while the folder still holds `had`, or nothing where there is
/// no `had`.
struct Step<'a> {
    want: Option<&'a ManifestRow>,
    had: Option<&'a ManifestRow>,
}

impl Step<'_> {
    fn key(&self) -> &LogicalKey {
        let row = self.want.or(self.had).expect("a step has a row");
        &row.logical_key
    }
}

#[derive(Default)]
struct Plan<'a> {
    steps: Vec<Step<'a>>,
    kept: Vec<PathBuf>,
    conflicts: Vec<PathBuf>,
    /// The metadata file's new text, where it is to be written, and the bytes
    /// it replaces, `None` where there was no file.
    metadata: Option<(String, Option<Vec<u8>>)>,
}

/// What is at a path, hashed with one algorithm.
pub(crate) enum OnDisk {
    Nothing,
    NotAFile,
    File(Box<(ObjectHash, u64)>),
}

impl OnDisk {
    /// A link, dangling or not, is not a file: a rename would replace it.
    pub(crate) async fn read(
        storage: &(impl Storage + Sync),
        path: &Path,
        algorithm: u64,
    ) -> Res<Self> {
        if storage.is_symlink(path).await {
            return Ok(Self::NotAFile);
        }
        if !storage.exists(path).await {
            return Ok(Self::Nothing);
        }
        if !storage.is_file(path).await {
            return Ok(Self::NotAFile);
        }
        Ok(Self::File(Box::new(
            hash_file(storage, path, algorithm).await?,
        )))
    }

    pub(crate) fn holds(&self, row: &ManifestRow) -> bool {
        matches!(self, Self::File(file) if file.0 == row.hash && file.1 == row.size)
    }
}

/// Puts `entry`'s revision, under its paths, in its working folder, and
/// writes the [metadata file](METADATA_FILE) beside it. `base` is the
/// revision the lock names for the package, if any.
///
/// A file unchanged since `base` takes the revision's version, and one the
/// revision dropped is deleted if unchanged. A local change is kept where the
/// revision did not change that file, and collides where it did; with no
/// `base`, any file that differs collides. A missing file is filled under
/// `read`, which never publishes. Under `sync` a file `base` has and the
/// folder lacks is a local deletion, a change like any other, and only a file
/// `base` lacks is filled. So a `sync` folder that is missing, or holds none
/// of `base`'s files, is refused rather than read as every file deleted, and
/// so is a `base` the bucket no longer holds.
///
/// A failure after the first rename leaves the files already placed; the
/// lock, which moves only on [`ReadOutcome::lock_base`], still names `base`.
/// A call dropped before it returns, such as a cancelled task, leaves its
/// `.quilt-read-*` staging folder, or a `metadata.toml.tmp-*` file, in the
/// working folder; either is safe to delete.
///
/// A revision whose metadata holds an integer TOML cannot is refused before
/// anything is written.
pub async fn read(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    entry: &Entry,
    base: Option<&str>,
) -> Result<ReadOutcome, EntryError> {
    if !matches!(entry.access, Access::Read | Access::Sync) {
        return Err(EntryError::Access(entry.access));
    }
    let Registry::S3(bucket) = &entry.registry;
    let uri = S3PackageUri {
        catalog: entry.catalog.clone(),
        bucket: bucket.clone(),
        namespace: entry.package.clone(),
        revision: entry.revision.clone(),
        path: None,
    };
    let revision = resolve_manifest_uri(remote, entry.catalog.as_ref(), &uri).await?;
    let target = fetch_jsonl(remote, &revision).await?;
    if holds_metadata_file(&target) {
        return Err(EntryError::HoldsMetadataFile);
    }
    let metadata = render(&target, &entry.paths).map_err(EntryError::MetadataUnwritable)?;
    let deletes = entry.access == Access::Sync;
    let other_base = match base {
        Some(hash) if hash != revision.hash => other_base(remote, &revision, hash, deletes).await?,
        _ => None,
    };
    let base = match base {
        Some(hash) if hash == revision.hash => Some(&target),
        _ => other_base.as_ref(),
    };

    let folder = &entry.folder;
    if deletes
        && let Some(base) = base
        && gone(storage, folder, base, &entry.paths).await?
    {
        return Err(EntryError::FolderGone(folder.clone()));
    }
    let plan = plan(
        storage,
        folder,
        &entry.paths,
        &target,
        base,
        metadata,
        deletes,
    )
    .await?;
    if !plan.conflicts.is_empty() {
        info!(
            "Reading {} into {} is blocked by {} local changes",
            revision.display(),
            folder.display(),
            plan.conflicts.len()
        );
        return Ok(ReadOutcome::Blocked {
            revision: revision.hash,
            conflicts: plan.conflicts,
        });
    }

    let mut report = ReadReport {
        revision: revision.hash.clone(),
        kept: plan.kept,
        ..ReadReport::default()
    };
    let staging = folder.join(format!("{STAGING_PREFIX}{}", uuid::Uuid::new_v4()));
    let applied = apply(
        storage,
        remote,
        &revision,
        folder,
        &staging,
        &plan.steps,
        plan.metadata.as_ref(),
        &mut report,
        base.filter(|_| deletes).map(|base| (base, &entry.paths)),
    )
    .await;
    let _ = storage.remove_dir_all(&staging).await;
    if !applied? {
        warn!(
            "{} cannot be read whole: some of its files' bytes are gone",
            revision.display()
        );
    } else if let Some((text, planned_on)) = plan.metadata {
        write_metadata(storage, folder, text, planned_on.as_deref()).await?;
        report.placed.push(PathBuf::from(METADATA_FILE));
    }
    info!(
        "✔️ Read {} into {}: {} placed, {} removed, {} kept",
        revision.display(),
        folder.display(),
        report.placed.len(),
        report.removed.len(),
        report.kept.len()
    );
    Ok(ReadOutcome::Read(report))
}

/// The base `hash`, a revision other than `revision` of its package, or
/// `None` where the bucket no longer holds it and a read may compare as on a
/// first read.
async fn other_base(
    remote: &impl Remote,
    revision: &ManifestUri,
    hash: &str,
    deletes: bool,
) -> Result<Option<Manifest>, EntryError> {
    let uri = ManifestUri {
        hash: hash.to_string(),
        ..revision.clone()
    };
    match fetch_jsonl(remote, &uri).await {
        Ok(manifest) => Ok(Some(manifest)),
        // Compared as a first read: any file that differs collides. Under
        // `sync` that would fill what the user deleted.
        Err(err) if err.is_not_found() && !deletes => {
            warn!(
                "The base {} is gone; reading as if there were none",
                uri.display()
            );
            Ok(None)
        }
        Err(err) => Err(err.into()),
    }
}

/// Compares each file `paths` cover in `folder` against `target` and `base`,
/// and the metadata file against `wanted_text` and `base`'s; see [`read`].
/// Where `deletes`, a file `base` has and the folder lacks was deleted.
async fn plan<'a>(
    storage: &(impl Storage + Sync),
    folder: &Path,
    paths: &Paths,
    target: &'a Manifest,
    base: Option<&'a Manifest>,
    wanted_text: String,
    deletes: bool,
) -> Res<Plan<'a>> {
    let mut plan = Plan::default();
    let wanted = covered(target, paths);
    let had = base.map_or_else(BTreeMap::new, |base| covered(base, paths));
    // The revision's keys found on disk, by their case-folded names: on a disk
    // that ignores case, a key renamed only in case finds the old key's file,
    // which removing the old key would delete.
    let mut found = BTreeMap::new();
    let had_folded: BTreeMap<String, &ManifestRow> = had
        .iter()
        .map(|(key, row)| (key.to_string_lossy().to_lowercase(), *row))
        .collect();
    for (key, want) in &wanted {
        let path = key.join_to(folder);
        let algorithm = want.hash.algorithm();
        let disk = OnDisk::read(storage, &path, algorithm).await?;
        let any_case = key.to_string_lossy().to_lowercase();
        if !matches!(disk, OnDisk::Nothing) {
            found.insert(any_case.clone(), *key);
        }
        // Where the disk found this key under the base's other spelling, that
        // spelling's row is what the file was read as.
        let had = match had.get(key) {
            Some(row) => Some(*row),
            None if !matches!(disk, OnDisk::Nothing)
                && had_folded.contains_key(&any_case)
                && !listed(storage, folder, key).await? =>
            {
                had_folded.get(&any_case).copied()
            }
            None => None,
        };
        // Under `sync`, a missing file `base` had was deleted: a local change,
        // compared below like any other. A folder above it that cannot be
        // listed is not a deletion, and collides first.
        let missing = matches!(disk, OnDisk::Nothing);
        if missing && folder_blocked(storage, folder, &path).await {
            plan.conflicts.push(key.to_path_buf());
        } else if missing && !(deletes && had.is_some() && not_found(storage, &path).await?) {
            plan.steps.push(Step {
                want: Some(want),
                had: None,
            });
        } else if disk.holds(want) {
        } else if let Some(had) = had
            && had.hash == want.hash
            && had.size == want.size
        {
            plan.kept.push(key.to_path_buf());
        } else if let Some(had) = had
            && (if had.hash.algorithm() == algorithm {
                disk.holds(had)
            } else {
                OnDisk::read(storage, &path, had.hash.algorithm())
                    .await?
                    .holds(had)
            })
        {
            plan.steps.push(Step {
                want: Some(want),
                had: Some(had),
            });
        } else {
            plan.conflicts.push(key.to_path_buf());
        }
    }
    // Two keys can name one local path, as `a\b` and `a/b` do on Windows.
    let wanted_paths: std::collections::BTreeSet<PathBuf> =
        wanted.keys().map(|key| key.join_to(folder)).collect();
    for (key, had) in had.iter().filter(|(key, _)| !wanted.contains_key(*key)) {
        if wanted_paths.contains(&key.join_to(folder)) {
            continue;
        }
        // Where the disk lists either spelling under the other, both name one
        // file.
        if let Some(renamed) = found.get(&key.to_string_lossy().to_lowercase())
            && (!listed(storage, folder, renamed).await? || !listed(storage, folder, key).await?)
        {
            continue;
        }
        let path = key.join_to(folder);
        match OnDisk::read(storage, &path, had.hash.algorithm()).await? {
            OnDisk::Nothing => {}
            disk if disk.holds(had) => plan.steps.push(Step {
                want: None,
                had: Some(had),
            }),
            _ => plan.conflicts.push(key.to_path_buf()),
        }
    }
    plan_metadata(storage, folder, paths, wanted_text, base, &mut plan).await?;
    Ok(plan)
}

/// Compares the metadata file against `wanted_text` and `base`'s; see
/// [`read`].
async fn plan_metadata(
    storage: &(impl Storage + Sync),
    folder: &Path,
    paths: &Paths,
    wanted_text: String,
    base: Option<&Manifest>,
    plan: &mut Plan<'_>,
) -> Res {
    let metadata_path = folder.join(METADATA_FILE);
    // A base TOML cannot hold was never written, so no file holds it.
    let had_text = base.and_then(|base| render(base, paths).ok());
    if !metadata_place_free(storage, &metadata_path).await {
        plan.conflicts.push(PathBuf::from(METADATA_FILE));
        return Ok(());
    }
    let on_disk = bytes_at(storage, &metadata_path).await?;
    match &on_disk {
        Some(text) if text == wanted_text.as_bytes() => {}
        Some(_) if had_text.as_ref() == Some(&wanted_text) => {
            plan.kept.push(PathBuf::from(METADATA_FILE));
        }
        None => plan.metadata = Some((wanted_text, on_disk)),
        Some(text) if had_text.as_ref().map(String::as_bytes) == Some(text.as_slice()) => {
            plan.metadata = Some((wanted_text, on_disk));
        }
        Some(_) => plan.conflicts.push(PathBuf::from(METADATA_FILE)),
    }
    Ok(())
}

/// Whether nothing is at `path`. An error that cannot say, such as a folder
/// above it that may be listed but not searched, is returned rather than
/// taken for a deletion.
async fn not_found(storage: &(impl Storage + Sync), path: &Path) -> Res<bool> {
    match storage.open_file(path).await {
        Ok(_) => Ok(false),
        Err(err) if err.is_not_found() => Ok(true),
        Err(err) => Err(err),
    }
}

/// Whether `base` covers a key under `paths` and nothing, not even a dangling
/// link, is at any of them in `folder`. A folder that is there and cannot be
/// listed is an error, not a folder gone.
async fn gone(
    storage: &(impl Storage + Sync),
    folder: &Path,
    base: &Manifest,
    paths: &Paths,
) -> Res<bool> {
    let had = covered(base, paths);
    match storage.read_dir(folder).await {
        Ok(_) => {}
        Err(err) if err.is_not_found() => return Ok(!had.is_empty()),
        Err(err) => return Err(err),
    }
    for key in had.keys() {
        let path = key.join_to(folder);
        if storage.is_symlink(&path).await
            || storage.exists(&path).await
            || !not_found(storage, &path).await?
        {
            return Ok(false);
        }
    }
    Ok(!had.is_empty())
}

/// Whether a file, or anything that is not a folder, stands between `folder`
/// and `path` where a folder must go.
async fn folder_blocked(storage: &(impl Storage + Sync), folder: &Path, path: &Path) -> bool {
    for ancestor in path
        .ancestors()
        .skip(1)
        .take_while(|ancestor| ancestor.starts_with(folder))
    {
        if blocks_a_folder(storage, ancestor).await {
            return true;
        }
    }
    false
}

/// The bytes of the file at `path`, `None` where there is none.
pub(crate) async fn bytes_at(storage: &(impl Storage + Sync), path: &Path) -> Res<Option<Vec<u8>>> {
    match storage.read_bytes(path).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(err) if err.is_not_found() => Ok(None),
        Err(err) => Err(err),
    }
}

/// Whether every folder between `folder` and `key`'s file lists the next name
/// exactly, so that a disk that ignores case did not find it under another.
async fn listed(storage: &(impl Storage + Sync), folder: &Path, key: &LogicalKey) -> Res<bool> {
    let mut parent = folder.to_path_buf();
    for name in key.iter() {
        let mut entries = storage.read_dir(&parent).await?;
        let mut found = false;
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_name() == name {
                found = true;
                break;
            }
        }
        if !found {
            return Ok(false);
        }
        parent.push(name);
    }
    Ok(true)
}

/// Whether the folder still holds what `step` was planned against: its
/// `had`, or nothing at all. A link holds neither. Where `deletes`, a file
/// to be replaced that has gone since was deleted, which the read must not
/// undo.
async fn as_planned(
    storage: &(impl Storage + Sync),
    folder: &Path,
    step: &Step<'_>,
    deletes: bool,
) -> Res<bool> {
    let path = step.key().join_to(folder);
    if storage.is_symlink(&path).await {
        return Ok(false);
    }
    if deletes && step.want.is_some() && step.had.is_some() && !storage.exists(&path).await {
        return Ok(false);
    }
    Ok(match step.had {
        Some(had) => !edited(storage, &path, had).await?,
        None => !storage.exists(&path).await,
    })
}

/// Writes the metadata file's `text` beside it, then renames it into place
/// where the file still holds `planned_on`.
async fn write_metadata(
    storage: &(impl Storage + Sync),
    folder: &Path,
    text: String,
    planned_on: Option<&[u8]>,
) -> Result<(), EntryError> {
    let temp = folder.join(format!("{METADATA_FILE}.tmp-{}", uuid::Uuid::new_v4()));
    storage
        .write_byte_stream(&temp, ByteStream::from(text.into_bytes()))
        .await?;
    let placed = async {
        if !metadata_as_planned(storage, folder, planned_on).await? {
            return Err(EntryError::Changed(PathBuf::from(METADATA_FILE)));
        }
        storage.rename(&temp, folder.join(METADATA_FILE)).await?;
        Ok(())
    }
    .await;
    if placed.is_err() {
        let _ = storage.remove_file(&temp).await;
    }
    placed
}

/// Whether the metadata file still holds the bytes it was planned against,
/// `None` being no file. A link holds none.
async fn metadata_as_planned(
    storage: &(impl Storage + Sync),
    folder: &Path,
    planned_on: Option<&[u8]>,
) -> Res<bool> {
    let path = folder.join(METADATA_FILE);
    if !metadata_place_free(storage, &path).await {
        return Ok(false);
    }
    // A file replaced while it is read can still read as the old bytes, but
    // its modified time moves, as `edited` has it.
    let modified = || async {
        match storage.modified_timestamp(&path).await {
            Ok(at) => Ok(Some(at)),
            Err(err) if err.is_not_found() => Ok(None),
            Err(err) => Err(err),
        }
    };
    let before = modified().await?;
    let bytes = bytes_at(storage, &path).await?;
    Ok(bytes.as_deref() == planned_on && modified().await? == before)
}

/// Whether the metadata file's place holds a file or nothing: not a link,
/// whatever it leads to, nor a folder.
pub(crate) async fn metadata_place_free(storage: &(impl Storage + Sync), path: &Path) -> bool {
    !storage.is_symlink(path).await && (!storage.exists(path).await || storage.is_file(path).await)
}

/// Fetches the files `steps` place into `staging`, then, once the folder is
/// seen to hold what every step and the metadata file, where it is to be
/// written, were planned against, renames each into the folder and deletes
/// the files they remove. Changes nothing, and returns `false`, where a file
/// could not be fetched. Under `sync`, with its base and paths in
/// `sync_base`, a folder that lost every file of the base meanwhile is
/// refused: the staging folder inside it may have put it back.
#[allow(clippy::too_many_arguments)]
async fn apply(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    revision: &ManifestUri,
    folder: &Path,
    staging: &Path,
    steps: &[Step<'_>],
    metadata: Option<&(String, Option<Vec<u8>>)>,
    report: &mut ReadReport,
    sync_base: Option<(&Manifest, &Paths)>,
) -> Result<bool, EntryError> {
    let deletes = sync_base.is_some();
    // A linked folder would put a write or a deletion outside the folder. Asked
    // before the fetch, and again after it.
    refuse_linked_ancestors(storage, folder, steps.iter().map(Step::key)).await?;
    let fetch: Vec<ManifestRow> = steps.iter().filter_map(|step| step.want.cloned()).collect();
    if !fetch.is_empty() {
        let fetched = install_to(
            storage,
            remote,
            revision.origin.as_ref(),
            &Manifest {
                rows: fetch,
                ..Manifest::default()
            },
            staging,
            None,
        )
        .await?;
        report.skipped = fetched.skipped;
        report.hash_mismatch = fetched.hash_mismatch;
    }
    // A revision some of whose bytes are gone cannot be read whole, and a
    // folder holding part of it under the old base would read as local work.
    if !report.skipped.is_empty() || !report.hash_mismatch.is_empty() {
        return Ok(false);
    }
    refuse_linked_ancestors(storage, folder, steps.iter().map(Step::key)).await?;
    if let Some((base, paths)) = sync_base
        && gone(storage, folder, base, paths).await?
    {
        return Err(EntryError::FolderGone(folder.to_path_buf()));
    }
    // Every step before the first change, so a concurrent edit stops the read
    // whole, then each again right before its own change.
    for step in steps {
        if !as_planned(storage, folder, step, deletes).await? {
            return Err(EntryError::Changed(step.key().to_path_buf()));
        }
    }
    if let Some((_, planned_on)) = metadata
        && !metadata_as_planned(storage, folder, planned_on.as_deref()).await?
    {
        return Err(EntryError::Changed(PathBuf::from(METADATA_FILE)));
    }
    for step in steps {
        let key = step.key();
        let path = key.join_to(folder);
        if !as_planned(storage, folder, step, deletes).await? {
            return Err(EntryError::Changed(key.to_path_buf()));
        }
        refuse_linked_ancestors(storage, folder, std::iter::once(key)).await?;
        if step.want.is_some() {
            if let Some(parent) = path.parent() {
                storage.create_dir_all(parent).await?;
            }
            storage.rename(key.join_to(staging), &path).await?;
            report.placed.push(key.to_path_buf());
        } else {
            match storage.remove_file(&path).await {
                Ok(()) => report.removed.push(key.to_path_buf()),
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => return Err(crate::Error::from(err).into()),
            }
        }
    }
    Ok(true)
}

fn covered<'a>(manifest: &'a Manifest, paths: &Paths) -> BTreeMap<&'a LogicalKey, &'a ManifestRow> {
    manifest
        .rows
        .iter()
        .filter(|row| paths.covers(&row.logical_key))
        .map(|row| (&row.logical_key, row))
        .collect()
}

#[cfg(test)]
mod tests;
