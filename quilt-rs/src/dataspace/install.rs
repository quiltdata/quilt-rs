//! `install`: a package added to a dataspace's `data.toml` under an access,
//! then read into its working folder, the contract written first and the
//! lock last (25.6, 25.14).

use std::path::Path;
use std::path::PathBuf;

use thiserror::Error;
use toml_edit::InlineTable;

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
use crate::dataspace::contract::Registry;
use crate::dataspace::contract::normalize;
use crate::dataspace::contract::parse_uri;
use crate::dataspace::edit::ContractEdit;
use crate::dataspace::hooks;
use crate::dataspace::hooks::HookPoint;
use crate::dataspace::lock::Lock;
use crate::dataspace::lock::LockEntry;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Mismatch;
use crate::dataspace::lock::Writer;
use crate::dataspace::read::ReadOutcome;
use crate::dataspace::read_all::holds_files;
use crate::dataspace::read_all::usable_lock;
use crate::dataspace::store::name_local;
use crate::domain::LocalRegistry;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use quilt_uri::Host;
use quilt_uri::RevisionPointer;
use quilt_uri::Tag;
use quilt_uri::mask_user;

/// What to install.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Install {
    /// The package, as a `quilt+s3://` or `quilt+local://` URI. A path the
    /// URI names is installed beside `paths`; a revision it names is read,
    /// and goes in the lock, never in the contract.
    pub uri: String,
    /// The list the entry goes in.
    pub access: Access,
    /// The parts of the package to install, as a contract spells them: a
    /// folder ends in `/`. None, with no path in the URI, is the whole
    /// package.
    pub paths: Vec<String>,
    /// The working folder, relative to the dataspace unless absolute;
    /// `None` places it as the contract places a package (25.4).
    pub folder: Option<PathBuf>,
}

impl Install {
    /// The whole package `uri` names, under `sync`, in its default folder.
    #[must_use]
    pub fn new(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            access: Access::Sync,
            paths: Vec::new(),
            folder: None,
        }
    }
}

/// How the read that followed the install ended.
#[derive(Debug)]
#[non_exhaustive]
pub enum InstallRead {
    /// The entry is under `write` or `replace`, which are not read.
    NotRead,
    /// The lock already named the revision and the working folder held its
    /// files, so nothing was read or written.
    Unchanged,
    /// The read ran. The lock took its base where
    /// [`ReadOutcome::lock_base`] names one.
    Ran(ReadOutcome),
    /// The lock names local commits of the package that are not pushed, so
    /// the package was not read.
    Unpushed,
    /// The read stopped; the lock was left.
    Failed(EntryError),
}

impl InstallRead {
    /// The base the lock holds for the package once the install completed.
    #[must_use]
    pub fn lock_base(&self) -> Option<&str> {
        match self {
            Self::Ran(outcome) => outcome.lock_base(),
            Self::NotRead | Self::Unchanged | Self::Unpushed | Self::Failed(_) => None,
        }
    }

    /// Whether the install completed, so the lock was written.
    #[must_use]
    pub fn completed(&self) -> bool {
        match self {
            Self::NotRead | Self::Unchanged => true,
            Self::Ran(outcome) => outcome.lock_base().is_some(),
            Self::Unpushed | Self::Failed(_) => false,
        }
    }
}

/// What [`install`] did.
#[derive(Debug)]
#[non_exhaustive]
pub struct Installed {
    /// The package's entry under the access installed, as the contract now
    /// expands it.
    pub entry: Entry,
    /// Whether `data.toml` was written: false where an entry already
    /// declared all that was asked.
    pub declared: bool,
    pub read: InstallRead,
}

