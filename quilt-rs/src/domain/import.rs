//! The import of a domain an older build keeps, in `.quilt/` beside its
//! credentials and settings, into the Domain's root.
//!
//! Each package's installed manifests, and the objects they name, are copied
//! into its registry's store: its bucket's, or the local registry's for a
//! package with no remote. Each ref is folded into the default dataspace,
//! which is the old home: an entry under `sync` in its `data.toml`, its
//! remote's catalog the entry's `catalog`, and its base and unpushed commits
//! in its `data.lock.toml`. The list of dataspaces on autosync moves from the
//! old `queue.db` into `domain.json`, and `queue.db` itself beside the new
//! `state.db`.
//!
//! The old folder is only read, so going back is continuing to use it: what
//! is done in the Domain after the import is not carried back. An import that
//! stops part way, killed or failing, is run again and finishes; one that
//! finished is not run again.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use aws_sdk_s3::primitives::ByteStream;
use thiserror::Error;
use tracing::info;
use tracing::warn;

use super::DomainRoot;
use super::LocalPublish;
use super::file::DomainFile;
use super::file::ImportRecord;
use crate::Error;
use crate::LineageError;
use crate::Res;
use crate::dataspace::contract::CONTRACT_FILE;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::FolderCase;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::contract::Registry;
use crate::dataspace::contract::dataspace_path;
use crate::dataspace::contract::folder_case;
use crate::dataspace::lock::LOCK_FILE;
use crate::dataspace::lock::Lock;
use crate::dataspace::lock::LockEntry;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Writer;
use crate::flow::PackageStore;
use crate::flow::list_files;
use crate::flow::row_objects;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;
use crate::io::storage::temp_name;
use crate::lineage::PackageLineage;
use crate::lineage::SyncScope;
use crate::lineage::refs::RefStore;
use crate::manifest::Manifest;
use crate::manifest::TopHasher;
use crate::paths::DomainPaths;
use crate::paths::OBJECTS_DIR;
use crate::queue::QueueError;
use crate::queue::copy_queue;
use quilt_uri::Host;
use quilt_uri::Namespace;
use quilt_uri::is_top_hash;

/// How many times a package whose ref changes while it is copied is copied
/// again before the import gives up on it.
const ATTEMPTS: usize = 3;

/// Written at the top of the imported `data.toml`.
const CONTRACT_HEADER: &str = "\
# Written by the import of the packages an older Quilt kept in this folder.
# Each is synced as it was, from the folder it was in.

";

/// What the import did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Imported {
    /// The default dataspace: the old home, now holding `data.toml` and
    /// `data.lock.toml`.
    pub dataspace: PathBuf,
    /// What became of each package the old domain held.
    pub packages: BTreeMap<Namespace, PackageImport>,
    /// The dataspaces on autosync, from the old `queue.db`.
    pub autosync: Vec<PathBuf>,
}

/// What became of one package.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum PackageImport {
    /// Under `sync` in the contract, with its base and unpushed commits,
    /// oldest first, in the lock.
    Locked {
        key: PackageKey,
        base: String,
        commits: Vec<String>,
    },
    /// Under `sync` in the contract, and not in the lock: it was never
    /// published, and a lock entry needs a base. Its commits, oldest first,
    /// are in its store's `installed/`, and its folder keeps its files.
    Unpublished {
        key: PackageKey,
        commits: Vec<String>,
    },
    /// Left out: it keeps only the files it tracks, and tracks none. Its
    /// manifests are in its store.
    NothingTracked { key: PackageKey },
    /// Left out: the local registry already has a package of this name,
    /// whose `latest` names another revision. Its manifests are in the
    /// local registry's `installed/`.
    LocalTaken {
        key: PackageKey,
        latest: Option<String>,
    },
    /// Left out: its bucket has no Package Store, as for a name with
    /// capitals, which a drive that ignores case would merge with another's.
    NoStore { bucket: String, reason: String },
    /// Left out: the contract can't name it.
    Undeclarable { key: PackageKey, reason: String },
    /// Left out: it has no revision, published or committed.
    NoRevision,
    /// Left out: its ref can't be read.
    Unreadable { reason: String },
}

