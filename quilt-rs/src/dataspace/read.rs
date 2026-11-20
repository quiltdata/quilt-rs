//! `read`: a `read` or `sync` entry's files and metadata, put in its working
//! folder with no Domain.

use std::borrow::Cow;
use std::collections::BTreeMap;
use std::collections::VecDeque;
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
use crate::dataspace::conflict::Other;
use crate::dataspace::conflict::Row;
use crate::dataspace::conflict::Side;
use crate::dataspace::contract::Access;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::Paths;
use crate::dataspace::contract::Registry;
use crate::dataspace::metadata::MetadataFile;
use crate::dataspace::metadata::at_metadata_file;
use crate::dataspace::metadata::holds_metadata_file;
use crate::dataspace::metadata::render;
use crate::dataspace::store::local_registry;
use crate::domain::LocalRegistry;
use crate::error::FsError;
use crate::flow::InstallToReport;
use crate::flow::PackageStore;
use crate::flow::blocks_a_folder;
use crate::flow::edited;
use crate::flow::install_to;
use crate::flow::refuse_clashing_local_paths;
use crate::flow::refuse_linked_ancestors;
use crate::flow::resolve_local_hash;
use crate::flow::row_objects;
use crate::io::manifest::fetch_jsonl;
use crate::io::manifest::hash_bytes;
use crate::io::manifest::resolve_manifest_uri;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::manifest::RawNumbers;
use crate::object_hash::MULTIHASH_SHA256;
use crate::object_hash::ObjectHash;
use crate::paths::file_url;
use quilt_uri::Host;
use quilt_uri::ManifestUri;
use quilt_uri::Namespace;
use quilt_uri::RevisionPointer;
use quilt_uri::S3PackageUri;
use quilt_uri::S3Uri;
use quilt_uri::Tag;
use quilt_uri::UriError;

/// The prefix of the folder a read stages its downloads in, inside the
/// working folder so that each file is renamed into place.
pub(crate) const STAGING_PREFIX: &str = ".quilt-read-";

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
    /// Colliding keys set as `conflict.json` decided.
    pub resolved: Vec<PathBuf>,
}

impl ReadReport {
    /// Lists a decided key only as `resolved`.
    fn keep_resolved_apart(&mut self) {
        let resolved = &self.resolved;
        self.placed.retain(|key| !resolved.contains(key));
        self.removed.retain(|key| !resolved.contains(key));
    }
}

/// One file to change: placed from `source`, or removed where there is none,
/// only while the folder still holds `had`, or nothing where there is no
/// `had`.
struct Step<'a> {
    key: &'a LogicalKey,
    source: Option<Source<'a>>,
    had: Option<Cow<'a, ManifestRow>>,
}

impl<'a> Step<'a> {
    fn new(
        key: &'a LogicalKey,
        want: Option<&'a ManifestRow>,
        had: Option<&'a ManifestRow>,
    ) -> Self {
        Self {
            key,
            source: want.map(Source::Revision),
            had: had.map(Cow::Borrowed),
        }
    }
}

/// Where a file to place comes from.
enum Source<'a> {
    Revision(&'a ManifestRow),
    File(PathBuf),
    Object(S3Uri),
}

#[derive(Default)]
struct Plan<'a> {
    steps: Vec<Step<'a>>,
    kept: Vec<PathBuf>,
    conflicts: Vec<PathBuf>,
    resolved: Vec<PathBuf>,
    /// Every colliding key, decided or not.
    rows: BTreeMap<LogicalKey, Row>,
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

    /// What is at `path`, as a row of `conflict.json` describes it.
    pub(crate) fn side(&self, path: &Path) -> Res<Option<Side>> {
        let (hash, size) = match self {
            Self::Nothing => return Ok(None),
            Self::NotAFile => (None, None),
            Self::File(file) => (Some(file.0.clone()), Some(file.1)),
        };
        Ok(Some(Side {
            physical_key: Some(file_url(path)?),
            hash,
            size,
        }))
    }
}