/// Why nothing was installed. The contract and the lock are as they were.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum InstallError {
    #[error(transparent)]
    Contract(Box<ContractError>),

    #[error(transparent)]
    Lock(Box<LockError>),

    #[error(
        "the lock was written for another {}, and a working folder holds files or the lock names local commits not pushed; remove the lock, or fix it, to install",
        .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    Mismatch(Vec<Mismatch>),

    #[error("{uri:?} is not a package to install: {problem}")]
    Uri { uri: String, problem: EntryProblem },

    #[error(
        "{package} is listed under `{declared}` in {CONTRACT_FILE}, not `{requested}`; edit {CONTRACT_FILE} to change its access"
    )]
    Access {
        package: Box<PackageKey>,
        declared: Access,
        requested: Access,
    },

    #[error(
        "{package} is in the folder {}, not {}; edit {CONTRACT_FILE} to move it",
        .declared.display(),
        .requested.display()
    )]
    Folder {
        package: Box<PackageKey>,
        declared: PathBuf,
        requested: PathBuf,
    },

    #[error(
        "{package} is installed from another catalog; edit {CONTRACT_FILE} to change its catalog"
    )]
    Catalog { package: Box<PackageKey> },

    #[error(
        "{package} is pinned to {pin} in {CONTRACT_FILE}; edit {CONTRACT_FILE} to install another revision"
    )]
    Pinned {
        package: Box<PackageKey>,
        /// The pin, as the contract spells it.
        pin: String,
    },

    #[error("a `{0}` entry is not read, so an install under it takes no revision")]
    RevisionNotRead(Access),

    /// No local registry was given, or the one given does not answer to the
    /// name the URI gives, as another machine's does not.
    #[error("the package is in a local registry this machine does not hold")]
    LocalRegistry,

    #[error(transparent)]
    Failed(Box<crate::Error>),
}

impl From<ContractError> for InstallError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<LockError> for InstallError {
    fn from(err: LockError) -> Self {
        Self::Lock(Box::new(err))
    }
}

impl From<crate::Error> for InstallError {
    fn from(err: crate::Error) -> Self {
        Self::Failed(Box::new(err))
    }
}

/// The package an [`Install`] names, as the contract would.
struct Wanted {
    key: PackageKey,
    catalog: Option<Host>,
    /// The revision to read, where the URI names one but `latest`.
    revision: Option<RevisionPointer>,
    /// Each path to install; `None` is the whole package.
    paths: Vec<Option<PackagePath>>,
}

impl Wanted {
    async fn parse(
        storage: &(impl Storage + Sync),
        local: Option<&LocalRegistry>,
        writer: &Writer,
        request: &Install,
    ) -> Result<Self, InstallError> {
        let refused = |problem| InstallError::Uri {
            uri: mask_user(&request.uri),
            problem,
        };
        let uri = parse_uri(&request.uri, writer.local_registry_name()).map_err(refused)?;
        let mut registry = uri.registry;
        // A local package is only this machine's local registry's, named by
        // its current name, as the contract's keys are.
        if let Registry::Local(name) = &registry {
            let Some(local) = local else {
                return Err(InstallError::LocalRegistry);
            };
            let names = local.names(storage).await?;
            if !names.answers_to(name) {
                return Err(InstallError::LocalRegistry);
            }
            registry = Registry::Local(names.name);
        }
        let mut paths = Vec::new();
        for text in uri
            .path
            .iter()
            .map(|path| path.to_string_lossy().into_owned())
            .chain(request.paths.iter().cloned())
        {
            paths.push(Some(PackagePath::parse(&text).map_err(refused)?));
        }
        if paths.is_empty() {
            paths.push(None);
        }
        Ok(Self {
            key: PackageKey {
                registry,
                package: uri.namespace,
            },
            catalog: uri.catalog,
            revision: Some(uri.revision).filter(|revision| *revision != latest()),
            paths,
        })
    }
}

fn latest() -> RevisionPointer {
    RevisionPointer::Tag(Tag::Latest)
}