/// Why the import did not run, or stopped.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ImportError {
    #[error("{} holds no domain to import", .0.display())]
    NoDomain(PathBuf),

    #[error(
        "the domain in {} does not keep its packages in refs yet; open it once with this version of quilt or QuiltSync, then import it",
        .0.display()
    )]
    NotOnRefs(PathBuf),

    #[error("this Domain was imported already, from {}", .0.display())]
    Done(PathBuf),

    #[error(
        "an import into this Domain from {} has not finished; run that one again",
        .0.display()
    )]
    Unfinished(PathBuf),

    #[error("this Domain is in use already, so nothing is imported into it")]
    InUse,

    #[error(
        "{} is a dataspace already, holding {CONTRACT_FILE} or {LOCK_FILE}, so it can't become the default one",
        .0.display()
    )]
    Dataspace(PathBuf),

    #[error(
        "the home was {} when this import stopped and is {} now; set it back and import again",
        .was.display(), .now.display()
    )]
    HomeChanged { was: PathBuf, now: PathBuf },

    #[error("{0} kept changing while it was copied; quit what is using it and import again")]
    Changing(Box<Namespace>),

    #[error(transparent)]
    Contract(Box<ContractError>),

    #[error(transparent)]
    Lock(Box<LockError>),

    #[error(transparent)]
    Queue(Box<QueueError>),

    #[error(transparent)]
    Failed(Box<Error>),
}

impl From<Error> for ImportError {
    fn from(err: Error) -> Self {
        Self::Failed(Box::new(err))
    }
}

impl From<ContractError> for ImportError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<LockError> for ImportError {
    fn from(err: LockError) -> Self {
        Self::Lock(Box::new(err))
    }
}

impl From<QueueError> for ImportError {
    fn from(err: QueueError) -> Self {
        Self::Queue(Box::new(err))
    }
}

impl From<std::io::Error> for ImportError {
    fn from(err: std::io::Error) -> Self {
        Error::from(err).into()
    }
}

/// One package's entry in the contract, before it is written.
#[derive(Clone, Debug)]
struct Declaration {
    key: PackageKey,
    catalog: Option<Host>,
    /// Each a path in the contract's spelling; none is the whole package.
    paths: Vec<String>,
}

/// What importing one package gives the dataspace's two files.
struct Package {
    outcome: PackageImport,
    declaration: Option<Declaration>,
    entry: Option<LockEntry>,
}

impl Package {
    fn left_out(outcome: PackageImport) -> Self {
        Self {
            outcome,
            declaration: None,
            entry: None,
        }
    }
}

/// Imports the domain kept in `from`, the folder holding `.quilt/`, into
/// `domain`, leaving `from` as it is. The old home becomes the default
/// dataspace. The whole import holds `domain.lock`.
///
/// Refused before anything is copied where the packages' folders can't all be
/// declared together, as for names differing only in case on a drive that
/// ignores case. What is copied is synced before the import records itself done.
pub async fn import(
    storage: &(impl Storage + Sync),
    from: &Path,
    domain: &DomainRoot,
) -> Result<Imported, ImportError> {
    import_comparing(storage, from, domain, None).await
}