/// Puts `entry`'s revision, under its paths, in its working folder, and
/// writes the [metadata file](METADATA_FILE) beside it, which holds the
/// metadata of the whole revision, outside the paths too. `base` is the
/// revision the lock names for the package, if any.
///
/// A file unchanged since `base` takes the revision's version, and one the
/// revision dropped is deleted if unchanged. A local change is kept where the
/// revision did not change that file, and collides where it did; with no
/// `base`, any file that differs collides. A missing file is filled under
/// `read`, which never publishes. Under `sync` a file `base` has and the
/// folder lacks is a local deletion, a change like any other, and only a file
/// `base` lacks is filled. So a `sync` folder that is missing, or empty but
/// for what Quilt writes there, is refused rather than read as every file
/// deleted, and so is a `base` the bucket no longer holds.
///
/// A failure after the first rename leaves the files already placed; the
/// lock, which moves only on [`ReadOutcome::lock_base`], still names `base`.
/// A call dropped before it returns, such as a cancelled task, leaves its
/// `.quilt-read-*` staging folder, or a `metadata.toml.tmp-*` file, in the
/// working folder; either is safe to delete.
///
/// A revision whose metadata holds an integer TOML cannot is refused before
/// anything is written.
///
/// An entry of the local registry `local`, under its name or one it had, is
/// read from it as a bucket's entry is read from the bucket. With no local
/// registry given, or another machine's, such an entry is refused.
pub async fn read(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    local: Option<&LocalRegistry>,
    entry: &Entry,
    base: Option<&str>,
) -> Result<ReadOutcome, EntryError> {
    let (outcome, _) = Box::pin(read_deciding(
        storage,
        remote,
        local,
        entry,
        base,
        &BTreeMap::new(),
    ))
    .await?;
    Ok(outcome)
}

/// The revision a read places, and where its manifests and bytes are: in a
/// bucket, reached through the remote, or in the local registry.
enum Revision<'a> {
    Bucket(ManifestUri),
    Local {
        registry: &'a LocalRegistry,
        package: &'a Namespace,
        hash: String,
    },
    /// A local commit of a bucket's package, not yet pushed: each of its
    /// rows fetched from the bucket, or copied from the store's `objects`.
    Recorded {
        uri: ManifestUri,
        objects: PathBuf,
    },
}

impl Revision<'_> {
    fn hash(&self) -> &str {
        match self {
            Self::Bucket(uri) | Self::Recorded { uri, .. } => &uri.hash,
            Self::Local { hash, .. } => hash,
        }
    }

    /// The catalog that signs the remote's reads; the local registry has
    /// none.
    fn origin(&self) -> Option<&Host> {
        match self {
            Self::Bucket(uri) | Self::Recorded { uri, .. } => uri.origin.as_ref(),
            Self::Local { .. } => None,
        }
    }

    fn display(&self) -> String {
        match self {
            Self::Bucket(uri) => uri.display(),
            Self::Local { package, hash, .. } => format!("{package}@{hash} in the local registry"),
            Self::Recorded { uri, .. } => format!("{} as committed locally", uri.display()),
        }
    }

    /// Puts the bytes of `rows` of the revision in `staging`: fetched from
    /// the bucket, or copied from the local registry or the store.
    async fn fetch(
        &self,
        storage: &(impl Storage + Sync),
        remote: &impl Remote,
        rows: Vec<ManifestRow>,
        staging: &Path,
    ) -> Res<InstallToReport> {
        let fetch = |rows| async {
            let manifest = Manifest {
                rows,
                ..Manifest::default()
            };
            install_to(storage, remote, self.origin(), &manifest, staging, None).await
        };
        match self {
            Self::Bucket(_) => fetch(rows).await,
            Self::Local { registry, .. } => {
                copy_from(storage, &registry.objects_dir(), &rows, staging).await
            }
            Self::Recorded { objects, .. } => {
                let (fetched, copied): (Vec<ManifestRow>, Vec<ManifestRow>) = rows
                    .into_iter()
                    .partition(|row| row.physical_key.starts_with("s3://"));
                let mut report = copy_from(storage, objects, &copied, staging).await?;
                if !fetched.is_empty() {
                    let mut more = fetch(fetched).await?;
                    report.placed.append(&mut more.placed);
                    report.skipped.append(&mut more.skipped);
                    report.hash_mismatch.append(&mut more.hash_mismatch);
                }
                Ok(report)
            }
        }
    }

    /// The revision `hash` of the same package.
    async fn other(
        &self,
        storage: &(impl Storage + Sync),
        remote: &impl Remote,
        hash: &str,
    ) -> Res<Manifest> {
        match self {
            Self::Bucket(uri) | Self::Recorded { uri, .. } => {
                let uri = ManifestUri {
                    hash: hash.to_string(),
                    ..uri.clone()
                };
                fetch_jsonl(remote, &uri).await
            }
            Self::Local { registry, .. } => {
                Manifest::from_path(storage, &registry.revision(hash)).await
            }
        }
    }
}

