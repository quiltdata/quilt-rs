//! `keep`: what a dataspace keeps of a package, changed in `data.toml`
//! because the user asked (25.3). Today's tracked set and stored sync scope
//! go (25.16): an entry's scope is its paths, and what it keeps is its
//! access and its pin. So narrowing or widening the paths, pinning or
//! unpinning a revision, and moving the package to another list are each an
//! edit of its declarations, and paths newly read are read before their
//! absence counts (25.7 rule 4).

use std::path::Path;

use thiserror::Error;
use tracing::info;

use crate::dataspace::EntryError;
use crate::dataspace::conflict::read_resolving;
use crate::dataspace::contract::Access;
use crate::dataspace::contract::CONTRACT_FILE;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::EntryProblem;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::contract::PackagePath;
use crate::dataspace::contract::Paths;
use crate::dataspace::contract::parse_revision;
use crate::dataspace::edit::ContractEdit;
use crate::dataspace::edit::declaration;
use crate::dataspace::install::write_lock;
use crate::dataspace::lock::LOCK_FILE;
use crate::dataspace::lock::LockEntry;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Mismatch;
use crate::dataspace::lock::Writer;
use crate::dataspace::read::ReadOutcome;
use crate::dataspace::read::revision_manifest;
use crate::dataspace::read_all::holds_files;
use crate::dataspace::read_all::usable_lock;
use crate::dataspace::store::current_key;
use crate::dataspace::store::name_local;
use crate::domain::LocalRegistry;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use quilt_uri::RevisionPointer;
use quilt_uri::Tag;

/// The paths an entry is to cover.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Scope {
    /// The whole package, files added later included.
    Whole,
    /// These paths, as a contract spells them: a folder ends in `/`.
    Only(Vec<String>),
}

/// The revision an entry is to read.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Pin {
    /// This one, as a contract's `revision` spells it: a hash, a prefix of
    /// one, or a tag after `:`. Only a `read` entry takes one.
    At(String),
    /// `latest`, as an unpinned entry reads.
    Latest,
}

/// What to change in what a dataspace keeps of a package. A change left
/// `None` keeps what the contract says; with none, nothing changes.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Keep {
    /// The package, under the local registry's current name or one it had.
    pub package: PackageKey,
    /// The list whose entry changes, where the package is in more than one,
    /// as a package with `read` and `write` paths is.
    pub from: Option<Access>,
    pub paths: Option<Scope>,
    pub revision: Option<Pin>,
    /// The list the entry moves to.
    pub access: Option<Access>,
}

impl Keep {
    /// No change to `package`: what [`keep`] then gives is the entry as the
    /// contract has it.
    #[must_use]
    pub fn new(package: PackageKey) -> Self {
        Self {
            package,
            from: None,
            paths: None,
            revision: None,
            access: None,
        }
    }
}

/// How the read of the paths the change newly read ended.
#[derive(Debug)]
#[non_exhaustive]
pub enum KeepRead {
    /// Nothing was to be read: the entry is not read, the change read no
    /// path anew, the lock names no revision the folder holds, so the next
    /// read fills it whole, or the revision has no file under the new paths.
    Nothing,
    /// The new paths were read at the lock's base. The lock was written
    /// where [`ReadOutcome::lock_base`] names one.
    Ran(ReadOutcome),
    /// The lock names local commits of the package not yet pushed, so the
    /// new paths were not read; the lock was left as it was.
    Unpushed,
    /// The read failed; the lock was left as it was.
    Failed(EntryError),
}

impl KeepRead {
    /// Whether the change completed, so the lock was written.
    #[must_use]
    pub fn completed(&self) -> bool {
        match self {
            Self::Nothing => true,
            Self::Ran(outcome) => outcome.lock_base().is_some(),
            Self::Unpushed | Self::Failed(_) => false,
        }
    }
}

/// What [`keep`] did.
#[derive(Debug)]
#[non_exhaustive]
pub struct Kept {
    /// The package's entry, as the contract now expands it.
    pub entry: Entry,
    /// Whether `data.toml` was written: false where the change asked for
    /// what the contract already said.
    pub declared: bool,
    pub read: KeepRead,
}

/// Why nothing was changed. The contract and the lock are as they were.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum KeepError {
    #[error(transparent)]
    Contract(Box<ContractError>),

    #[error(transparent)]
    Lock(Box<LockError>),

    #[error(
        "{LOCK_FILE} was written for another {}; remove the lock, or fix it, to change what is kept",
        .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    Mismatch(Vec<Mismatch>),

    #[error("{CONTRACT_FILE} does not declare {0}")]
    NotDeclared(Box<PackageKey>),

    #[error("{package} is not under `{access}` in {CONTRACT_FILE}")]
    NotUnder {
        package: Box<PackageKey>,
        access: Access,
    },

    #[error(
        "{package} is under {} in {CONTRACT_FILE}; name the one to change",
        .accesses.iter().map(|access| format!("`{access}`")).collect::<Vec<_>>().join(" and ")
    )]
    Ambiguous {
        package: Box<PackageKey>,
        accesses: Vec<Access>,
    },

    #[error("{path:?} is not a path to keep: {problem}")]
    Path { path: String, problem: EntryProblem },

    #[error("the paths given overlap, one as a file and one as a folder")]
    Paths,

    #[error(transparent)]
    Revision(EntryProblem),

    #[error(transparent)]
    Failed(Box<crate::Error>),
}

