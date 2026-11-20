//! The tick's merge: where `latest` moved past the base of a package's local
//! commits, the local change, its commits and what the folder holds beyond
//! them, merged onto `latest` key by key, as the ref tick of `QuiltSync`
//! merges a diverged copy.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::ops::ControlFlow;
use std::path::Path;
use std::path::PathBuf;

use tracing::info;
use tracing::warn;

use super::EntrySync;
use super::LockFile;
use super::Stop;
use super::Taken;
use super::base_manifest;
use super::commit_workflow;
use super::take;
use super::take_changed;
use crate::LogicalKey;
use crate::checksum::hash_file;
use crate::dataspace::EntryError;
use crate::dataspace::METADATA_FILE;
use crate::dataspace::conflict::Other;
use crate::dataspace::conflict::PackageRows;
use crate::dataspace::conflict::Row;
use crate::dataspace::conflict::Side;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::Entry;
use crate::dataspace::hooks;
use crate::dataspace::hooks::HookPoint;
use crate::dataspace::lock::Lock;
use crate::dataspace::lock::LockEntry;
use crate::dataspace::metadata::MetadataFile;
use crate::dataspace::metadata::holds_metadata_file;
use crate::dataspace::metadata::render;
use crate::dataspace::publish::same_as;
use crate::dataspace::read::Decision;
use crate::dataspace::read::OnDisk;
use crate::dataspace::read::ReadReport;
use crate::dataspace::read::apply_merged;
use crate::dataspace::read::row_side;
use crate::dataspace::read::set_decided;
use crate::dataspace::read::text_side;
use crate::dataspace::read::write_metadata;
use crate::dataspace::store::Store;
use crate::flow::MergeDirMetaError;
use crate::flow::MergeRefusal;
use crate::flow::WorkflowCheck;
use crate::flow::merge_dir_meta;
use crate::flow::merge_onto;
use crate::flow::merge_rows_onto;
use crate::flow::record;
use crate::io::manifest::RowContent;
use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;
use crate::lineage::PackageLineage;
use crate::manifest::DirMeta;
use crate::manifest::HeaderMessage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::paths::file_url;
use crate::quiltignore::Rules;

/// What the tick merges: the lock's base, `None` for a package never
/// published, and `current`, the manifest of its last commit.
pub(super) struct Local<'a> {
    pub(super) base: Option<&'a str>,
    pub(super) current: &'a Manifest,
}