/// [`import`], comparing folder names as `case` says, or where it is `None`
/// as the drive holding the old home does.
async fn import_comparing(
    storage: &(impl Storage + Sync),
    from: &Path,
    domain: &DomainRoot,
    case: Option<FolderCase>,
) -> Result<Imported, ImportError> {
    let from = std::path::absolute(from)?;
    let paths = DomainPaths::new(from.clone());
    let refs = RefStore::new(paths.clone());
    storage.create_dir_all(domain.root()).await?;
    let _held = storage.lock_exclusive(domain.domain_lock()).await?;

    let resumed = resuming(storage, domain, &from).await?;
    let resuming = resumed.is_some();
    let namespaces = match refs.namespaces(storage).await {
        Ok(namespaces) => namespaces,
        Err(Error::Lineage(LineageError::Missing)) if !storage.exists(paths.lineage()).await => {
            return Err(ImportError::NoDomain(from));
        }
        Err(Error::Lineage(
            LineageError::Missing
            | LineageError::MissingHome
            | LineageError::RefsNotWritable { .. },
        )) => return Err(ImportError::NotOnRefs(from)),
        Err(err) => return Err(err.into()),
    };
    let home = refs.home(storage).await?;
    let dataspace = dataspace_path(home.as_ref())?;
    if let Some(was) = resumed.and_then(|record| record.default_dataspace)
        && was != dataspace
    {
        return Err(ImportError::HomeChanged {
            was,
            now: dataspace,
        });
    }
    if !resuming {
        for file in [CONTRACT_FILE, LOCK_FILE] {
            if storage.exists(dataspace.join(file)).await {
                return Err(ImportError::Dataspace(dataspace));
            }
        }
    }

    let local_name = domain.local().names(storage).await?.name;
    let target = Target {
        domain,
        dataspace: &dataspace,
        local_name: &local_name,
    };
    let case = match case {
        Some(case) => case,
        None => folder_case(storage, &dataspace).await?,
    };
    check_together(storage, &refs, &target, &namespaces, case).await?;

    let mut record = DomainFile {
        default_dataspace: Some(dataspace.clone()),
        autosync: Vec::new(),
        import: Some(ImportRecord {
            from: from.clone(),
            done: false,
        }),
    };
    if !resuming {
        domain.write_file(storage, &record).await?;
    }

    let mut packages = BTreeMap::new();
    let mut declarations = Vec::new();
    let mut entries = BTreeMap::new();
    let mut buckets = BTreeSet::new();
    for namespace in namespaces {
        let package = import_package(storage, &paths, &refs, &target, &namespace).await?;
        if let Some(declaration) = package.declaration {
            if let Registry::S3(bucket) = &declaration.key.registry {
                buckets.insert(bucket.clone());
            }
            if let Some(entry) = package.entry {
                entries.insert(declaration.key.clone(), entry);
            }
            declarations.push(declaration);
        }
        info!("Imported {namespace}: {:?}", package.outcome);
        packages.insert(namespace, package.outcome);
        crate::failpoint::stop("import.package")?;
    }
    for bucket in &buckets {
        copy_cached_manifests(storage, &paths, domain, bucket).await?;
    }
    let autosync = move_queue(storage, &paths, domain).await?;
    sync_folders(storage, domain).await?;
    write_dataspace(storage, &target, &declarations, entries).await?;

    record.autosync.clone_from(&autosync);
    record.import = Some(ImportRecord { from, done: true });
    domain.write_file(storage, &record).await?;
    Ok(Imported {
        dataspace,
        packages,
        autosync,
    })
}

/// The record of the import this one finishes, where one stopped part way.
/// Refused where one finished, one from another folder has not, or the
/// Domain is in use.
async fn resuming(
    storage: &(impl Storage + Sync),
    domain: &DomainRoot,
    from: &Path,
) -> Result<Option<DomainFile>, ImportError> {
    match domain.read_file(storage).await? {
        Some(file) => match &file.import {
            Some(ImportRecord {
                from: was,
                done: true,
            }) => Err(ImportError::Done(was.clone())),
            Some(ImportRecord { from: was, .. }) if was == from => Ok(Some(file)),
            Some(ImportRecord { from: was, .. }) => Err(ImportError::Unfinished(was.clone())),
            None => Err(ImportError::InUse),
        },
        None if storage.exists(domain.queue_db()).await => Err(ImportError::InUse),
        None => Ok(None),
    }
}

/// Refuses the import, before anything is copied, where the entries it would
/// write can't be read back together under `case`: two packages whose names
/// differ only in case are one folder on a drive that ignores it. A package
/// the import would leave out for a reason known before copying is not
/// counted; one left out only on copying, as [`PackageImport::LocalTaken`], is.
async fn check_together(
    storage: &(impl Storage + Sync),
    refs: &RefStore,
    target: &Target<'_>,
    namespaces: &BTreeSet<Namespace>,
    case: FolderCase,
) -> Result<(), ImportError> {
    let mut declarations = Vec::new();
    for namespace in namespaces {
        let Ok(lineage) = refs.read(storage, namespace).await else {
            continue;
        };
        let Ok((key, catalog)) = target.key(namespace, &lineage) else {
            continue;
        };
        if lineage.base_hash.is_empty() && commits_since_base(&lineage).is_empty() {
            continue;
        }
        if let Ok(declaration) = target.declared(declare(&key, catalog, &lineage), key) {
            declarations.push(declaration);
        }
    }
    let text = contract_text(&declarations).map_err(std::io::Error::other)?;
    Contract::parse(
        text.as_bytes(),
        target.dataspace,
        case,
        Some(target.local_name),
    )?;
    Ok(())
}