/// Installs the package `request` names in the dataspace in the folder
/// `dataspace`: adds an entry to `data.toml` under `request.access`, then,
/// for a `read` or `sync` entry, reads it into its working folder. A
/// dataspace with no `data.toml` gets one.
///
/// The entry is unpinned: a revision the URI names is read and goes in the
/// lock, so the next read or sync moves the folder on to `latest`. Its
/// registry is written only where it is not the contract's default, so a
/// package of the default registry is placed in `owner/name/` and another's
/// in `<registry>/owner/name/` (25.4).
///
/// An entry the contract already has under the same access is extended by
/// the paths it does not cover yet, which are read first at the lock's base
/// so that their files are not taken for deletions; one that covers them
/// all is left as it is, and installing a revision the lock already names
/// into a folder that holds files does nothing. The package under another
/// access, in another folder or from another catalog, or pinned to another
/// revision, is refused before anything is written: the contract is the
/// user's to change.
///
/// `data.toml` is written first, and the lock only once the install
/// completed, taking the revision read as the package's base. A lock that
/// matched the contract before the install names the new one after, so the
/// install is not taken for an edit by hand. A read that local changes
/// block, or that fails, leaves the new entry in `data.toml` and the lock as
/// it was, for the next read or sync. A package whose lock entry names
/// unpushed commits is not read. `filled` is never set or cleared here.
///
/// The lock is used as [`read_all`](crate::dataspace::read_all::read_all)
/// uses it, and `conflict.json` as [`read_resolving`] does. An entry of the
/// local registry `local` is read from it; with none given, a local package
/// is refused when it is read. The contract's `pre-read` hook runs before
/// the read, and `post-read` once the lock moved to a newer revision.
pub async fn install(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    local: Option<&LocalRegistry>,
    dataspace: &Path,
    writer: &Writer,
    request: &Install,
) -> Result<Installed, InstallError> {
    let mut edit = ContractEdit::open(storage, dataspace).await?;
    let mut contract = edit.original(writer.local_registry_name())?;
    let (mut lock, mut writer) = usable_lock(storage, &contract, writer)
        .await?
        .map_err(InstallError::Mismatch)?;
    let matched = lock.header.data_toml_hash == contract.data_toml_hash();
    name_local(storage, local, &mut contract, &mut lock, &mut writer).await?;
    let wanted = Wanted::parse(storage, local, &writer, request).await?;
    if wanted.revision.is_some() && !request.access.is_read() {
        return Err(InstallError::RevisionNotRead(request.access));
    }

    let added = declare(&mut edit, &contract, &wanted, request)?;
    let mut edited = edit.edited(writer.local_registry_name())?;
    name_local(storage, local, &mut edited, &mut lock, &mut writer).await?;
    let entry = edited
        .entries()
        .iter()
        .find(|entry| entry.key() == wanted.key && entry.access == request.access)
        .cloned()
        .ok_or_else(|| ContractError::Edit(format!("{} was not declared", wanted.key)))?;
    if let Some(revision) = &wanted.revision
        && entry.revision != latest()
        && !agrees(revision, &entry.revision)
    {
        let pin = match &entry.revision {
            RevisionPointer::Hash(hash) | RevisionPointer::HashPrefix(hash) => hash.clone(),
            RevisionPointer::Tag(tag) => format!(":{tag}"),
        };
        return Err(InstallError::Pinned {
            package: Box::new(wanted.key),
            pin,
        });
    }
    let declared = edit.changed();
    if declared {
        edit.write(storage).await?;
    }

    let locked = lock.packages.get(&entry.key()).cloned();
    let read = first_read(
        storage,
        remote,
        local,
        &edited,
        &entry,
        wanted.revision.as_ref(),
        &added,
        locked.as_ref(),
    )
    .await;
    let before = locked.as_ref().and_then(|locked| locked.base.as_deref());
    let newer = hooks::newer(before, read.lock_base()).map(str::to_string);
    if read.completed() && (declared || newer.is_some()) {
        if let Some(base) = read.lock_base() {
            lock.packages.insert(entry.key(), LockEntry::at(base));
        }
        write_lock(storage, &mut lock, &edited, &writer, matched).await?;
        if let Some(revision) = &newer {
            hooks::after(&edited, HookPoint::PostRead, &entry, revision).await;
        }
    }
    Ok(Installed {
        entry,
        declared,
        read,
    })
}