/// Merges the local change onto `latest`: from the lock's base, or from the
/// empty revision for a package never published, to its last commit, with
/// what the working folder holds beyond that commit. Keys only one side
/// changed take that side's change, and the folder takes `latest`'s; a key
/// both changed alike is no collision. Where they collide, nothing is
/// applied, and `conflict.json` holds a publish's row for each colliding key
/// and one for `metadata.toml` where both changed metadata differently; a
/// later tick applies the decisions there, every one or none.
///
/// The merge is recorded as one commit on `latest`, squashing the lock's
/// commits, with the last one's message, or `message` where the folder held
/// more; its workflow is the one a commit takes. The folder is brought to it,
/// and only then does the lock name it, based at `latest`, for the tick to
/// push as it pushes any commit. Where the merge is `latest` as it was,
/// nothing is recorded and the lock is based at `latest`.
///
/// Continues with what the folder took from `latest`; breaks with how the
/// tick ends where the merge collides, or where some of `latest`'s bytes are
/// gone, the folder and the lock left as they were.
#[allow(clippy::too_many_arguments)]
pub(super) async fn onto_latest(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    store: &Store,
    entry: &Entry,
    lock: &mut Lock,
    lock_file: &LockFile<'_, impl Storage + Sync>,
    ours: Local<'_>,
    latest: &str,
    message: &str,
    user_rules: &Path,
) -> Result<ControlFlow<EntrySync, ReadReport>, Stop> {
    let contract = lock_file.contract;
    hooks::run(contract, HookPoint::PreRead, entry, None)
        .await
        .map_err(EntryError::from)?;
    let base = match ours.base {
        Some(hash) => base_manifest(storage, remote, store, entry, hash).await?,
        None => Manifest::default(),
    };
    let theirs = base_manifest(storage, remote, store, entry, latest).await?;
    if holds_metadata_file(&theirs) {
        return Err(EntryError::HoldsMetadataFile.into());
    }
    let rows = PackageRows::of(storage, store.local(), contract.dataspace(), entry).await;
    let saved = rows.saved(storage).await?;
    let merge = Merge {
        storage,
        remote,
        store,
        entry,
        latest,
        base,
        theirs,
        rows,
        saved,
    };
    let settled = match merge.settle(contract, ours.current, user_rules).await? {
        ControlFlow::Continue(settled) => settled,
        ControlFlow::Break(outcome) => return Ok(ControlFlow::Break(outcome)),
    };
    let target = merge.record(&settled, message).await?;
    let to = target
        .as_ref()
        .map_or(&merge.theirs, |(_, manifest)| manifest);
    let report = apply_merged(
        storage,
        remote,
        store.local(),
        entry,
        latest,
        &settled.folder,
        to,
        settled.taken.metadata_text.as_deref(),
    )
    .await?;
    if !report.skipped.is_empty() || !report.hash_mismatch.is_empty() {
        return Ok(ControlFlow::Break(unread(report)));
    }
    let locked = match target {
        Some((hash, _)) => LockEntry {
            base: Some(latest.to_string()),
            commits: vec![hash],
        },
        None => LockEntry::at(latest),
    };
    lock.packages.insert(entry.key(), locked);
    lock_file.save(lock).await?;
    if !merge.saved.is_empty()
        && let Err(err) = merge.rows.write(storage, None).await
    {
        warn!("The merge completed, and conflict.json still holds its rows: {err}");
    }
    // The base moved to `latest`, which moved past the base.
    hooks::after(contract, HookPoint::PostRead, entry, latest).await;
    Ok(ControlFlow::Continue(report))
}

fn unread(report: ReadReport) -> EntrySync {
    EntrySync::Ran {
        read: Some(report),
        published: None,
    }
}

/// One merge onto `latest`: the base, `latest`'s manifest, `theirs`, and
/// the package's rows of `conflict.json`, as they were read before it.
struct Merge<'a, S, R> {
    storage: &'a S,
    remote: &'a R,
    store: &'a Store,
    entry: &'a Entry,
    latest: &'a str,
    base: Manifest,
    theirs: Manifest,
    rows: PackageRows<'a>,
    saved: BTreeMap<LogicalKey, Row>,
}

/// The folder settled for the merge: taken as a commit takes it, whether it
/// changed from the last commit, hashed, and the merge itself.
struct Settled {
    taken: Taken,
    changed: bool,
    folder: Manifest,
    merged: Manifest,
}