/// Syncs each folder under the Domain's `registries/`, and the root, so the
/// folders the copies made survive a power loss as the copies do: this run's
/// and those of a run that stopped part way. A store's own `objects/` and
/// `packages/` hold only files, so they are synced and not listed.
async fn sync_folders(storage: &(impl Storage + Sync), domain: &DomainRoot) -> Res {
    // Each folder with its depth below `registries/`: a store is at 1.
    let mut folders = vec![(domain.root().join(super::REGISTRIES_DIR), 0)];
    while let Some((folder, depth)) = folders.pop() {
        let mut entries = match storage.read_dir(&folder).await {
            Ok(entries) => entries,
            Err(err) if err.is_not_found() => continue,
            Err(err) => return Err(err),
        };
        storage.sync_dir(&folder).await;
        while let Some(entry) = entries.next_entry().await? {
            if !entry.file_type().await?.is_dir() {
                continue;
            }
            let name = entry.file_name();
            if depth == 1 && (name == OBJECTS_DIR || name == super::PACKAGES_DIR) {
                storage.sync_dir(entry.path()).await;
            } else {
                folders.push((entry.path(), depth + 1));
            }
        }
    }
    storage.sync_dir(domain.root()).await;
    Ok(())
}

/// Copies the old `queue.db` beside the new `state.db`, and returns the
/// dataspaces it held on autosync.
async fn move_queue(
    storage: &(impl Storage + Sync),
    paths: &DomainPaths,
    domain: &DomainRoot,
) -> Result<Vec<PathBuf>, ImportError> {
    let old_queue = paths.queue_db();
    if !storage.exists(&old_queue).await {
        return Ok(Vec::new());
    }
    let new_queue = domain.queue_db();
    // Only an import that stopped part way made it.
    if storage.exists(&new_queue).await {
        storage.remove_file(&new_queue).await?;
    }
    let autosync = copy_queue(&old_queue, &new_queue).await?;
    // A queue this build can't read is skipped, and no copy is made.
    if storage.is_file(&new_queue).await {
        storage.sync_file(&new_queue).await?;
    }
    Ok(autosync)
}

/// Writes the default dataspace's contract, then its lock.
async fn write_dataspace(
    storage: &(impl Storage + Sync),
    target: &Target<'_>,
    declarations: &[Declaration],
    entries: BTreeMap<PackageKey, LockEntry>,
) -> Result<(), ImportError> {
    let text = contract_text(declarations).map_err(std::io::Error::other)?;
    let temp = format!("{CONTRACT_FILE}.tmp-{}", uuid::Uuid::new_v4());
    storage
        .write_byte_stream_via(
            target.dataspace.join(CONTRACT_FILE),
            temp,
            ByteStream::from(text.into_bytes()),
            None,
        )
        .await?;
    crate::failpoint::stop("import.lock")?;
    let contract = Contract::read(storage, target.dataspace, Some(target.local_name)).await?;
    let writer = Writer::here(target.local_name)?;
    let mut lock = Lock::new(&contract, &writer);
    // Not `filled`: the first tick reads every entry in full, so a file the
    // old build never placed is filled, not published as deleted.
    lock.packages = entries;
    lock.write(storage, &contract, &writer).await?;
    Ok(())
}

/// Where packages are imported to.
struct Target<'a> {
    domain: &'a DomainRoot,
    /// The default dataspace.
    dataspace: &'a Path,
    /// The local registry's name.
    local_name: &'a str,
}