/// The revision `entry` names, in its bucket or in the local registry
/// `local`, and its manifest.
async fn revision_of<'a>(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    local: Option<&'a LocalRegistry>,
    entry: &'a Entry,
) -> Result<(Revision<'a>, Manifest), EntryError> {
    if let Some(registry) = local_registry(storage, local, entry).await? {
        return local_revision(storage, registry, entry).await;
    }
    let Registry::S3(bucket) = &entry.registry else {
        return Err(EntryError::LocalRegistry);
    };
    let uri = S3PackageUri {
        catalog: entry.catalog.clone(),
        bucket: bucket.clone(),
        namespace: entry.package.clone(),
        revision: entry.revision.clone(),
        path: None,
    };
    let uri = resolve_manifest_uri(remote, entry.catalog.as_ref(), &uri).await?;
    let target = fetch_jsonl(remote, &uri).await?;
    Ok((Revision::Bucket(uri), target))
}

/// The revision `entry` names in the local registry `registry`, and its
/// manifest; see [`local_hash`].
async fn local_revision<'a>(
    storage: &(impl Storage + Sync),
    registry: &'a LocalRegistry,
    entry: &'a Entry,
) -> Result<(Revision<'a>, Manifest), EntryError> {
    let hash = local_hash(storage, registry, entry).await?;
    let manifest = Manifest::from_path(storage, &registry.revision(&hash)).await?;
    let revision = Revision::Local {
        registry,
        package: &entry.package,
        hash,
    };
    Ok((revision, manifest))
}

/// The hash of the revision `entry` names in the local registry
/// `registry`. The registry keeps no tag but `latest`, and a hash prefix
/// names the one revision of the registry whose hash it starts.
pub(crate) async fn local_hash(
    storage: &(impl Storage + Sync),
    registry: &LocalRegistry,
    entry: &Entry,
) -> Result<String, EntryError> {
    let package = &entry.package;
    Ok(match &entry.revision {
        RevisionPointer::Tag(Tag::Latest) => registry
            .read_latest(storage, package)
            .await?
            .ok_or_else(|| {
                crate::Error::Fs(FsError::NotFound {
                    path: registry.latest(package),
                })
            })?,
        RevisionPointer::Hash(hash) => hash.clone(),
        RevisionPointer::HashPrefix(prefix) => {
            resolve_local_hash(storage, registry.revisions_dir(), prefix).await?
        }
        RevisionPointer::Tag(tag) => {
            return Err(crate::Error::from(UriError::Package(format!(
                "the local registry keeps no tag but latest, so it has no revision at {tag}"
            )))
            .into());
        }
    })
}

/// [`read`], setting each colliding key whose row in `decided` still
/// describes it as the row decides; see
/// [`read_resolving`](crate::dataspace::conflict::read_resolving). Returns
/// with the outcome a row for every colliding key, decided or not. An entry
/// of the local registry `local` is read from it as a bucket's is read from
/// the bucket.
pub(crate) async fn read_deciding(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    local: Option<&LocalRegistry>,
    entry: &Entry,
    base: Option<&str>,
    decided: &BTreeMap<LogicalKey, Row>,
) -> Result<(ReadOutcome, BTreeMap<LogicalKey, Row>), EntryError> {
    if !matches!(entry.access, Access::Read | Access::Sync) {
        return Err(EntryError::Access(entry.access));
    }
    let (revision, target) = revision_of(storage, remote, local, entry).await?;
    let deletes = entry.access == Access::Sync;
    let other_base = match base {
        Some(hash) if hash != revision.hash() => {
            other_base(storage, remote, &revision, hash, deletes).await?
        }
        _ => None,
    };
    let base = match base {
        Some(hash) if hash == revision.hash() => Some(&target),
        _ => other_base.as_ref(),
    };
    read_onto(storage, remote, entry, &revision, &target, base, decided).await
}

/// Reads `target`, a revision a merge recorded in `store` and has not
/// pushed, or `latest` itself where the merge left it as it was, into
/// `entry`'s folder over `base`, as [`read_deciding`] reads a revision: the
/// rows the merge took from `latest`, the revision `latest`, are fetched from
/// its registry, and the rest copied from the store.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn read_recorded(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    local: Option<&LocalRegistry>,
    store: &impl PackageStore,
    entry: &Entry,
    hash: &str,
    target: &Manifest,
    base: &Manifest,
    decided: &BTreeMap<LogicalKey, Row>,
) -> Result<(ReadOutcome, BTreeMap<LogicalKey, Row>), EntryError> {
    let revision = match revision_at(storage, local, entry, hash).await? {
        Revision::Bucket(uri) => Revision::Recorded {
            uri,
            objects: store.objects_dir(),
        },
        revision => revision,
    };
    read_onto(
        storage,
        remote,
        entry,
        &revision,
        target,
        Some(base),
        decided,
    )
    .await
}