impl<S: Storage + Sync, R: Remote> Merge<'_, S, R> {
    /// Compares the folder, as it holds the local change, against `latest`
    /// before anything is committed; where every collision is decided, sets
    /// the decisions in the folder, takes it as a commit takes it, and
    /// merges.
    async fn settle(
        &self,
        contract: &Contract,
        current: &Manifest,
        user_rules: &Path,
    ) -> Result<ControlFlow<EntrySync, Settled>, Stop> {
        let (storage, entry, theirs) = (self.storage, self.entry, &self.theirs);
        let rules = Rules::load(&entry.folder, user_rules)?;
        let taken = take(storage, entry, &rules, current).await?;
        let found = Collisions::of(&self.base, &hashed(storage, &taken).await?, theirs)?;
        let decided = found
            .rows(storage, entry, &taken, theirs, &self.saved)
            .await?;
        if decided.values().any(|row| row.resolved.is_none()) {
            return Ok(ControlFlow::Break(self.blocked(decided).await?));
        }
        let decisions = Decided::of(storage, &found, &decided, theirs).await?;
        let local = self.store.local();
        let folder = decisions.in_folder();
        if !set_decided(storage, self.remote, local, entry, self.latest, &folder).await? {
            return Ok(ControlFlow::Break(unread(ReadReport {
                revision: self.latest.to_string(),
                ..ReadReport::default()
            })));
        }
        if let Some(text) = &decisions.metadata_text {
            let planned_on = taken.metadata_text.as_deref();
            write_metadata(storage, &entry.folder, text.clone(), planned_on).await?;
        }

        // Taken again as a commit takes it, the decisions in it.
        let (taken, changed) =
            take_changed(storage, contract, entry, user_rules, current, Some(current)).await?;
        let mut folder = hashed(storage, &taken).await?;
        let (mut was, mut onto) = (self.base.clone(), theirs.clone());
        decisions.settle(&found, &mut was, &mut folder, &mut onto);
        let merged = match merge_onto(&was, &folder, onto) {
            Ok(merged) => merged,
            Err(MergeRefusal::Invalid(reason)) => {
                return Err(EntryError::Unmergeable(reason).into());
            }
            // The folder changed since it was compared, or its decisions do
            // not settle what it collides on now.
            Err(refusal) => {
                let found = Collisions::of(&self.base, &hashed(storage, &taken).await?, theirs)?;
                let rows = found
                    .rows(storage, entry, &taken, theirs, &self.saved)
                    .await?;
                if rows.values().all(|row| row.resolved.is_some()) {
                    return Err(EntryError::Unmergeable(format!("{refusal:?}")).into());
                }
                return Ok(ControlFlow::Break(self.blocked(rows).await?));
            }
        };
        Ok(ControlFlow::Continue(Settled {
            taken,
            changed,
            folder,
            merged,
        }))
    }

    /// Records the merge as a commit on `latest`, `None` where it is
    /// `latest` as it was: the hash, and the manifest as recorded.
    async fn record(
        &self,
        settled: &Settled,
        message: &str,
    ) -> Result<Option<(String, Manifest)>, Stop> {
        let (storage, store, entry) = (self.storage, self.store, self.entry);
        let mut merged = settled.merged.clone();
        if unchanged(&merged, &self.theirs) {
            info!("latest {} already holds the local change", self.latest);
            return Ok(None);
        }
        if settled.changed {
            merged.header.message = HeaderMessage::Text(message.to_string());
        }
        merged.header.workflow = commit_workflow(self.remote, store, entry).await?;
        // The content of each row the folder changed that the merge took.
        let content: BTreeMap<LogicalKey, RowContent> = settled
            .taken
            .content
            .iter()
            .filter(|(key, _)| {
                merged
                    .rows
                    .iter()
                    .any(|row| row.logical_key == **key && row.physical_key.is_empty())
            })
            .map(|(key, content)| (key.clone(), content.clone()))
            .collect();
        let lineage = PackageLineage {
            base_hash: self.latest.to_string(),
            ..PackageLineage::default()
        };
        let (_, recorded) = record(
            store,
            storage,
            self.remote,
            // As a commit's: the push checks the revision.
            WorkflowCheck::Warn {
                host: entry.catalog.as_ref(),
            },
            &entry.package,
            lineage,
            merged,
            &content,
        )
        .await?;
        let Some(hash) = recorded.hash() else {
            return Ok(None);
        };
        let installed = store.installed_manifest(&entry.package, hash);
        let manifest = Manifest::from_path(storage, &installed).await?;
        Ok(Some((hash.to_string(), manifest)))
    }

    /// Writes `rows` for the package, and says the merge is blocked by those
    /// still undecided.
    async fn blocked(&self, rows: BTreeMap<LogicalKey, Row>) -> Result<EntrySync, EntryError> {
        let conflicts = rows
            .iter()
            .filter(|(_, row)| row.resolved.is_none())
            .map(|(key, _)| key.to_path_buf())
            .collect();
        // Nothing was applied, and without the map nothing can be decided.
        self.rows.write(self.storage, Some(rows)).await?;
        Ok(EntrySync::Blocked {
            revision: self.latest.to_string(),
            conflicts,
        })
    }
}