impl Target<'_> {
    /// The key the package of `lineage` is imported under, and its catalog:
    /// its bucket's, or the local registry's where it has no remote. Where
    /// its bucket has no Package Store, what then becomes of it.
    fn key(
        &self,
        namespace: &Namespace,
        lineage: &PackageLineage,
    ) -> Result<(PackageKey, Option<Host>), Box<PackageImport>> {
        let Some(remote) = &lineage.remote_uri else {
            let key = PackageKey {
                registry: Registry::Local(self.local_name.to_string()),
                package: namespace.clone(),
            };
            return Ok((key, None));
        };
        if let Err(err) = self.domain.registry(&remote.bucket) {
            return Err(Box::new(PackageImport::NoStore {
                bucket: remote.bucket.clone(),
                reason: err.to_string(),
            }));
        }
        let key = PackageKey {
            registry: Registry::S3(remote.bucket.clone()),
            package: namespace.clone(),
        };
        Ok((key, remote.origin.clone()))
    }

    /// `declaration`, unless the contract can't name it, or `None` where
    /// there is none: what then becomes of the package.
    fn declared(
        &self,
        declaration: Option<Declaration>,
        key: PackageKey,
    ) -> Result<Declaration, Box<PackageImport>> {
        let Some(declaration) = declaration else {
            return Err(Box::new(PackageImport::NothingTracked { key }));
        };
        let parsed = contract_text(std::slice::from_ref(&declaration))
            .map_err(|err| err.to_string())
            .and_then(|text| {
                Contract::parse(
                    text.as_bytes(),
                    self.dataspace,
                    FolderCase::Sensitive,
                    Some(self.local_name),
                )
                .map_err(|err| err.to_string())
            });
        match parsed {
            Ok(_) => Ok(declaration),
            Err(reason) => Err(Box::new(PackageImport::Undeclarable { key, reason })),
        }
    }
}

/// Imports one package, copying it again while its ref changes underneath.
async fn import_package(
    storage: &(impl Storage + Sync),
    paths: &DomainPaths,
    refs: &RefStore,
    target: &Target<'_>,
    namespace: &Namespace,
) -> Result<Package, ImportError> {
    let ref_bytes = || async {
        match storage.read_bytes(paths.package_ref(namespace)).await {
            Ok(bytes) => Ok(Some(bytes)),
            Err(err) if err.is_not_found() => Ok(None),
            Err(err) => Err(err),
        }
    };
    for _ in 0..ATTEMPTS {
        let before = ref_bytes().await?;
        let package = copy_package(storage, paths, refs, target, namespace).await?;
        if ref_bytes().await? == before {
            return Ok(package);
        }
    }
    Err(ImportError::Changing(Box::new(namespace.clone())))
}

async fn copy_package(
    storage: &(impl Storage + Sync),
    paths: &DomainPaths,
    refs: &RefStore,
    target: &Target<'_>,
    namespace: &Namespace,
) -> Result<Package, ImportError> {
    let lineage = match refs.read(storage, namespace).await {
        Ok(lineage) => lineage,
        Err(err) => {
            return Ok(Package::left_out(PackageImport::Unreadable {
                reason: err.to_string(),
            }));
        }
    };
    let since_base = commits_since_base(&lineage);
    let base = lineage.base_hash.clone();
    let (key, catalog) = match target.key(namespace, &lineage) {
        Ok(key) => key,
        Err(left_out) => return Ok(Package::left_out(*left_out)),
    };
    let Registry::S3(bucket) = &key.registry else {
        let local = target.domain.local();
        copy_manifests(storage, paths, &local, namespace).await?;
        let declaration = match target.declared(declare(&key, None, &lineage), key.clone()) {
            Ok(declaration) => declaration,
            Err(left_out) => return Ok(Package::left_out(*left_out)),
        };
        // A package with no bucket was never published, so its oldest
        // revision becomes its first in the local registry: the lock's base.
        let mut revisions: Vec<String> = Some(base)
            .filter(|base| !base.is_empty())
            .into_iter()
            .collect();
        revisions.extend(since_base);
        let Some((first, commits)) = revisions.split_first() else {
            return Ok(Package::left_out(PackageImport::NoRevision));
        };
        return Ok(
            match local.publish(storage, namespace, None, first).await? {
                LocalPublish::Advanced => Package {
                    outcome: PackageImport::Locked {
                        key,
                        base: first.clone(),
                        commits: commits.to_vec(),
                    },
                    declaration: Some(declaration),
                    entry: Some(LockEntry {
                        base: first.clone(),
                        commits: commits.to_vec(),
                    }),
                },
                LocalPublish::Diverged(latest) => {
                    Package::left_out(PackageImport::LocalTaken { key, latest })
                }
            },
        );
    };

    let store = target.domain.registry(bucket)?;
    copy_manifests(storage, paths, &store, namespace).await?;
    let declared = declare(&key, catalog, &lineage);
    let declaration = match target.declared(declared, key.clone()) {
        Ok(declaration) => declaration,
        Err(left_out) => return Ok(Package::left_out(*left_out)),
    };
    Ok(match (base.is_empty(), since_base.is_empty()) {
        (true, true) => Package::left_out(PackageImport::NoRevision),
        (true, false) => Package {
            outcome: PackageImport::Unpublished {
                key,
                commits: since_base,
            },
            declaration: Some(declaration),
            entry: None,
        },
        (false, _) => Package {
            outcome: PackageImport::Locked {
                key,
                base: base.clone(),
                commits: since_base.clone(),
            },
            declaration: Some(declaration),
            entry: Some(LockEntry {
                base,
                commits: since_base,
            }),
        },
    })
}