/// [`read_deciding`], of `target`, the manifest of `revision`, over `base`.
async fn read_onto(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    entry: &Entry,
    revision: &Revision<'_>,
    target: &Manifest,
    base: Option<&Manifest>,
    decided: &BTreeMap<LogicalKey, Row>,
) -> Result<(ReadOutcome, BTreeMap<LogicalKey, Row>), EntryError> {
    if holds_metadata_file(target) {
        return Err(EntryError::HoldsMetadataFile);
    }
    let metadata = render(target).map_err(EntryError::MetadataUnwritable)?;
    let deletes = entry.access == Access::Sync;
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
        target,
        base,
        metadata,
        deletes,
        decided,
    )
    .await?;
    if !plan.conflicts.is_empty() {
        info!(
            "Reading {} into {} is blocked by {} local changes",
            revision.display(),
            folder.display(),
            plan.conflicts.len()
        );
        let blocked = ReadOutcome::Blocked {
            revision: revision.hash().to_string(),
            conflicts: plan.conflicts,
        };
        return Ok((blocked, plan.rows));
    }

    let mut report = ReadReport {
        revision: revision.hash().to_string(),
        kept: plan.kept,
        resolved: plan.resolved,
        ..ReadReport::default()
    };
    let staging = folder.join(format!("{STAGING_PREFIX}{}", uuid::Uuid::new_v4()));
    let applied = apply(
        storage,
        remote,
        revision,
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
    report.keep_resolved_apart();
    info!(
        "✔️ Read {} into {}: {} placed, {} removed, {} kept",
        revision.display(),
        folder.display(),
        report.placed.len(),
        report.removed.len(),
        report.kept.len()
    );
    Ok((ReadOutcome::Read(report), plan.rows))
}

/// The base `hash`, a revision other than `revision` of its package, or
/// `None` where its registry no longer holds it and a read may compare as on
/// a first read.
async fn other_base(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    revision: &Revision<'_>,
    hash: &str,
    deletes: bool,
) -> Result<Option<Manifest>, EntryError> {
    match revision.other(storage, remote, hash).await {
        Ok(manifest) => Ok(Some(manifest)),
        // Compared as a first read: any file that differs collides. Under
        // `sync` that would fill what the user deleted.
        Err(err) if err.is_not_found() && !deletes => {
            warn!("The base {hash} is gone; reading as if there were none");
            Ok(None)
        }
        Err(err) => Err(err.into()),
    }
}

/// Compares each file `paths` cover in `folder` against `target` and `base`,
/// and the metadata file against `wanted_text` and `base`'s; see [`read`].
/// Where `deletes`, a file `base` has and the folder lacks was deleted. A
/// colliding key is set as its row in `decided` says, where that row still
/// describes it.
#[allow(clippy::too_many_arguments)]
async fn plan<'a>(
    storage: &(impl Storage + Sync),
    folder: &Path,
    paths: &Paths,
    target: &'a Manifest,
    base: Option<&'a Manifest>,
    wanted_text: String,
    deletes: bool,
    decided: &BTreeMap<LogicalKey, Row>,
) -> Result<Plan<'a>, EntryError> {
    let mut plan = plan_files(storage, folder, paths, target, base, deletes, decided).await?;
    plan_metadata(storage, folder, wanted_text, base, decided, &mut plan).await?;
    Ok(plan)
}

/// [`plan`], for the files alone.
async fn plan_files<'a>(
    storage: &(impl Storage + Sync),
    folder: &Path,
    paths: &Paths,
    target: &'a Manifest,
    base: Option<&'a Manifest>,
    deletes: bool,
    decided: &BTreeMap<LogicalKey, Row>,
) -> Result<Plan<'a>, EntryError> {
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
            plan.collide(storage, key, Some(want), &path, &OnDisk::NotAFile, decided)
                .await?;
        } else if missing && !(deletes && had.is_some() && not_found(storage, &path).await?) {
            plan.steps.push(Step::new(key, Some(want), None));
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
            plan.steps.push(Step::new(key, Some(want), Some(had)));
        } else {
            plan.collide(storage, key, Some(want), &path, &disk, decided)
                .await?;
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
            disk if disk.holds(had) => plan.steps.push(Step::new(key, None, Some(had))),
            disk => {
                plan.collide(storage, key, None, &path, &disk, decided)
                    .await?;
            }
        }
    }
    Ok(plan)
}

impl<'a> Plan<'a> {
    /// Records `row` for the colliding `key`, taking the decision of its row
    /// in `decided` where that row still describes both sides, and returns
    /// the decision.
    fn record(
        &mut self,
        key: &LogicalKey,
        mut row: Row,
        decided: &BTreeMap<LogicalKey, Row>,
    ) -> Option<String> {
        let here = |side: &Option<Side>| side.as_ref().and_then(|side| side.physical_key.clone());
        row.resolved = decided
            .get(key)
            .filter(|earlier| earlier.same_collision(&row))
            .and_then(|earlier| match &earlier.resolved {
                // The folder's own file, wherever its path was spelled then.
                Some(resolved) if Some(resolved) == here(&earlier.current).as_ref() => {
                    here(&row.current)
                }
                resolved => resolved.clone(),
            });
        let resolved = row.resolved.clone();
        match resolved {
            Some(_) => self.resolved.push(key.to_path_buf()),
            None => self.conflicts.push(key.to_path_buf()),
        }
        self.rows.insert(key.clone(), row);
        resolved
    }