/// Adds to `edit` a declaration for each of `wanted`'s paths that
/// `contract`'s entry of the package under `request.access` does not cover,
/// and returns those paths; `None` among them is the whole package.
fn declare(
    edit: &mut ContractEdit,
    contract: &Contract,
    wanted: &Wanted,
    request: &Install,
) -> Result<Vec<Option<PackagePath>>, InstallError> {
    let package = || Box::new(wanted.key.clone());
    let mut existing = None;
    let mut other = None;
    for entry in contract.entries() {
        if entry.key() != wanted.key {
            continue;
        }
        if entry.access == request.access {
            existing = Some(entry);
        } else {
            other.get_or_insert(entry.access);
        }
    }
    if let (None, Some(declared)) = (existing, other) {
        return Err(InstallError::Access {
            package: package(),
            declared,
            requested: request.access,
        });
    }
    let stated = contract.default_registry() != Some(&wanted.key.registry);
    let Some(existing) = existing else {
        let folder = request.folder.as_ref().map(|folder| spelled(folder));
        for path in &wanted.paths {
            let declaration = declaration(
                contract,
                wanted,
                stated,
                wanted.catalog.as_ref(),
                path.as_ref(),
                folder.clone(),
            );
            edit.add(request.access, declaration);
        }
        return Ok(wanted.paths.clone());
    };

    // The declarations added must expand to the existing entry's folder and
    // catalog, or the contract refuses them as a second folder or catalog.
    let requested = request
        .folder
        .as_ref()
        .map(|folder| normalize(&contract.dataspace().join(folder)));
    if let Some(requested) = requested
        && !contract.same_folder(&requested, &existing.folder)
    {
        return Err(InstallError::Folder {
            package: package(),
            declared: existing.folder.clone(),
            requested,
        });
    }
    if wanted
        .catalog
        .as_ref()
        .is_some_and(|catalog| existing.catalog.as_ref() != Some(catalog))
    {
        return Err(InstallError::Catalog { package: package() });
    }
    let placed = contract
        .placed(stated, &wanted.key)
        .map_err(|problem| InstallError::Uri {
            uri: mask_user(&request.uri),
            problem,
        })?;
    let folder = (!contract.same_folder(&placed, &existing.folder)).then(|| {
        let folder = existing
            .folder
            .strip_prefix(contract.dataspace())
            .unwrap_or(&existing.folder);
        spelled(folder)
    });
    let added: Vec<Option<PackagePath>> = wanted
        .paths
        .iter()
        .filter(|path| match path {
            None => existing.paths != Paths::Whole,
            Some(path) => !existing.paths.covers_path(path),
        })
        .cloned()
        .collect();
    for path in &added {
        let declaration = declaration(
            contract,
            wanted,
            stated,
            existing.catalog.as_ref(),
            path.as_ref(),
            folder.clone(),
        );
        edit.add(request.access, declaration);
    }
    Ok(added)
}

/// A folder as a contract spells it, with `/` between its parts.
fn spelled(folder: &Path) -> String {
    if folder.is_absolute() {
        return folder.to_string_lossy().into_owned();
    }
    let parts: Vec<String> = folder
        .components()
        .map(|part| part.as_os_str().to_string_lossy().into_owned())
        .collect();
    parts.join("/")
}

/// One declaration of `wanted`'s package: its registry where `stated`, its
/// catalog where it is not the contract's default, then its package, and
/// the path and folder given. Never a revision.
fn declaration(
    contract: &Contract,
    wanted: &Wanted,
    stated: bool,
    catalog: Option<&Host>,
    path: Option<&PackagePath>,
    folder: Option<String>,
) -> InlineTable {
    let mut table = InlineTable::new();
    if stated {
        table.insert("registry", wanted.key.registry.spelled().into());
    }
    if let Some(catalog) = catalog
        && contract.default_catalog() != Some(catalog)
    {
        table.insert("catalog", catalog.to_string().into());
    }
    table.insert("package", wanted.key.package.to_string().into());
    if let Some(path) = path {
        table.insert("path", path.spelled().into());
    }
    if let Some(folder) = folder {
        table.insert("folder", folder.into());
    }
    table
}

/// Whether the revision `wanted` names is `named`, a hash or a prefix of one
/// naming the same hash.
fn agrees(wanted: &RevisionPointer, named: &RevisionPointer) -> bool {
    match (wanted, named) {
        (RevisionPointer::Hash(hash), RevisionPointer::HashPrefix(prefix))
        | (RevisionPointer::HashPrefix(prefix), RevisionPointer::Hash(hash)) => {
            hash.starts_with(prefix.as_str())
        }
        (one, other) => one == other,
    }
}