impl From<ContractError> for KeepError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<LockError> for KeepError {
    fn from(err: LockError) -> Self {
        Self::Lock(Box::new(err))
    }
}

impl From<crate::Error> for KeepError {
    fn from(err: crate::Error) -> Self {
        Self::Failed(Box::new(err))
    }
}

/// Changes what the dataspace in the folder `dataspace` keeps of
/// `request.package`, as `request` asks: its entry's paths, its pin, its
/// list. Each is an edit of the package's declarations in `data.toml`, which
/// keeps the file's comments, writes the package where its declarations
/// were, and is refused where the contract would refuse the result, as a
/// pin outside `read` or a path under `replace`. `data.toml` is written
/// first and the lock last.
///
/// Where the entry is read and the change reads a file it did not before,
/// widening its paths or moving it to `read` or `sync`, those files are read
/// at the lock's base before anything else, so their absence is not taken
/// for deletions (25.7 rule 4): only the files the base has that the entry
/// did not cover, so a file deleted under the old paths stays deleted. A
/// folder that holds nothing, or a package the lock names no base for, is
/// read whole by the next read instead. A package whose lock entry names
/// unpushed commits is not read.
///
/// Narrowing the paths, or moving the entry to `write` or `replace`, leaves
/// the files in the folder: they are the user's, and no longer the entry's.
/// A pin takes effect at the next read.
///
/// The lock is written, naming the new contract where it matched the old
/// one, only once the change completed; a read that is blocked or fails
/// leaves the edit in `data.toml` and the lock as it was, as an install
/// does. The lock is used as [`read_all`](crate::dataspace::read_all::read_all)
/// uses it, and a local package's files are read from `local`.
pub async fn keep(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    local: Option<&LocalRegistry>,
    dataspace: &Path,
    writer: &Writer,
    request: &Keep,
) -> Result<Kept, KeepError> {
    // Read whole first: a missing `data.toml` is refused, not made.
    Contract::read(storage, dataspace, writer.local_registry_name()).await?;
    let mut edit = ContractEdit::open(storage, dataspace).await?;
    let mut contract = edit.original(writer.local_registry_name())?;
    let (mut lock, mut writer) = usable_lock(storage, &contract, writer)
        .await?
        .map_err(KeepError::Mismatch)?;
    let matched = lock.header.data_toml_hash == contract.data_toml_hash();
    name_local(storage, local, &mut contract, &mut lock, &mut writer).await?;
    let key = current_key(storage, local, &request.package).await?;

    let entry = chosen(&contract, &key, request.from)?;
    let wanted = wanted(&entry, request)?;
    if wanted == entry {
        return Ok(Kept {
            entry,
            declared: false,
            read: KeepRead::Nothing,
        });
    }
    let places: Vec<(Access, usize)> = contract
        .declared()
        .iter()
        .filter(|(access, _, declared)| *access == entry.access && *declared == key)
        .map(|(access, index, _)| (*access, *index))
        .collect();
    let declarations = match &wanted.paths {
        Paths::Whole => vec![declaration(&contract, &wanted, None)],
        Paths::Only(paths) => paths
            .iter()
            .map(|path| declaration(&contract, &wanted, Some(path)))
            .collect(),
    }
    .into_iter()
    .collect::<Result<Vec<_>, _>>()
    .map_err(|problem| ContractError::Edit(problem.to_string()))?;
    edit.redeclare(&places, wanted.access, declarations);
    let mut edited = edit.edited(writer.local_registry_name())?;
    name_local(storage, local, &mut edited, &mut lock, &mut writer).await?;
    let entry = edited
        .entries()
        .iter()
        .find(|entry| entry.key() == key && entry.access == wanted.access)
        .cloned()
        .ok_or_else(|| ContractError::Edit(format!("{key} was not declared")))?;
    let declared = edit.changed();
    if declared {
        edit.write(storage).await?;
    }

    let read = read_added(
        storage,
        remote,
        local,
        &edited,
        &entry,
        read_paths(&contract, &key),
        lock.packages.get(&key),
    )
    .await;
    if declared && read.completed() {
        write_lock(storage, &mut lock, &edited, &writer, matched).await?;
    }
    info!(
        "✔️ {key} keeps {} under `{}`",
        match &entry.paths {
            Paths::Whole => "the whole package".to_string(),
            Paths::Only(paths) => paths
                .iter()
                .map(PackagePath::spelled)
                .collect::<Vec<_>>()
                .join(", "),
        },
        entry.access
    );
    Ok(Kept {
        entry,
        declared,
        read,
    })
}