    /// Records that `key` collides, `want` being the revision's row and
    /// `disk` what is at `path`, and plans the step its decision takes.
    async fn collide(
        &mut self,
        storage: &(impl Storage + Sync),
        key: &'a LogicalKey,
        want: Option<&'a ManifestRow>,
        path: &Path,
        disk: &OnDisk,
        decided: &BTreeMap<LogicalKey, Row>,
    ) -> Result<(), EntryError> {
        let row = Row {
            other: Other::Incoming(want.map(row_side)),
            current: disk.side(path)?,
            resolved: None,
        };
        let here = row
            .current
            .as_ref()
            .and_then(|side| side.physical_key.clone());
        let Some(resolved) = self.record(key, row, decided) else {
            return Ok(());
        };
        let had = match disk {
            // The folder's own file, kept as it is.
            OnDisk::File(_) if here.as_ref() == Some(&resolved) => return Ok(()),
            OnDisk::File(file) => Some(Cow::Owned(ManifestRow {
                logical_key: key.clone(),
                physical_key: String::new(),
                hash: file.0.clone(),
                size: file.1,
                meta: None,
                meta_numbers: RawNumbers::default(),
            })),
            OnDisk::Nothing => None,
            OnDisk::NotAFile => return Err(refused(key, NOT_A_FILE)),
        };
        let source = source(storage, &resolved, want)
            .await
            .map_err(|why| refused(key, why))?;
        if source.is_some() || had.is_some() {
            self.steps.push(Step { key, source, had });
        }
        Ok(())
    }
}

const NOT_A_FILE: &str = "the working folder holds something other than a file there";

fn refused(key: &LogicalKey, why: impl std::fmt::Display) -> EntryError {
    EntryError::ConflictMap(format!("{key} cannot be set: {why}"))
}

/// Where `resolved` takes a file from: `None` for `""`, which deletes it,
/// the revision's own object `want`, a local file, or an S3 object at an
/// exact version.
async fn source<'a>(
    storage: &(impl Storage + Sync),
    resolved: &str,
    want: Option<&'a ManifestRow>,
) -> Result<Option<Source<'a>>, String> {
    if resolved.is_empty() {
        return Ok(None);
    }
    if let Some(want) = want.filter(|want| want.physical_key == resolved) {
        return Ok(Some(Source::Revision(want)));
    }
    if resolved.starts_with("s3://") {
        let uri = S3Uri::try_from(resolved).map_err(|err| err.to_string())?;
        // Without a version, the key could name other bytes by the time it is read.
        return match uri.version.as_deref() {
            Some(version) if version != "null" => Ok(Some(Source::Object(uri))),
            _ => Err(format!("{resolved} names no versionId")),
        };
    }
    let path = local_file(resolved)?;
    if storage.is_symlink(&path).await || !storage.is_file(&path).await {
        return Err(format!("{resolved} is not a file"));
    }
    Ok(Some(Source::File(path)))
}

fn local_file(resolved: &str) -> Result<PathBuf, String> {
    url::Url::parse(resolved)
        .ok()
        .filter(|url| url.scheme() == "file")
        .and_then(|url| url.to_file_path().ok())
        .ok_or_else(|| format!("{resolved:?} is not \"\", a file:// URL or an s3:// object"))
}