/// The package's local commits after its base, oldest first: its pending
/// commit and the ones before it, back to the base.
fn commits_since_base(lineage: &PackageLineage) -> Vec<String> {
    let Some(commit) = &lineage.commit else {
        return Vec::new();
    };
    let mut commits: Vec<String> = Vec::new();
    for hash in std::iter::once(&commit.hash).chain(&commit.prev_hashes) {
        if *hash == lineage.base_hash {
            break;
        }
        if !commits.contains(hash) {
            commits.push(hash.clone());
        }
    }
    commits.reverse();
    commits
}

/// The package's entry, its paths the files it keeps: the whole package, or
/// its folders and the files it tracks outside them, or only the files it
/// tracks. `None` where it tracks none and keeps only those.
fn declare(
    key: &PackageKey,
    catalog: Option<Host>,
    lineage: &PackageLineage,
) -> Option<Declaration> {
    let tracked = lineage
        .paths
        .keys()
        .map(|key| key.to_string_lossy().into_owned());
    let paths: Vec<String> = match &lineage.sync_scope {
        SyncScope::EntirePackage => Vec::new(),
        SyncScope::Folders(folders) => {
            let covered = |key: &String| folders.iter().any(|f| key.starts_with(&format!("{f}/")));
            folders
                .iter()
                .map(|folder| format!("{folder}/"))
                .chain(tracked.filter(|key| !covered(key)))
                .collect()
        }
        SyncScope::IndividualFiles => {
            let files: Vec<String> = tracked.collect();
            if files.is_empty() {
                return None;
            }
            files
        }
    };
    Some(Declaration {
        key: key.clone(),
        catalog,
        paths,
    })
}

/// `data.toml` for `declarations`, each under `sync` in the folder its
/// package was in: `<owner>/<name>` in the dataspace.
fn contract_text(declarations: &[Declaration]) -> Result<String, toml::ser::Error> {
    let mut sync = Vec::new();
    for declaration in declarations {
        sync.extend(entries(declaration));
    }
    let mut packages = toml::Table::new();
    packages.insert("sync".to_string(), toml::Value::Array(sync));
    let mut contract = toml::Table::new();
    contract.insert("packages".to_string(), toml::Value::Table(packages));
    Ok(format!("{CONTRACT_HEADER}{}", toml::to_string(&contract)?))
}

/// The tables declaring `declaration`, one per path.
fn entries(declaration: &Declaration) -> Vec<toml::Value> {
    let key = &declaration.key;
    let mut table = toml::Table::new();
    let registry = match &key.registry {
        Registry::S3(bucket) => format!("s3://{bucket}"),
        Registry::Local(name) => format!("quilt+local://{name}"),
    };
    table.insert("registry".to_string(), registry.into());
    if let Some(catalog) = &declaration.catalog {
        table.insert("catalog".to_string(), catalog.to_string().into());
    }
    table.insert("package".to_string(), key.package.to_string().into());
    table.insert("folder".to_string(), key.package.to_string().into());
    if declaration.paths.is_empty() {
        return vec![toml::Value::Table(table)];
    }
    declaration
        .paths
        .iter()
        .map(|path| {
            let mut table = table.clone();
            table.insert("path".to_string(), path.clone().into());
            toml::Value::Table(table)
        })
        .collect()
}