/// `key`'s entry in `contract` under `from`, or under its only list.
fn chosen(contract: &Contract, key: &PackageKey, from: Option<Access>) -> Result<Entry, KeepError> {
    let entries: Vec<&Entry> = contract
        .entries()
        .iter()
        .filter(|entry| entry.key() == *key)
        .collect();
    let package = || Box::new(key.clone());
    match (entries.as_slice(), from) {
        ([], _) => Err(KeepError::NotDeclared(package())),
        ([only], None) => Ok((*only).clone()),
        (_, None) => Err(KeepError::Ambiguous {
            package: package(),
            accesses: entries.iter().map(|entry| entry.access).collect(),
        }),
        (_, Some(access)) => entries
            .iter()
            .find(|entry| entry.access == access)
            .map(|entry| (*entry).clone())
            .ok_or(KeepError::NotUnder {
                package: package(),
                access,
            }),
    }
}

/// `entry` as `request` would have it.
fn wanted(entry: &Entry, request: &Keep) -> Result<Entry, KeepError> {
    let mut wanted = entry.clone();
    match &request.paths {
        None => {}
        Some(Scope::Whole) => wanted.paths = Paths::Whole,
        Some(Scope::Only(texts)) => {
            let mut paths = Vec::new();
            for text in texts {
                paths.push(PackagePath::parse(text).map_err(|problem| KeepError::Path {
                    path: text.clone(),
                    problem,
                })?);
            }
            let declared: Vec<Option<&PackagePath>> = paths.iter().map(Some).collect();
            wanted.paths = match Paths::union(&declared) {
                // No path at all is no scope to keep.
                Some(Paths::Only(paths)) if paths.is_empty() => Paths::Whole,
                Some(paths) => paths,
                None => return Err(KeepError::Paths),
            };
        }
    }
    match &request.revision {
        None => {}
        Some(Pin::Latest) => wanted.revision = RevisionPointer::Tag(Tag::Latest),
        Some(Pin::At(text)) => {
            wanted.revision = parse_revision(text).map_err(KeepError::Revision)?;
        }
    }
    if let Some(access) = request.access {
        wanted.access = access;
    }
    Ok(wanted)
}

/// The paths `contract` reads of `key`: those of its `read` or `sync` entry,
/// or `None` where it has neither.
fn read_paths(contract: &Contract, key: &PackageKey) -> Option<Paths> {
    contract
        .entries()
        .iter()
        .find(|entry| entry.key() == *key && entry.access.is_read())
        .map(|entry| entry.paths.clone())
}

/// Reads, at `locked`'s base, the files of `entry` that `before`, the paths
/// read of its package until the change, did not cover; see [`keep`].
async fn read_added(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    local: Option<&LocalRegistry>,
    contract: &Contract,
    entry: &Entry,
    before: Option<Paths>,
    locked: Option<&LockEntry>,
) -> KeepRead {
    if !entry.access.is_read() {
        return KeepRead::Nothing;
    }
    let newly = |key| before.as_ref().is_none_or(|before| !before.covers(key));
    let Some(locked) = locked else {
        return KeepRead::Nothing;
    };
    if !locked.commits.is_empty() {
        return KeepRead::Unpushed;
    }
    let Some(base) = locked.base.as_deref() else {
        return KeepRead::Nothing;
    };
    if before
        .as_ref()
        .is_some_and(|before| covers_all(before, &entry.paths))
        || !holds_files(storage, &entry.folder).await
    {
        return KeepRead::Nothing;
    }
    let mut at_base = entry.clone();
    at_base.access = Access::Read;
    at_base.revision = RevisionPointer::Hash(base.to_string());
    let manifest = match revision_manifest(storage, remote, local, &at_base).await {
        Ok(manifest) => manifest,
        Err(err) => return KeepRead::Failed(err),
    };
    let fresh: Vec<PackagePath> = manifest
        .rows
        .iter()
        .map(|row| &row.logical_key)
        .filter(|key| entry.paths.covers(key) && newly(key))
        .map(|key| PackagePath::File(key.clone()))
        .collect();
    if fresh.is_empty() {
        return KeepRead::Nothing;
    }
    let declared: Vec<Option<&PackagePath>> = fresh.iter().map(Some).collect();
    // Keys that would share a place on disk are refused by the read itself.
    at_base.paths = Paths::union(&declared).unwrap_or_else(|| entry.paths.clone());
    match read_resolving(
        storage,
        remote,
        local,
        contract.dataspace(),
        &at_base,
        Some(base),
    )
    .await
    {
        Ok(outcome) => KeepRead::Ran(outcome),
        Err(err) => KeepRead::Failed(err),
    }
}

/// Whether every path of `after` is one of `before`'s or inside one.
fn covers_all(before: &Paths, after: &Paths) -> bool {
    match after {
        Paths::Whole => *before == Paths::Whole,
        Paths::Only(paths) => paths.iter().all(|path| before.covers_path(path)),
    }
}

#[cfg(test)]
mod tests;