/// Compares the metadata file against `wanted_text` and `base`'s; see
/// [`read`]. Decided, `""` takes the revision's file, since a read fills a
/// missing one, and a `file://` URL that file's text.
async fn plan_metadata(
    storage: &(impl Storage + Sync),
    folder: &Path,
    wanted_text: String,
    base: Option<&Manifest>,
    decided: &BTreeMap<LogicalKey, Row>,
    plan: &mut Plan<'_>,
) -> Result<(), EntryError> {
    let metadata_path = folder.join(METADATA_FILE);
    // A base TOML cannot hold was never written, so no file holds it.
    let had_text = base.and_then(|base| render(base).ok());
    // `None` where something other than a file is there.
    let place = if metadata_place_free(storage, &metadata_path).await {
        Some(bytes_at(storage, &metadata_path).await?)
    } else {
        None
    };
    match &place {
        Some(Some(text)) if text == wanted_text.as_bytes() => return Ok(()),
        Some(Some(_)) if had_text.as_ref() == Some(&wanted_text) => {
            plan.kept.push(PathBuf::from(METADATA_FILE));
            return Ok(());
        }
        Some(None) => {
            plan.metadata = Some((wanted_text, None));
            return Ok(());
        }
        Some(Some(text)) if had_text.as_ref().map(String::as_bytes) == Some(text.as_slice()) => {
            plan.metadata = Some((wanted_text, Some(text.clone())));
            return Ok(());
        }
        _ => {}
    }

    let key = LogicalKey::new(METADATA_FILE).expect("a logical key");
    let physical_key = Some(file_url(&metadata_path)?);
    let current = match &place {
        Some(Some(text)) => text_side(text, physical_key).await?,
        _ => Side {
            physical_key,
            hash: None,
            size: None,
        },
    };
    let row = Row {
        other: Other::Incoming(Some(text_side(wanted_text.as_bytes(), None).await?)),
        current: Some(current),
        resolved: None,
    };
    let Some(resolved) = plan.record(&key, row, decided) else {
        return Ok(());
    };
    let Some(Some(on_disk)) = place else {
        return Err(refused(&key, NOT_A_FILE));
    };
    let text = match source(storage, &resolved, None)
        .await
        .map_err(|why| refused(&key, why))?
    {
        None => wanted_text,
        Some(Source::File(path)) => {
            let bytes = storage
                .read_bytes(&path)
                .await
                .map_err(|err| refused(&key, err))?;
            let text = String::from_utf8(bytes).map_err(|err| refused(&key, err))?;
            MetadataFile::parse(Some(&text)).map_err(|why| refused(&key, why))?;
            text
        }
        Some(_) => return Err(refused(&key, "it takes \"\" or a file:// URL")),
    };
    plan.metadata = Some((text, Some(on_disk)));
    Ok(())
}

/// A row's object, as a row of `conflict.json` describes it.
pub(crate) fn row_side(row: &ManifestRow) -> Side {
    Side {
        physical_key: Some(row.physical_key.clone()),
        hash: Some(row.hash.clone()),
        size: Some(row.size),
    }
}