/// `taken`'s manifest with each row a file changed or added hashed as a
/// commit hashes it, so it can be compared before anything is committed.
async fn hashed(storage: &(impl Storage + Sync), taken: &Taken) -> Result<Manifest, EntryError> {
    let algorithm = HostConfig::default().checksums.algorithm_code();
    let mut manifest = taken.manifest.clone();
    for row in &mut manifest.rows {
        if let Some(RowContent::Path(path)) = taken.content.get(&row.logical_key) {
            (row.hash, row.size) = hash_file(storage, path, algorithm).await?;
        }
    }
    Ok(manifest)
}

/// Whether merging left `theirs` as it was, so there is nothing to commit:
/// the same files, metadata and folder metadata. The merged side holds only
/// folder metadata that says something, sorted, and `theirs` is compared in
/// that form.
fn unchanged(merged: &Manifest, theirs: &Manifest) -> bool {
    let mut theirs = theirs.clone();
    theirs.dir_meta.retain(DirMeta::holds_meta);
    theirs
        .dir_meta
        .sort_by(|a, b| a.logical_key.cmp(&b.logical_key));
    same_as(merged, Some(&theirs), false)
}

/// What collides between the local change, `base` to `ours`, and `latest`'s,
/// `base` to `theirs`.
#[derive(Debug, Default)]
struct Collisions {
    /// Keys both changed to other bytes, or that a file and a folder of the
    /// other side's take.
    files: BTreeSet<LogicalKey>,
    /// Files both hold the same bytes of, with other metadata.
    entries: BTreeSet<LogicalKey>,
    /// Folders both gave other metadata.
    folders: BTreeSet<LogicalKey>,
    /// Whether both changed the package's metadata differently.
    package: bool,
}

impl Collisions {
    fn of(base: &Manifest, ours: &Manifest, theirs: &Manifest) -> Result<Self, EntryError> {
        let mut found = Self::default();
        let rows = match merge_rows_onto(&base.rows, &ours.rows, theirs.rows.clone()) {
            Ok(rows) => rows,
            Err(paths) => {
                found.add(paths, ours, theirs)?;
                // Folders are asked about apart from the files.
                Vec::new()
            }
        };
        match merge_dir_meta(&base.dir_meta, &ours.dir_meta, &theirs.dir_meta, &rows) {
            Ok(_) => {}
            Err(MergeDirMetaError::Paths(paths)) => found.add(paths, ours, theirs)?,
            Err(MergeDirMetaError::Invalid(err)) => return Err(err.into()),
        }
        let header = |manifest: &Manifest| Manifest {
            header: manifest.header.clone(),
            ..Manifest::default()
        };
        found.package = matches!(
            merge_onto(&header(base), &header(ours), header(theirs)),
            Err(MergeRefusal::UserMeta)
        );
        Ok(found)
    }

    /// Adds `paths` a merge refused: a folder's, ending in `/`, or a file's.
    fn add(
        &mut self,
        paths: Vec<PathBuf>,
        ours: &Manifest,
        theirs: &Manifest,
    ) -> Result<(), EntryError> {
        for path in paths {
            let text = path.to_string_lossy();
            if let Some(folder) = text.strip_suffix('/') {
                self.folders
                    .insert(LogicalKey::new(folder).map_err(crate::Error::from)?);
                continue;
            }
            let key = LogicalKey::new(path).map_err(crate::Error::from)?;
            let bytes = |manifest: &Manifest| {
                row_of(manifest, &key).map(|row| (row.hash.clone(), row.size))
            };
            if bytes(ours).is_some() && bytes(ours) == bytes(theirs) {
                self.entries.insert(key);
            } else {
                self.files.insert(key);
            }
        }
        Ok(())
    }