/// Copies the package's installed manifests into `store`, and before each
/// the objects it names. A row whose `file://` key names an object in the
/// old store is pointed at the copy: the top hash leaves the key out, so the
/// manifest is the same revision. A manifest already copied is left.
async fn copy_manifests(
    storage: &(impl Storage + Sync),
    paths: &DomainPaths,
    store: &impl PackageStore,
    namespace: &Namespace,
) -> Res {
    let old_objects = paths.objects_dir();
    let objects = store.objects_dir();
    for (hash, source, _) in list_files(storage, &paths.installed_manifests_dir(namespace)).await? {
        let target = store.manifests_dir(namespace).join(&hash);
        if !is_top_hash(&hash) || storage.is_file(&target).await {
            continue;
        }
        let mut manifest = Manifest::from_path(storage, &source).await?;
        let mut moved = false;
        for row in &mut manifest.rows {
            let names: BTreeSet<String> = row_objects(&row.hash, &row.physical_key).collect();
            for name in &names {
                let object = old_objects.join(name);
                if storage.is_file(&object).await {
                    copy_object(storage, &object, &objects.join(name)).await?;
                }
            }
            if let Some(name) = object_named(&row.physical_key)
                && storage.is_file(objects.join(&name)).await
            {
                row.physical_key = crate::paths::file_url(&objects.join(&name))?;
                moved = true;
            }
        }
        let bytes = if moved && top_hash(&manifest)? == hash {
            manifest.try_to_jsonlines()?.into_bytes()
        } else {
            if moved {
                warn!(
                    "{} reads back as another revision, so it is copied as it was, its objects named in {}",
                    source.display(),
                    old_objects.display()
                );
            }
            storage.read_bytes(&source).await?
        };
        storage
            .write_byte_stream(&target, ByteStream::from(bytes))
            .await?;
    }
    Ok(())
}

/// The name of the object a `file://` key names in a store's objects
/// folder, as [`row_objects`] reads it.
fn object_named(physical_key: &str) -> Option<String> {
    let path = url::Url::parse(physical_key)
        .ok()
        .filter(|url| url.scheme() == "file")?
        .to_file_path()
        .ok()?;
    let in_objects = path
        .parent()
        .and_then(Path::file_name)
        .is_some_and(|dir| dir == OBJECTS_DIR);
    if !in_objects {
        return None;
    }
    Some(path.file_name()?.to_string_lossy().into_owned())
}

fn top_hash(manifest: &Manifest) -> Res<String> {
    let mut hasher = TopHasher::new();
    hasher.append_header(&manifest.header)?;
    for row in &manifest.rows {
        hasher.append(row)?;
    }
    Ok(hasher.finalize())
}

/// Copies `from` to `to`, by way of a hidden name beside it, unless `to` is
/// there already. The copy is synced before it takes its name; its folder is
/// synced by [`sync_folders`].
async fn copy_object(storage: &(impl Storage + Sync), from: &Path, to: &Path) -> Res {
    if storage.is_file(to).await {
        return Ok(());
    }
    let folder = to.parent().unwrap_or(Path::new(""));
    storage.create_dir_all(folder).await?;
    let temp = folder.join(temp_name());
    let copied = async {
        storage.copy(from, &temp).await?;
        storage.sync_file(&temp).await?;
        storage.rename(&temp, to).await
    }
    .await;
    if copied.is_err() && storage.exists(&temp).await {
        let _ = storage.remove_file(&temp).await;
    }
    copied
}

/// Copies the manifests cached from `bucket` into its store, those already
/// there left as they are. A bucket with no store has none copied.
async fn copy_cached_manifests(
    storage: &(impl Storage + Sync),
    paths: &DomainPaths,
    domain: &DomainRoot,
    bucket: &str,
) -> Res {
    let Ok(store) = domain.registry(bucket) else {
        return Ok(());
    };
    for (hash, source, _) in list_files(storage, &paths.cached_manifests_dir(bucket)).await? {
        let target = store.cached_manifest(&hash);
        if !is_top_hash(&hash) || storage.is_file(&target).await {
            continue;
        }
        let bytes = storage.read_bytes(&source).await?;
        storage
            .write_byte_stream(&target, ByteStream::from(bytes))
            .await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests;