/// The text `text`, at `physical_key` where it is a file, as a row of
/// `conflict.json` describes it.
pub(crate) async fn text_side(text: &[u8], physical_key: Option<String>) -> Res<Side> {
    Ok(Side {
        physical_key,
        hash: Some(hash_bytes(text, MULTIHASH_SHA256).await?),
        size: Some(text.len() as u64),
    })
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

/// Whether `base` covers a key under `paths` and `folder` is missing or
/// empty: no file or link under `paths`, empty folders and what Quilt writes
/// there aside. A folder that is there and cannot be listed is an error, not
/// a folder gone.
pub(crate) async fn gone(
    storage: &(impl Storage + Sync),
    folder: &Path,
    base: &Manifest,
    paths: &Paths,
) -> Res<bool> {
    let had = covered(base, paths);
    if had.is_empty() {
        return Ok(false);
    }
    let mut queue = VecDeque::from([(folder.to_path_buf(), PathBuf::new())]);
    while let Some((dir, prefix)) = queue.pop_front() {
        let root = prefix.as_os_str().is_empty();
        let mut listing = match storage.read_dir(&dir).await {
            Ok(listing) => listing,
            Err(err) if err.is_not_found() && root => return Ok(true),
            Err(err) if root => return Err(err),
            // Below the root a folder that cannot be listed proves nothing.
            Err(_) => continue,
        };
        while let Some(entry) = listing.next_entry().await? {
            let name = entry.file_name();
            let text = name.to_string_lossy();
            if root && (at_metadata_file(&text) || left_by_read(&text)) {
                continue;
            }
            let relative = prefix.join(&name);
            if entry.file_type().await?.is_dir() {
                queue.push_back((dir.join(&name), relative));
                continue;
            }
            // What an OS leaves in a folder it shows, as `.DS_Store`, is no
            // file of the user's, unless the package holds it.
            if LogicalKey::new(crate::paths::logical_key(&relative)).is_ok_and(|key| {
                paths.covers(&key)
                    && (had.contains_key(&key) || crate::junk::check(&relative).is_none())
            }) {
                return Ok(false);
            }
        }
    }
    Ok(true)
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
    let path = step.key.join_to(folder);
    if storage.is_symlink(&path).await {
        return Ok(false);
    }
    if deletes && step.source.is_some() && step.had.is_some() && !storage.exists(&path).await {
        return Ok(false);
    }
    Ok(match &step.had {
        Some(had) => !edited(storage, &path, had).await?,
        None => !storage.exists(&path).await,
    })
}

/// Writes the metadata file's `text` beside it, then renames it into place
/// where the file still holds `planned_on`.
pub(crate) async fn write_metadata(
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
    revision: &Revision<'_>,
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
    refuse_linked_ancestors(storage, folder, steps.iter().map(|step| step.key)).await?;
    let fetch: Vec<ManifestRow> = steps
        .iter()
        .filter_map(|step| match step.source {
            Some(Source::Revision(row)) => Some(row.clone()),
            _ => None,
        })
        .collect();
    if !fetch.is_empty() {
        let fetched = revision.fetch(storage, remote, fetch, staging).await?;
        report.skipped = fetched.skipped;
        report.hash_mismatch = fetched.hash_mismatch;
    }
    // A revision some of whose bytes are gone cannot be read whole, and a
    // folder holding part of it under the old base would read as local work.
    if !report.skipped.is_empty() || !report.hash_mismatch.is_empty() {
        report.resolved.clear();
        return Ok(false);
    }
    for step in steps {
        let staged = step.key.join_to(staging);
        match &step.source {
            Some(Source::File(path)) => {
                storage
                    .create_dir_all(staged.parent().unwrap_or(staging))
                    .await?;
                storage.copy(path, &staged).await?;
            }
            // S3 refuses a version it does not hold, so these are its bytes.
            Some(Source::Object(uri)) => {
                let object = remote
                    .get_object_stream(revision.origin(), uri)
                    .await
                    .map_err(|err| refused(step.key, err))?;
                storage
                    .create_dir_all(staged.parent().unwrap_or(staging))
                    .await?;
                storage
                    .write_staged_byte_stream(&staged, object.body)
                    .await?;
            }
            Some(Source::Revision(_)) | None => {}
        }
    }
    refuse_linked_ancestors(storage, folder, steps.iter().map(|step| step.key)).await?;
    if let Some((base, paths)) = sync_base
        && gone(storage, folder, base, paths).await?
    {
        return Err(EntryError::FolderGone(folder.to_path_buf()));
    }
    // Every step before the first change, so a concurrent edit stops the read
    // whole, then each again right before its own change.
    for step in steps {
        if !as_planned(storage, folder, step, deletes).await? {
            return Err(EntryError::Changed(step.key.to_path_buf()));
        }
    }
    if let Some((_, planned_on)) = metadata
        && !metadata_as_planned(storage, folder, planned_on.as_deref()).await?
    {
        return Err(EntryError::Changed(PathBuf::from(METADATA_FILE)));
    }
    for step in steps {
        let key = step.key;
        let path = key.join_to(folder);
        if !as_planned(storage, folder, step, deletes).await? {
            return Err(EntryError::Changed(key.to_path_buf()));
        }
        refuse_linked_ancestors(storage, folder, std::iter::once(key)).await?;
        if step.source.is_some() {
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
        crate::failpoint::stop("read.file")?;
    }
    Ok(true)
}

/// Copies `rows` from `objects`, the local registry's or a store's, into
/// `staging`, as [`install_to`] fetches a bucket's: each copy is checked
/// against its row's hash, and a row whose object is gone, or whose copy does
/// not hash to it, is reported and left out. Two keys that name one local
/// file are refused before the first copy.
async fn copy_from(
    storage: &(impl Storage + Sync),
    objects: &Path,
    rows: &[ManifestRow],
    staging: &Path,
) -> Res<InstallToReport> {
    refuse_clashing_local_paths(rows)?;
    let mut report = InstallToReport::default();
    for row in rows {
        let key = row.logical_key.to_path_buf();
        let mut object = None;
        for name in row_objects(&row.hash, &row.physical_key) {
            let path = objects.join(name);
            if storage.is_file(&path).await {
                object = Some(path);
                break;
            }
        }
        let Some(object) = object else {
            report.skipped.push(key);
            continue;
        };
        let staged = row.logical_key.join_to(staging);
        storage
            .create_dir_all(staged.parent().unwrap_or(staging))
            .await?;
        storage.copy(&object, &staged).await?;
        if OnDisk::read(storage, &staged, row.hash.algorithm())
            .await?
            .holds(row)
        {
            report.placed.push(key);
        } else {
            storage.remove_file(&staged).await?;
            report.hash_mismatch.push(key);
        }
    }
    Ok(report)
}

/// The revision `hash` of `entry`'s package, in its bucket or in the local
/// registry `local`, to fetch rows from.
async fn revision_at<'a>(
    storage: &(impl Storage + Sync),
    local: Option<&'a LocalRegistry>,
    entry: &'a Entry,
    hash: &str,
) -> Result<Revision<'a>, EntryError> {
    if let Some(registry) = local_registry(storage, local, entry).await? {
        return Ok(Revision::Local {
            registry,
            package: &entry.package,
            hash: hash.to_string(),
        });
    }
    let Registry::S3(bucket) = &entry.registry else {
        return Err(EntryError::LocalRegistry);
    };
    Ok(Revision::Bucket(ManifestUri {
        origin: entry.catalog.clone(),
        bucket: bucket.clone(),
        namespace: entry.package.clone(),
        hash: hash.to_string(),
    }))
}