    fn metadata(&self) -> bool {
        self.package || !self.entries.is_empty() || !self.folders.is_empty()
    }

    /// The publish's rows of `conflict.json`: one for each colliding file and
    /// one for `metadata.toml` where metadata collides, `current` being what
    /// the folder `taken` from holds, and `destination` what `theirs`,
    /// `latest`, holds. Each takes the decision of its row in `saved` that
    /// still describes it.
    async fn rows(
        &self,
        storage: &(impl Storage + Sync),
        entry: &Entry,
        taken: &Taken,
        theirs: &Manifest,
        saved: &BTreeMap<LogicalKey, Row>,
    ) -> Result<BTreeMap<LogicalKey, Row>, EntryError> {
        let algorithm = HostConfig::default().checksums.algorithm_code();
        let mut rows = BTreeMap::new();
        for key in &self.files {
            let current = if entry.paths.covers(key) {
                let path = key.join_to(&entry.folder);
                OnDisk::read(storage, &path, algorithm).await?.side(&path)?
            } else {
                row_of(&taken.manifest, key).map(row_side)
            };
            let row = Row {
                other: Other::Destination(row_of(theirs, key).map(row_side)),
                current,
                resolved: None,
            };
            rows.insert(key.clone(), row);
        }
        if self.metadata() {
            let path = entry.folder.join(METADATA_FILE);
            let physical_key = Some(file_url(&path)?);
            let current = match &taken.metadata_text {
                Some(text) => text_side(text, physical_key).await?,
                None => Side {
                    physical_key,
                    hash: None,
                    size: None,
                },
            };
            let text = render(theirs).map_err(EntryError::MetadataUnwritable)?;
            let row = Row {
                other: Other::Destination(Some(text_side(text.as_bytes(), None).await?)),
                current: Some(current),
                resolved: None,
            };
            rows.insert(metadata_key(), row);
        }
        for (key, row) in &mut rows {
            let here = |row: &Row| row.current.as_ref()?.physical_key.clone();
            row.resolved = saved
                .get(key)
                .filter(|earlier| earlier.same_collision(row))
                .and_then(|earlier| match &earlier.resolved {
                    // The folder's own file, wherever its path was spelled then.
                    Some(resolved) if Some(resolved) == here(earlier).as_ref() => here(row),
                    resolved => resolved.clone(),
                });
        }
        Ok(rows)
    }
}

fn metadata_key() -> LogicalKey {
    LogicalKey::new(METADATA_FILE).expect("a logical key")
}

fn row_of<'a>(manifest: &'a Manifest, key: &LogicalKey) -> Option<&'a ManifestRow> {
    manifest.rows.iter().find(|row| row.logical_key == *key)
}

/// Which side a decided key takes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Taking {
    /// `latest`'s, as the destination is.
    Theirs,
    /// The folder's, once the decision is set in it.
    Ours,
}

/// The decisions on every row, each checked and sorted by what it does.
struct Decided<'a> {
    /// Each colliding file's side.
    files: BTreeMap<&'a LogicalKey, Taking>,
    /// The decisions to set in the folder first, each file's taking it.
    folder: Vec<(&'a LogicalKey, Option<&'a ManifestRow>, &'a str)>,
    /// The side the colliding metadata takes.
    metadata: Option<Taking>,
    /// A metadata file a decision names other than the folder's own, its text
    /// to put in the folder.
    metadata_text: Option<String>,
}