/// Whether `wanted` names the revision `base`.
fn names(wanted: &RevisionPointer, base: &str) -> bool {
    match wanted {
        RevisionPointer::Hash(hash) => hash == base,
        RevisionPointer::HashPrefix(prefix) => base.starts_with(prefix.as_str()),
        RevisionPointer::Tag(_) => false,
    }
}

/// Reads `entry` after its install: in full where the lock has no base for
/// it or its folder holds nothing, filling what is missing; otherwise the
/// paths just `added` at the lock's base, then the revision `wanted`, or
/// the contract's, against the base, as a read of the entry would.
#[allow(clippy::too_many_arguments, reason = "each is a distinct input")]
async fn first_read(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    local: Option<&LocalRegistry>,
    contract: &Contract,
    entry: &Entry,
    wanted: Option<&RevisionPointer>,
    added: &[Option<PackagePath>],
    locked: Option<&LockEntry>,
) -> InstallRead {
    if !entry.access.is_read() {
        return InstallRead::NotRead;
    }
    if locked.is_some_and(|locked| !locked.commits.is_empty()) {
        return InstallRead::Unpushed;
    }
    let base = locked.and_then(|locked| locked.base.as_deref());
    let target = wanted.cloned().unwrap_or_else(|| entry.revision.clone());
    let filled = holds_files(storage, &entry.folder).await;
    let unchanged = base.is_some_and(|base| filled && added.is_empty() && names(&target, base));
    if unchanged {
        return InstallRead::Unchanged;
    }
    if let Err(err) = hooks::run(contract, HookPoint::PreRead, entry, None).await {
        return InstallRead::Failed(err.into());
    }
    let dataspace = contract.dataspace();
    let Some(base) = base.filter(|_| filled) else {
        // Read in full, so a missing file is filled and not taken for a
        // deletion, as the sync tick's fill does.
        let mut whole = entry.clone();
        whole.access = Access::Read;
        whole.revision = target;
        return ran(read_resolving(storage, remote, local, dataspace, &whole, base).await);
    };
    if !added.is_empty() {
        let declared: Vec<Option<&PackagePath>> = added.iter().map(Option::as_ref).collect();
        let mut fresh = entry.clone();
        fresh.access = Access::Read;
        fresh.revision = RevisionPointer::Hash(base.to_string());
        fresh.paths = Paths::union(&declared).unwrap_or(Paths::Whole);
        match read_resolving(storage, remote, local, dataspace, &fresh, Some(base)).await {
            Ok(outcome) if outcome.lock_base().is_some() && names(&target, base) => {
                return InstallRead::Ran(outcome);
            }
            Ok(outcome) if outcome.lock_base().is_some() => {}
            other => return ran(other),
        }
    }
    let mut at = entry.clone();
    at.revision = target;
    match read_resolving(storage, remote, local, dataspace, &at, Some(base)).await {
        Ok(ReadOutcome::Read(report))
            if report.revision == base
                && report.placed.is_empty()
                && report.removed.is_empty()
                && report.skipped.is_empty()
                && report.hash_mismatch.is_empty()
                && added.is_empty() =>
        {
            InstallRead::Unchanged
        }
        other => ran(other),
    }
}

fn ran(read: Result<ReadOutcome, EntryError>) -> InstallRead {
    match read {
        Ok(outcome) => InstallRead::Ran(outcome),
        Err(err) => InstallRead::Failed(err),
    }
}

/// Writes `lock` for `contract`: naming it as written for the contract's
/// `data.toml` where it `matched` the contract before the install changed
/// it, and otherwise as [`Lock::write`] does.
pub(crate) async fn write_lock(
    storage: &(impl Storage + Sync),
    lock: &mut Lock,
    contract: &Contract,
    writer: &Writer,
    matched: bool,
) -> Result<(), LockError> {
    if matched {
        lock.write_for(storage, contract, writer, contract.data_toml_hash())
            .await
    } else {
        lock.write(storage, contract, writer).await
    }
}

#[cfg(test)]
mod tests;