/// A decision of a publish's row of `conflict.json`, to set in the working
/// folder: `key` as `resolved` says, `destination` being what `latest` holds
/// there.
pub(crate) struct Decision<'a> {
    pub(crate) key: &'a LogicalKey,
    pub(crate) destination: Option<&'a ManifestRow>,
    pub(crate) resolved: &'a str,
}

/// Sets each key of `decisions` in `entry`'s working folder as its decision
/// says, `latest`'s object fetched from its registry: `""` deletes the file,
/// and a `file://` URL or an `s3://` object at its `versionId` puts those
/// bytes there. A decision naming the folder's own file leaves it. Either
/// every decision is set, or none: one that cannot be is refused before
/// anything is changed, and `Ok(false)` says an object's bytes are gone.
pub(crate) async fn set_decided(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    local: Option<&LocalRegistry>,
    entry: &Entry,
    latest: &str,
    decisions: &[Decision<'_>],
) -> Result<bool, EntryError> {
    let folder = &entry.folder;
    let mut steps = Vec::new();
    for decision in decisions {
        let key = decision.key;
        let path = key.join_to(folder);
        let disk = OnDisk::read(storage, &path, MULTIHASH_SHA256).await?;
        let here = disk.side(&path)?.and_then(|side| side.physical_key);
        if here.as_deref() == Some(decision.resolved) {
            continue;
        }
        let had = match disk {
            OnDisk::File(file) => Some(Cow::Owned(ManifestRow {
                logical_key: key.clone(),
                physical_key: String::new(),
                hash: file.0,
                size: file.1,
                meta: None,
                meta_numbers: RawNumbers::default(),
            })),
            OnDisk::Nothing => None,
            OnDisk::NotAFile => return Err(refused(key, NOT_A_FILE)),
        };
        let source = source(storage, decision.resolved, decision.destination)
            .await
            .map_err(|why| refused(key, why))?;
        if source.is_some() || had.is_some() {
            steps.push(Step { key, source, had });
        }
    }
    if steps.is_empty() {
        return Ok(true);
    }
    let revision = revision_at(storage, local, entry, latest).await?;
    let staging = folder.join(format!("{STAGING_PREFIX}{}", uuid::Uuid::new_v4()));
    let mut report = ReadReport::default();
    let applied = apply(
        storage,
        remote,
        &revision,
        folder,
        &staging,
        &steps,
        None,
        &mut report,
        None,
    )
    .await;
    let _ = storage.remove_dir_all(&staging).await;
    applied
}

/// Changes `entry`'s working folder from `from`, what it was taken as, to
/// `to`, the local work merged onto `latest`: each file under the entry's
/// paths that `to` holds other bytes of is placed from `latest`'s registry,
/// and each it no longer holds is removed, as a read of `to` over the base
/// `from` places them, and the metadata file is written as `to`'s where it
/// still holds `planned_on`. A file changed since the folder was taken stops
/// it before anything is placed. The report names `latest`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn apply_merged(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    local: Option<&LocalRegistry>,
    entry: &Entry,
    latest: &str,
    from: &Manifest,
    to: &Manifest,
    planned_on: Option<&[u8]>,
) -> Result<ReadReport, EntryError> {
    let folder = &entry.folder;
    let text = render(to).map_err(EntryError::MetadataUnwritable)?;
    let paths = &entry.paths;
    let mut plan = plan_files(
        storage,
        folder,
        paths,
        to,
        Some(from),
        true,
        &BTreeMap::new(),
    )
    .await?;
    if let Some(key) = plan.conflicts.first() {
        return Err(EntryError::Changed(key.clone()));
    }
    if planned_on != Some(text.as_bytes()) {
        plan.metadata = Some((text, planned_on.map(<[u8]>::to_vec)));
    }
    let revision = revision_at(storage, local, entry, latest).await?;
    let mut report = ReadReport {
        revision: latest.to_string(),
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
        Some((from, paths)),
    )
    .await;
    let _ = storage.remove_dir_all(&staging).await;
    if applied? && let Some((text, planned_on)) = plan.metadata {
        write_metadata(storage, folder, text, planned_on.as_deref()).await?;
        report.placed.push(PathBuf::from(METADATA_FILE));
    }
    Ok(report)
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