impl<'a> Decided<'a> {
    async fn of(
        storage: &(impl Storage + Sync),
        found: &'a Collisions,
        rows: &'a BTreeMap<LogicalKey, Row>,
        theirs: &'a Manifest,
    ) -> Result<Self, EntryError> {
        let mut decided = Self {
            files: BTreeMap::new(),
            folder: Vec::new(),
            metadata: None,
            metadata_text: None,
        };
        for (key, row) in rows {
            let Some(resolved) = row.resolved.as_deref() else {
                continue;
            };
            let here = row
                .current
                .as_ref()
                .and_then(|side| side.physical_key.as_deref());
            if *key == metadata_key() && found.metadata() {
                decided.metadata = Some(if resolved.is_empty() {
                    // `""` takes the revision's metadata, as a read's does.
                    Taking::Theirs
                } else if Some(resolved) == here {
                    Taking::Ours
                } else {
                    decided.metadata_text = Some(metadata_text(storage, resolved).await?);
                    Taking::Ours
                });
                continue;
            }
            let taking = if Some(resolved) == row.others_key() {
                Taking::Theirs
            } else {
                // Anything else is the folder's, once it holds it.
                decided.folder.push((key, row_of(theirs, key), resolved));
                Taking::Ours
            };
            decided.files.insert(key, taking);
        }
        Ok(decided)
    }

    fn in_folder(&self) -> Vec<Decision<'a>> {
        self.folder
            .iter()
            .map(|(key, destination, resolved)| Decision {
                key,
                destination: *destination,
                resolved,
            })
            .collect()
    }

    /// Settles each decided collision in the merge's sides, so that the
    /// merge takes the side decided: `base`, `was`, takes the other side's
    /// version of a file, so only the side taken changed it; colliding
    /// metadata is made the side taken's on the other side.
    fn settle(
        &self,
        found: &Collisions,
        was: &mut Manifest,
        ours: &mut Manifest,
        theirs: &mut Manifest,
    ) {
        for (key, taking) in &self.files {
            let other = match taking {
                Taking::Theirs => row_of(ours, key),
                Taking::Ours => row_of(theirs, key),
            };
            set_row(was, key, other.cloned());
        }
        let (from, to) = match self.metadata {
            Some(Taking::Theirs) => (&*theirs, ours),
            Some(Taking::Ours) => (&*ours, theirs),
            None => return,
        };
        let from = from.clone();
        if found.package {
            to.header.user_meta.clone_from(&from.header.user_meta);
            to.header
                .user_meta_numbers
                .clone_from(&from.header.user_meta_numbers);
        }
        for key in &found.entries {
            if let (Some(from), Some(row)) = (
                row_of(&from, key),
                to.rows.iter_mut().find(|row| row.logical_key == *key),
            ) {
                row.meta.clone_from(&from.meta);
                row.meta_numbers.clone_from(&from.meta_numbers);
            }
        }
        for key in &found.folders {
            to.dir_meta.retain(|dir| dir.logical_key != *key);
            if let Some(dir) = from.dir_meta.iter().find(|dir| dir.logical_key == *key) {
                to.dir_meta.push(dir.clone());
            }
        }
    }
}

/// `manifest` with `row` at `key`, or none there.
fn set_row(manifest: &mut Manifest, key: &LogicalKey, row: Option<ManifestRow>) {
    manifest.rows.retain(|row| row.logical_key != *key);
    manifest.rows.extend(row);
}

/// The text of the metadata file a decision names: a `file://` URL.
async fn metadata_text(
    storage: &(impl Storage + Sync),
    resolved: &str,
) -> Result<String, EntryError> {
    let refused =
        |why: String| EntryError::ConflictMap(format!("{METADATA_FILE} cannot be set: {why}"));
    let path = url::Url::parse(resolved)
        .ok()
        .filter(|url| url.scheme() == "file")
        .and_then(|url| url.to_file_path().ok())
        .ok_or_else(|| refused(format!("{resolved:?} is not \"\" or a file:// URL")))?;
    let bytes = storage
        .read_bytes(&path)
        .await
        .map_err(|err| refused(err.to_string()))?;
    let text = String::from_utf8(bytes).map_err(|err| refused(err.to_string()))?;
    MetadataFile::parse(Some(&text)).map_err(refused)?;
    Ok(text)
}
