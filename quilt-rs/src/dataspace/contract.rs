//! `data.toml`, a dataspace's contract.
//!
//! An entry is spelled one of three ways: a plain string is a folder, whose
//! package is implied; a `quilt+s3://` or `quilt+local://` URI string is a
//! package; a table gives any subset of the fields. [`Contract::read`] expands
//! each to one [`Entry`] and validates the expanded entries together, so a
//! contract is read whole or refused.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use serde::Deserialize;
use sha2::Digest;
use sha2::Sha256;
use thiserror::Error;

use crate::LogicalKey;
use crate::LogicalKeyError;
use crate::dataspace::METADATA_FILE;
use crate::dataspace::conflict::CONFLICT_FILE;
use crate::dataspace::hooks::Hooks;
use crate::dataspace::ignore::Ignore;
use crate::dataspace::lock::LOCK_FILE;
use crate::domain::LocalNames;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;
use quilt_uri::Host;
use quilt_uri::LocalPackageUri;
use quilt_uri::MIN_HASH_PREFIX_LEN;
use quilt_uri::Namespace;
use quilt_uri::RevisionPointer;
use quilt_uri::S3PackageUri;
use quilt_uri::TOP_HASH_LEN;
use quilt_uri::Tag;
use quilt_uri::UriError;
use quilt_uri::check_bucket;
use quilt_uri::is_top_hash;
use quilt_uri::mask_user;

/// The contract's file name, in the dataspace's folder.
pub const CONTRACT_FILE: &str = "data.toml";

/// Which of the contract's four lists an entry is in.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Access {
    /// Install the package's files; never publish.
    Read,
    /// Publish additions and modifications, never deletions.
    Write,
    /// Publish the whole package, removing what the folder lacks.
    Replace,
    /// Both directions over the same paths.
    Sync,
}

impl Access {
    /// A package under `replace` or `sync` is listed under nothing else.
    fn is_exclusive(self) -> bool {
        matches!(self, Self::Replace | Self::Sync)
    }
}

impl fmt::Display for Access {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Read => "read",
            Self::Write => "write",
            Self::Replace => "replace",
            Self::Sync => "sync",
        })
    }
}

/// The list's name as [`Display`](fmt::Display) writes it.
impl FromStr for Access {
    type Err = UnknownAccess;

    fn from_str(text: &str) -> Result<Self, UnknownAccess> {
        match text {
            "read" => Ok(Self::Read),
            "write" => Ok(Self::Write),
            "replace" => Ok(Self::Replace),
            "sync" => Ok(Self::Sync),
            _ => Err(UnknownAccess(text.to_owned())),
        }
    }
}

#[derive(Debug, Error)]
#[error("{0:?} is not an access: read, write, replace or sync")]
pub struct UnknownAccess(pub String);

/// Where a package's names, revisions and tags live.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Registry {
    /// An S3 bucket, by name.
    S3(String),
    /// A local registry, by the name it keeps.
    Local(String),
}

impl Registry {
    /// The folder a registry's packages are placed in when an entry states
    /// it: a bucket's name, or `<name>@local`, which no bucket can be.
    pub(crate) fn dir_name(&self) -> String {
        match self {
            Self::S3(bucket) => bucket.clone(),
            Self::Local(name) => format!("{name}{LOCAL_SUFFIX}"),
        }
    }

    /// The registry as a contract's `registry` field spells it:
    /// `s3://<bucket>` or `quilt+local://<name>`.
    #[must_use]
    pub fn spelled(&self) -> String {
        match self {
            Self::S3(bucket) => format!("s3://{bucket}"),
            Self::Local(name) => format!("quilt+local://{name}"),
        }
    }
}

/// What ends the folder of a local registry's packages.
pub(crate) const LOCAL_SUFFIX: &str = "@local";

/// A package by its registry and name, without revision, path or catalog.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackageKey {
    pub registry: Registry,
    pub package: Namespace,
}

/// The canonical URI: `quilt+s3://bucket#package=owner/name`, or
/// `quilt+local://name#package=owner/name`.
impl fmt::Display for PackageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let namespace = self.package.clone();
        let revision = RevisionPointer::Tag(Tag::Latest);
        match &self.registry {
            Registry::S3(bucket) => S3PackageUri {
                catalog: None,
                bucket: bucket.clone(),
                namespace,
                revision,
                path: None,
            }
            .fmt(f),
            Registry::Local(name) => LocalPackageUri {
                registry: Some(name.clone()),
                namespace,
                revision,
                path: None,
            }
            .fmt(f),
        }
    }
}

/// Reads only the canonical URI, so one with a revision, path or catalog, or
/// spelled any other way, the local shorthand included, is refused.
impl FromStr for PackageKey {
    type Err = UriError;

    fn from_str(text: &str) -> Result<Self, UriError> {
        let uri = parse_uri(text, None).map_err(|problem| match problem {
            EntryProblem::Uri(err) => err,
            EntryProblem::NoLocalRegistry => UriError::Package(format!(
                "a package's key names its registry: {:?}",
                mask_user(text)
            )),
            other => UriError::Package(format!("{other}: {:?}", mask_user(text))),
        })?;
        let key = Self {
            registry: uri.registry,
            package: uri.namespace,
        };
        if key.to_string() == text {
            Ok(key)
        } else {
            Err(UriError::Package(format!(
                "not a package's canonical URI: {:?}",
                mask_user(text)
            )))
        }
    }
}

/// A part of a package an entry covers.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum PackagePath {
    /// Everything under this folder, files added later included.
    Folder(LogicalKey),
    /// This one key.
    File(LogicalKey),
}

impl PackagePath {
    /// A path as a contract spells it: a folder ends in `/`.
    pub fn parse(text: &str) -> Result<Self, EntryProblem> {
        match text.strip_suffix('/') {
            Some(folder) => LogicalKey::new(folder).map(Self::Folder),
            None => LogicalKey::new(text).map(Self::File),
        }
        .map_err(EntryProblem::Path)
    }

    /// The path as a contract spells it, which [`PackagePath::parse`] reads.
    #[must_use]
    pub fn spelled(&self) -> String {
        match self {
            Self::Folder(key) => format!("{}/", key.to_string_lossy()),
            Self::File(key) => key.to_string_lossy().into_owned(),
        }
    }

    fn key(&self) -> &LogicalKey {
        match self {
            Self::Folder(key) | Self::File(key) => key,
        }
    }

    fn covers(&self, other: &Self) -> bool {
        match self {
            Self::Folder(folder) => self == other || folder.is_folder_of(other.key()),
            Self::File(_) => self == other,
        }
    }

    /// Whether the two would share a place on disk, whichever is a file.
    fn overlaps(&self, other: &Self) -> bool {
        let (one, other) = (self.key(), other.key());
        one == other || one.is_folder_of(other) || other.is_folder_of(one)
    }
}

/// The parts of a package an entry covers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Paths {
    Whole,
    /// Sorted, none covering another.
    Only(Vec<PackagePath>),
}

impl Paths {
    /// The union of what the declarations cover, a `None` among them being the
    /// whole package. `None` where one names as a file what another puts
    /// inside a folder of that name.
    pub(crate) fn union(declared: &[Option<&PackagePath>]) -> Option<Self> {
        let paths: Vec<&PackagePath> = declared.iter().flatten().copied().collect();
        let collides = |one: &PackagePath, other: &PackagePath| {
            one.overlaps(other) && !one.covers(other) && !other.covers(one)
        };
        if paths
            .iter()
            .enumerate()
            .any(|(i, one)| paths[i + 1..].iter().any(|other| collides(one, other)))
        {
            return None;
        }
        if paths.len() < declared.len() {
            return Some(Self::Whole);
        }
        let mut union: Vec<PackagePath> = Vec::new();
        for path in paths {
            if !union.iter().any(|kept| kept.covers(path)) {
                union.retain(|kept| !path.covers(kept));
                union.push(path.clone());
            }
        }
        union.sort();
        Some(Self::Only(union))
    }

    /// Whether `key` is a file these paths cover.
    #[must_use]
    pub fn covers(&self, key: &LogicalKey) -> bool {
        match self {
            Self::Whole => true,
            Self::Only(paths) => paths.iter().any(|path| match path {
                PackagePath::Folder(folder) => folder.is_folder_of(key),
                PackagePath::File(file) => file == key,
            }),
        }
    }

    /// Whether `path` is one of these paths or inside one.
    #[must_use]
    pub fn covers_path(&self, path: &PackagePath) -> bool {
        match self {
            Self::Whole => true,
            Self::Only(paths) => paths.iter().any(|kept| kept.covers(path)),
        }
    }

    /// Whether the folder `key` is one of these paths or inside one.
    pub(crate) fn covers_folder(&self, key: &LogicalKey) -> bool {
        match self {
            Self::Whole => true,
            Self::Only(paths) => {
                let folder = PackagePath::Folder(key.clone());
                paths.iter().any(|path| path.covers(&folder))
            }
        }
    }

    fn overlaps(&self, other: &Self) -> bool {
        match (self, other) {
            (Self::Only(ours), Self::Only(theirs)) => ours
                .iter()
                .any(|this| theirs.iter().any(|that| this.overlaps(that))),
            _ => true,
        }
    }
}

/// One package under one access, expanded from every declaration of it there.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Entry {
    pub registry: Registry,
    /// The entry's own catalog, else the contract's default, else none.
    pub catalog: Option<Host>,
    pub package: Namespace,
    /// `:latest` unless the entry is pinned, which only `read` allows.
    pub revision: RevisionPointer,
    pub paths: Paths,
    /// The working folder, as an absolute path. A package listed under two
    /// accesses has one.
    pub folder: PathBuf,
    pub access: Access,
}

impl Entry {
    #[must_use]
    pub fn key(&self) -> PackageKey {
        PackageKey {
            registry: self.registry.clone(),
            package: self.package.clone(),
        }
    }
}

/// A dataspace's contract, its entries expanded and validated.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Contract {
    entries: Vec<Entry>,
    /// Each declaration's package, by its list and its place in the list.
    declared: Vec<(Access, usize, PackageKey)>,
    registry: Option<Registry>,
    catalog: Option<Host>,
    hooks: Hooks,
    ignore: Ignore,
    dataspace: PathBuf,
    case: FolderCase,
    data_toml_hash: String,
}

impl Contract {
    /// Reads `data.toml` in the folder `dataspace`. Folders are compared as
    /// the file system holding it compares names. `quilt+local://` with no
    /// name stands for `local_registry`, this machine's local registry, and
    /// is refused where there is none.
    pub async fn read(
        storage: &(impl Storage + Sync),
        dataspace: &Path,
        local_registry: Option<&str>,
    ) -> Result<Self, ContractError> {
        let file = ContractFile::read(storage, dataspace).await?;
        Self::parse(&file.bytes, &file.dataspace, file.case, local_registry)
    }

    /// One entry per package and access, in the order of their keys.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Names the local registry `names` describes by its current name in
    /// each entry that names it by an alias, so that the entry's key is the
    /// one a lock writes. Its folder stays as the contract placed it.
    ///
    /// # Errors
    ///
    /// [`PackageProblem::Folders`] where two names for one registry leave a
    /// package in two folders, which the contract's own check could not see
    /// under different names.
    pub(crate) fn name_local(&mut self, names: &LocalNames) -> Result<(), ContractError> {
        let mut renamed = false;
        let registries = self
            .entries
            .iter_mut()
            .map(|entry| &mut entry.registry)
            .chain(
                self.declared
                    .iter_mut()
                    .map(|(_, _, key)| &mut key.registry),
            )
            .chain(self.registry.as_mut());
        for registry in registries {
            if let Registry::Local(name) = registry
                && name != &names.name
                && names.answers_to(name)
            {
                *registry = Registry::Local(names.name.clone());
                renamed = true;
            }
        }
        if !renamed {
            return Ok(());
        }
        for (i, entry) in self.entries.iter().enumerate() {
            if let Some(other) = self.entries[..i]
                .iter()
                .find(|other| other.key() == entry.key() && other.folder != entry.folder)
            {
                return Err(ContractError::Package {
                    package: Box::new(entry.key()),
                    problem: PackageProblem::Folders(other.folder.clone(), entry.folder.clone()),
                });
            }
        }
        Ok(())
    }

    /// The scripts declared under `[hooks]`.
    #[must_use]
    pub fn hooks(&self) -> &Hooks {
        &self.hooks
    }

    /// The `[ignore]` table.
    #[must_use]
    pub fn ignore(&self) -> &Ignore {
        &self.ignore
    }

    /// What the `[ignore]` table sets for packaging `folder`, an absolute
    /// path, which may be any folder: an entry's, or one that holds the
    /// dataspace. Under
    /// `folder`, whatever any rule says, it leaves out `metadata.toml` and
    /// `data.toml` at its top; where the dataspace is `folder` or inside it,
    /// the contract and `conflict.json`, and the lock unless `lockfile =
    /// false`; and the folder of each other package the contract declares
    /// inside it, unless `packages = false`.
    #[must_use]
    pub fn ignore_policy(&self, folder: &Path) -> crate::quiltignore::Policy {
        let folders = Folders::new(&self.dataspace, self.case);
        let folder = normalize(folder);
        let top = folders.names(&folder);
        // `path` below `folder`, as its own names, by the file system's
        // comparison.
        let under = |path: &Path| -> Option<PathBuf> {
            let names = folders.names(path);
            (names.len() > top.len() && names.starts_with(&top))
                .then(|| path.components().skip(top.len()).collect())
        };
        let mut left_out = vec![PathBuf::from(METADATA_FILE), PathBuf::from(CONTRACT_FILE)];
        let mut own = vec![CONTRACT_FILE, CONFLICT_FILE];
        if self.ignore.lockfile {
            own.push(LOCK_FILE);
        }
        left_out.extend(
            own.iter()
                .filter_map(|file| under(&self.dataspace.join(file))),
        );
        if self.ignore.packages {
            left_out.extend(self.entries.iter().filter_map(|entry| under(&entry.folder)));
        }
        left_out.sort();
        left_out.dedup();
        crate::quiltignore::Policy {
            left_out,
            fold_case: self.case == FolderCase::Insensitive,
            standard_patterns: self.ignore.standard_patterns,
            extend_patterns: self.ignore.extend_patterns.clone(),
        }
    }

    /// The dataspace's folder, as an absolute path.
    #[must_use]
    pub fn dataspace(&self) -> &Path {
        &self.dataspace
    }

    /// The hex SHA-256 of the `data.toml` bytes this contract was read from.
    #[must_use]
    pub fn data_toml_hash(&self) -> &str {
        &self.data_toml_hash
    }

    /// The default `registry`, which an entry that names none takes.
    #[must_use]
    pub fn default_registry(&self) -> Option<&Registry> {
        self.registry.as_ref()
    }

    /// The default `catalog`, which a bucket's entry that names none takes.
    #[must_use]
    pub fn default_catalog(&self) -> Option<&Host> {
        self.catalog.as_ref()
    }

    /// Each declaration's package, by the list it is in and its place there,
    /// in the order of the lists and then of the file.
    pub(crate) fn declared(&self) -> &[(Access, usize, PackageKey)] {
        &self.declared
    }

    /// The folder a declaration of `key` that names none is placed in: under
    /// its registry's folder only where it `states` the registry.
    pub(crate) fn placed(&self, stated: bool, key: &PackageKey) -> Result<PathBuf, EntryProblem> {
        Folders::new(&self.dataspace, self.case).placed(stated, key)
    }

    /// Whether two folders are one, compared as the file system holding the
    /// dataspace compares names.
    pub(crate) fn same_folder(&self, one: &Path, other: &Path) -> bool {
        Folders::new(&self.dataspace, self.case).same(one, other)
    }

    /// Parses a contract held in `dataspace`, an absolute path.
    pub(crate) fn parse(
        bytes: &[u8],
        dataspace: &Path,
        case: FolderCase,
        local_registry: Option<&str>,
    ) -> Result<Self, ContractError> {
        let raw: RawContract = toml::from_slice(bytes)?;
        if let Some((index, reason)) = raw.ignore.problem() {
            return Err(ContractError::Ignore { index, reason });
        }
        let folders = Folders::new(dataspace, case);
        let defaults = Defaults::new(&raw, local_registry)?;
        let lists = [
            (Access::Read, raw.packages.read),
            (Access::Write, raw.packages.write),
            (Access::Replace, raw.packages.replace),
            (Access::Sync, raw.packages.sync),
        ];
        let mut packages: BTreeMap<PackageKey, Vec<Declared>> = BTreeMap::new();
        let mut keys = Vec::new();
        for (access, list) in lists {
            for (index, value) in list.into_iter().enumerate() {
                let declared = expand(value, access, &defaults, &folders).map_err(|problem| {
                    ContractError::Entry {
                        access,
                        index,
                        problem,
                    }
                })?;
                keys.push((access, index, declared.key.clone()));
                packages
                    .entry(declared.key.clone())
                    .or_default()
                    .push(declared);
            }
        }
        let mut entries = Vec::new();
        for (key, declared) in &packages {
            let combined =
                combine(key, declared, &folders).map_err(|problem| ContractError::Package {
                    package: Box::new(key.clone()),
                    problem,
                })?;
            entries.extend(combined);
        }
        let mut placed: Vec<(&PackageKey, Vec<String>)> = Vec::new();
        for (key, declared) in &packages {
            let folder = folders.names(&declared[0].folder);
            if let Some((other, _)) = placed.iter().find(|(_, other)| overlap(&folder, other)) {
                return Err(ContractError::FoldersOverlap {
                    first: Box::new((*other).clone()),
                    second: Box::new(key.clone()),
                });
            }
            placed.push((key, folder));
        }
        Ok(Self {
            entries,
            declared: keys,
            registry: defaults.registry,
            catalog: defaults.catalog,
            hooks: raw.hooks,
            ignore: raw.ignore,
            dataspace: dataspace.to_path_buf(),
            case,
            data_toml_hash: hex::encode(Sha256::digest(bytes)),
        })
    }
}

/// `data.toml`'s bytes, as [`Contract::read`] finds them, before they are
/// parsed.
pub(crate) struct ContractFile {
    /// The dataspace's folder, as [`Contract::dataspace`] names it.
    pub(crate) dataspace: PathBuf,
    pub(crate) bytes: Vec<u8>,
    pub(crate) case: FolderCase,
}

impl ContractFile {
    /// `data.toml` in the folder `dataspace`; see [`Contract::read`].
    pub(crate) async fn read(
        storage: &(impl Storage + Sync),
        dataspace: &Path,
    ) -> Result<Self, ContractError> {
        let unreadable = |path: &Path, source: crate::Error| ContractError::Read {
            path: path.to_path_buf(),
            source: Box::new(source),
        };
        let root = dataspace_path(dataspace).map_err(|err| unreadable(dataspace, err.into()))?;
        let path = root.join(CONTRACT_FILE);
        let bytes = storage
            .read_bytes(&path)
            .await
            .map_err(|err| unreadable(&path, err))?;
        let case = if same_file(&path, &root.join(CONTRACT_FILE.to_uppercase())).await {
            FolderCase::Insensitive
        } else {
            FolderCase::Sensitive
        };
        Ok(Self {
            dataspace: root,
            bytes,
            case,
        })
    }
}

/// Why a contract was refused.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum ContractError {
    #[error("can't read {}: {source}", .path.display())]
    Read {
        path: PathBuf,
        #[source]
        source: Box<crate::Error>,
    },

    #[error("can't write {}: {source}", .path.display())]
    Write {
        path: PathBuf,
        #[source]
        source: Box<crate::Error>,
    },

    #[error("{CONTRACT_FILE} can't be edited: {0}")]
    Edit(String),

    #[error("{CONTRACT_FILE} is not a valid contract: {0}")]
    Syntax(#[from] toml::de::Error),

    #[error("the default `{key}` is not valid: {reason}")]
    Default { key: &'static str, reason: String },

    #[error("packages.{access}[{index}]: {problem}")]
    Entry {
        access: Access,
        index: usize,
        problem: EntryProblem,
    },

    #[error("{package}: {problem}")]
    Package {
        package: Box<PackageKey>,
        problem: PackageProblem,
    },

    #[error("the working folders of {first} and {second} overlap")]
    FoldersOverlap {
        first: Box<PackageKey>,
        second: Box<PackageKey>,
    },

    #[error("ignore.extend_patterns[{index}] is not a valid pattern: {reason}")]
    Ignore { index: usize, reason: String },
}

/// Why one entry, on its own, was refused.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EntryProblem {
    #[error("an entry is a string or a table, not a {0}")]
    Spelling(&'static str),

    #[error(transparent)]
    Table(toml::de::Error),

    #[error("it names no registry, and the contract has no default `registry`")]
    NoDefaultRegistry,

    #[error("it names this machine's local registry, and there is none here")]
    NoLocalRegistry,

    #[error("a local package has no catalog")]
    LocalCatalog,

    #[error("a registry is `s3://<bucket>` or `quilt+local://<name>`; got {0:?}")]
    Registry(String),

    #[error(transparent)]
    Uri(UriError),

    #[error("its catalog is not valid: {0}")]
    Catalog(UriError),

    #[error("its package is not valid: {0}")]
    Package(UriError),

    #[error("its package's name holds `@`, which starts a hash in the package's URI")]
    AtInPackage,

    #[error("a revision is a hash, or a tag after `:`; got {0:?}")]
    Revision(String),

    #[error(transparent)]
    Path(LogicalKeyError),

    #[error("its `uri` and its `{0}` disagree")]
    Disagrees(&'static str),

    #[error("it names no package, folder or uri")]
    NoPackage,

    #[error("a plain string whose first segment ends in `@local` names a local registry's folder")]
    LocalFolder,

    #[error("its folder is, holds or sits in the dataspace's contract, lock or conflict map")]
    DataspaceFolder,

    #[error(
        "its folder is too short to imply a package, and the contract has no default for the rest"
    )]
    FolderTooShort,

    #[error("`replace` covers the whole package, so it takes no path")]
    PathUnderReplace,
}

/// Why the declarations of one package, together, were refused.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum PackageProblem {
    #[error("it is placed in two folders, {} and {}", .0.display(), .1.display())]
    Folders(PathBuf, PathBuf),

    #[error("it is given two catalogs")]
    Catalogs,

    #[error("it is given two revisions")]
    Revisions,

    #[error(
        "it is listed under `{0}` and `{1}`; a package under `replace` or `sync` is listed under nothing else"
    )]
    ExclusiveAccess(Access, Access),

    #[error("it is pinned and listed under `{0}`; a pin is allowed under `read` only")]
    PinOutsideRead(Access),

    #[error("its paths under `{0}` and `{1}` overlap; a path belongs to one access")]
    PathsOverlap(Access, Access),

    #[error("its paths under `{0}` name one place as both a file and a folder")]
    PathsCollide(Access),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawContract {
    registry: Option<String>,
    catalog: Option<String>,
    default_prefix: Option<String>,
    default_suffix: Option<String>,
    #[serde(default)]
    packages: RawPackages,
    #[serde(default)]
    hooks: Hooks,
    #[serde(default)]
    ignore: Ignore,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawPackages {
    #[serde(default)]
    read: Vec<toml::Value>,
    #[serde(default)]
    write: Vec<toml::Value>,
    #[serde(default)]
    replace: Vec<toml::Value>,
    #[serde(default)]
    sync: Vec<toml::Value>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawEntry {
    registry: Option<String>,
    catalog: Option<String>,
    package: Option<String>,
    revision: Option<String>,
    path: Option<String>,
    folder: Option<String>,
    uri: Option<String>,
}

struct Defaults<'a> {
    registry: Option<Registry>,
    catalog: Option<Host>,
    prefix: Option<String>,
    suffix: Option<String>,
    /// This machine's local registry, which `quilt+local://` names.
    local_registry: Option<&'a str>,
}

impl<'a> Defaults<'a> {
    fn new(raw: &RawContract, local_registry: Option<&'a str>) -> Result<Self, ContractError> {
        let registry = raw
            .registry
            .as_deref()
            .map(|text| parse_registry(text, local_registry))
            .transpose()
            .map_err(|problem| ContractError::Default {
                key: "registry",
                reason: problem.to_string(),
            })?;
        let catalog = raw
            .catalog
            .as_deref()
            .map(Host::from_str)
            .transpose()
            .map_err(|err| ContractError::Default {
                key: "catalog",
                reason: err.to_string(),
            })?;
        Ok(Self {
            registry,
            catalog,
            prefix: raw.default_prefix.as_deref().map(clean_name),
            suffix: raw.default_suffix.as_deref().map(clean_name),
            local_registry,
        })
    }
}

/// One declaration, expanded but not yet combined with the package's others.
struct Declared {
    access: Access,
    key: PackageKey,
    catalog: Option<Host>,
    /// The revision the entry gives, `:latest` included; a URI that names
    /// none gives none.
    revision: Option<RevisionPointer>,
    path: Option<PackagePath>,
    folder: PathBuf,
}

/// The fields one of an entry's three spellings gives.
fn raw_entry(value: toml::Value) -> Result<RawEntry, EntryProblem> {
    Ok(match value {
        toml::Value::String(uri) if uri.starts_with("quilt+") || uri.contains("://") => RawEntry {
            uri: Some(uri),
            ..RawEntry::default()
        },
        toml::Value::String(folder) => {
            let first = Path::new(&folder).components().find_map(|part| match part {
                Component::Normal(name) => Some(name),
                _ => None,
            });
            if first.is_some_and(|name| name.to_string_lossy().ends_with(LOCAL_SUFFIX)) {
                return Err(EntryProblem::LocalFolder);
            }
            RawEntry {
                folder: Some(folder),
                ..RawEntry::default()
            }
        }
        table @ toml::Value::Table(_) => table.try_into().map_err(EntryProblem::Table)?,
        other => return Err(EntryProblem::Spelling(other.type_str())),
    })
}

fn expand(
    value: toml::Value,
    access: Access,
    defaults: &Defaults,
    folders: &Folders,
) -> Result<Declared, EntryProblem> {
    let raw = raw_entry(value)?;
    let local_registry = defaults.local_registry;
    let uri = raw
        .uri
        .as_deref()
        .map(|text| parse_uri(text, local_registry))
        .transpose()?;
    let registry = raw
        .registry
        .as_deref()
        .map(|text| parse_registry(text, local_registry))
        .transpose()?;
    let catalog = raw
        .catalog
        .as_deref()
        .map(Host::from_str)
        .transpose()
        .map_err(EntryProblem::Catalog)?;
    let package = raw
        .package
        .map(Namespace::try_from)
        .transpose()
        .map_err(EntryProblem::Package)?;
    if package
        .as_ref()
        .is_some_and(|package| package.prefix().contains('@') || package.name().contains('@'))
    {
        return Err(EntryProblem::AtInPackage);
    }
    let revision = raw.revision.as_deref().map(parse_revision).transpose()?;
    let path = raw.path.as_deref().map(PackagePath::parse).transpose()?;

    let (registry, catalog, package, revision, path) = match uri {
        None => (registry, catalog, package, revision, path),
        Some(uri) => {
            let uri_path = uri
                .path
                .as_deref()
                .map(|path| PackagePath::parse(&path.to_string_lossy()))
                .transpose()?;
            let uri_revision = raw
                .uri
                .as_deref()
                .is_some_and(names_revision)
                .then_some(uri.revision);
            agree(
                "revision",
                revision.clone().map(pin_of),
                Some(uri_revision.clone().and_then(pin_of)),
            )?;
            if matches!(uri.registry, Registry::Local(_)) && catalog.is_some() {
                return Err(EntryProblem::LocalCatalog);
            }
            (
                agree("registry", registry, Some(uri.registry))?,
                agree("catalog", catalog, uri.catalog)?,
                agree("package", package, Some(uri.namespace))?,
                revision.or(uri_revision),
                agree("path", path, uri_path)?,
            )
        }
    };
    if access == Access::Replace && path.is_some() {
        return Err(EntryProblem::PathUnderReplace);
    }
    let stated = registry.is_some();
    let registry = registry
        .or_else(|| defaults.registry.clone())
        .ok_or(EntryProblem::NoDefaultRegistry)?;
    let catalog = match registry {
        Registry::S3(_) => catalog.or_else(|| defaults.catalog.clone()),
        Registry::Local(_) if catalog.is_some() => return Err(EntryProblem::LocalCatalog),
        Registry::Local(_) => None,
    };
    let folder = raw
        .folder
        .map(|folder| folders.resolve(&folder))
        .transpose()?;
    let package = match (package, &folder) {
        (Some(package), _) => package,
        (None, Some(folder)) => implied_package(folder, defaults)?,
        (None, None) => return Err(EntryProblem::NoPackage),
    };
    let key = PackageKey { registry, package };
    let folder = match folder {
        Some(folder) => folder,
        None => folders.placed(stated, &key)?,
    };
    Ok(Declared {
        access,
        key,
        catalog,
        revision,
        path,
        folder,
    })
}

/// Each part the entry gives must be what its `uri` says.
fn agree<T: PartialEq>(
    field: &'static str,
    given: Option<T>,
    from_uri: Option<T>,
) -> Result<Option<T>, EntryProblem> {
    match given {
        Some(given) if Some(&given) != from_uri.as_ref() => Err(EntryProblem::Disagrees(field)),
        _ => Ok(from_uri),
    }
}

/// Whether `text` is a `quilt+local:` URI, its scheme lowercase as written.
fn is_local(text: &str) -> bool {
    text.starts_with("quilt+local:")
}

/// The local registry `name` names, this machine's where it is empty, held to
/// the names a local registry can keep.
fn local(name: Option<String>, local_registry: Option<&str>) -> Result<Registry, EntryProblem> {
    let name = name
        .or_else(|| local_registry.map(str::to_string))
        .ok_or(EntryProblem::NoLocalRegistry)?;
    match crate::domain::check_name(&name) {
        Ok(()) => Ok(Registry::Local(name)),
        Err(_) => Err(EntryProblem::Registry(mask_user(&format!(
            "quilt+local://{name}"
        )))),
    }
}

/// What a package URI in a contract names.
pub(crate) struct ParsedUri {
    pub(crate) registry: Registry,
    pub(crate) catalog: Option<Host>,
    pub(crate) namespace: Namespace,
    pub(crate) revision: RevisionPointer,
    pub(crate) path: Option<PathBuf>,
}

/// Whether a package URI's text names a revision, which the parsed URI cannot
/// tell for `:latest`.
fn names_revision(uri: &str) -> bool {
    // The fragment exactly as `S3PackageUri::try_from` reads it.
    let Ok(uri) = url::Url::parse(uri) else {
        return false;
    };
    let Some(fragment) = uri.fragment() else {
        return false;
    };
    let decode = |part| percent_encoding::percent_decode_str(part).decode_utf8_lossy();
    fragment.split('&').any(|piece| {
        piece.split_once('=').is_some_and(|(key, spec)| {
            decode(key) == "package" && decode(spec).contains([':', '@'])
        })
    })
}

/// A `quilt+s3://` or `quilt+local://` package URI, `quilt+local://` with no
/// name standing for `local_registry`, held to the names a contract takes.
pub(crate) fn parse_uri(
    text: &str,
    local_registry: Option<&str>,
) -> Result<ParsedUri, EntryProblem> {
    if is_local(text) {
        let uri = LocalPackageUri::try_from(text).map_err(EntryProblem::Uri)?;
        return Ok(ParsedUri {
            registry: local(uri.registry, local_registry)?,
            catalog: None,
            namespace: uri.namespace,
            revision: uri.revision,
            path: uri.path,
        });
    }
    let uri = S3PackageUri::try_from(text).map_err(EntryProblem::Uri)?;
    Ok(ParsedUri {
        registry: Registry::S3(uri.bucket),
        catalog: uri.catalog,
        namespace: uri.namespace,
        revision: uri.revision,
        path: uri.path,
    })
}

/// `s3://<bucket>`, or `quilt+local://<name>`, this machine's with no name.
fn parse_registry(text: &str, local_registry: Option<&str>) -> Result<Registry, EntryProblem> {
    if let Some(bucket) = text.strip_prefix("s3://")
        && check_bucket(bucket).is_ok()
    {
        return Ok(Registry::S3(bucket.to_string()));
    }
    match text.strip_prefix("quilt+local://") {
        Some("") => local(None, local_registry),
        Some(name) => local(Some(name.to_string()), local_registry),
        None => Err(EntryProblem::Registry(mask_user(text))),
    }
}

fn parse_revision(text: &str) -> Result<RevisionPointer, EntryProblem> {
    let invalid = || EntryProblem::Revision(text.to_string());
    let revision = if let Some(tag) = text.strip_prefix(':') {
        RevisionPointer::Tag(tag.parse().map_err(|_| invalid())?)
    } else if is_top_hash(text) {
        RevisionPointer::Hash(text.to_string())
    } else if (MIN_HASH_PREFIX_LEN..TOP_HASH_LEN).contains(&text.len())
        && text.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
    {
        RevisionPointer::HashPrefix(text.to_string())
    } else {
        return Err(invalid());
    };
    Ok(revision)
}

/// `None` for `:latest`, which pins nothing.
fn pin_of(revision: RevisionPointer) -> Option<RevisionPointer> {
    (revision != RevisionPointer::Tag(Tag::Latest)).then_some(revision)
}

/// The last two segments of `folder`, with the contract's defaults standing in
/// for the segments it runs out of.
fn implied_package(folder: &Path, defaults: &Defaults) -> Result<Namespace, EntryProblem> {
    let names: Vec<String> = folder
        .components()
        .filter_map(|part| match part {
            Component::Normal(name) => Some(clean_name(&name.to_string_lossy())),
            _ => None,
        })
        .collect();
    let too_short = |default: &Option<String>| default.clone().ok_or(EntryProblem::FolderTooShort);
    let (owner, name) = match names.as_slice() {
        [.., owner, name] => (owner.clone(), name.clone()),
        [name] => (too_short(&defaults.prefix)?, name.clone()),
        [] => (too_short(&defaults.prefix)?, too_short(&defaults.suffix)?),
    };
    Namespace::try_from(format!("{owner}/{name}")).map_err(EntryProblem::Package)
}

/// A name holds letters, digits, `_` and `-`; anything else becomes `_`.
fn clean_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// Joins the declarations of one package into an entry per access.
fn combine(
    key: &PackageKey,
    declared: &[Declared],
    folders: &Folders,
) -> Result<Vec<Entry>, PackageProblem> {
    let first = &declared[0];
    let mut revision: Option<&RevisionPointer> = None;
    for other in declared {
        if !folders.same(&first.folder, &other.folder) {
            return Err(PackageProblem::Folders(
                first.folder.clone(),
                other.folder.clone(),
            ));
        }
        if other.catalog != first.catalog {
            return Err(PackageProblem::Catalogs);
        }
        if other.access != first.access
            && (first.access.is_exclusive() || other.access.is_exclusive())
        {
            return Err(PackageProblem::ExclusiveAccess(first.access, other.access));
        }
        match (revision, &other.revision) {
            (Some(one), Some(other)) if one != other => return Err(PackageProblem::Revisions),
            (None, Some(other)) => revision = Some(other),
            _ => {}
        }
    }
    let revision = revision
        .cloned()
        .unwrap_or(RevisionPointer::Tag(Tag::Latest));
    if pin_of(revision.clone()).is_some()
        && let Some(publishing) = declared.iter().find(|d| d.access != Access::Read)
    {
        return Err(PackageProblem::PinOutsideRead(publishing.access));
    }

    let mut by_access: BTreeMap<Access, Vec<Option<&PackagePath>>> = BTreeMap::new();
    for d in declared {
        by_access.entry(d.access).or_default().push(d.path.as_ref());
    }
    let by_access: Vec<(Access, Paths)> = by_access
        .into_iter()
        .map(|(access, paths)| {
            Paths::union(&paths)
                .map(|paths| (access, paths))
                .ok_or(PackageProblem::PathsCollide(access))
        })
        .collect::<Result<_, _>>()?;
    for (i, (access, paths)) in by_access.iter().enumerate() {
        if let Some((other, _)) = by_access[i + 1..]
            .iter()
            .find(|(_, others)| paths.overlaps(others))
        {
            return Err(PackageProblem::PathsOverlap(*access, *other));
        }
    }

    Ok(by_access
        .into_iter()
        .map(|(access, paths)| Entry {
            registry: key.registry.clone(),
            catalog: first.catalog.clone(),
            package: key.package.clone(),
            revision: revision.clone(),
            paths,
            folder: first.folder.clone(),
            access,
        })
        .collect())
}

/// How the file system holding the dataspace compares names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum FolderCase {
    Sensitive,
    Insensitive,
}

/// Places folders in a dataspace, and compares them.
struct Folders<'a> {
    root: &'a Path,
    case: FolderCase,
    /// The paths of the contract, the lock and the conflict map, by their
    /// names.
    own_files: Vec<Vec<String>>,
}

impl<'a> Folders<'a> {
    fn new(root: &'a Path, case: FolderCase) -> Self {
        let mut folders = Self {
            root,
            case,
            own_files: Vec::new(),
        };
        folders.own_files = [CONTRACT_FILE, LOCK_FILE, CONFLICT_FILE]
            .iter()
            .map(|file| folders.names(&root.join(file)))
            .collect();
        folders
    }

    /// `folder`, relative to the dataspace unless absolute.
    fn resolve(&self, folder: &str) -> Result<PathBuf, EntryProblem> {
        self.checked(normalize(&self.root.join(folder)))
    }

    /// Where a package goes when its entry names no folder: under its
    /// registry's [`Registry::dir_name`] only when the entry states one.
    fn placed(&self, stated: bool, key: &PackageKey) -> Result<PathBuf, EntryProblem> {
        let mut folder = self.root.to_path_buf();
        if stated {
            folder.push(key.registry.dir_name());
        }
        folder.push(key.package.prefix());
        folder.push(key.package.name());
        self.checked(folder)
    }

    /// Refuses a folder that is, holds or sits in one of `own_files`.
    fn checked(&self, folder: PathBuf) -> Result<PathBuf, EntryProblem> {
        let names = self.names(&folder);
        if self.own_files.iter().any(|file| overlap(&names, file)) {
            return Err(EntryProblem::DataspaceFolder);
        }
        Ok(folder)
    }

    fn names(&self, path: &Path) -> Vec<String> {
        path.components()
            .map(|part| {
                let name = part.as_os_str().to_string_lossy();
                match self.case {
                    FolderCase::Sensitive => name.into_owned(),
                    FolderCase::Insensitive => name.to_lowercase(),
                }
            })
            .collect()
    }

    fn same(&self, one: &Path, other: &Path) -> bool {
        self.names(one) == self.names(other)
    }
}

/// Whether one path, by its names, is the other or inside it.
fn overlap(one: &[String], other: &[String]) -> bool {
    one.starts_with(other) || other.starts_with(one)
}

/// Whether two paths name one existing file, a link to it not counting. On a
/// file system that gives no file ids, whether `other` exists at all.
async fn same_file(one: &Path, other: &Path) -> bool {
    let (one, other) = (one.to_path_buf(), other.to_path_buf());
    tokio::task::spawn_blocking(move || {
        let stamp = |path: &Path| crate::file_stamp::stamp(path).ok();
        match (stamp(&one), stamp(&other)) {
            (Some(one), Some(other)) => match (one.id, other.id) {
                (Some(one), Some(other)) => one == other,
                (None, None) => true,
                _ => false,
            },
            _ => false,
        }
    })
    .await
    .unwrap_or(false)
}

/// How the file system holding the folder `dataspace` compares names, as
/// [`Contract::read`] will find it once `data.toml` is there: told by a probe
/// file written in the folder and removed. Makes the folder if it is missing.
pub(crate) async fn folder_case(
    storage: &(impl Storage + Sync),
    dataspace: &Path,
) -> crate::Res<FolderCase> {
    storage.create_dir_all(dataspace).await?;
    let name = crate::io::storage::temp_name();
    let probe = dataspace.join(&name);
    storage
        .write_byte_stream(&probe, crate::io::storage::ByteStream::from_static(b""))
        .await?;
    let case = if same_file(&probe, &dataspace.join(name.to_uppercase())).await {
        FolderCase::Insensitive
    } else {
        FolderCase::Sensitive
    };
    storage.remove_file(&probe).await?;
    Ok(case)
}

/// The dataspace at `dataspace` as [`Contract::dataspace`] names it.
pub(crate) fn dataspace_path(dataspace: &Path) -> std::io::Result<PathBuf> {
    std::path::absolute(dataspace).map(|root| normalize(&root))
}

/// `path` with its `.` and `..` resolved by name, never by the file system.
pub(crate) fn normalize(path: &Path) -> PathBuf {
    let mut normal = PathBuf::new();
    for part in path.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                normal.pop();
            }
            other => normal.push(other),
        }
    }
    normal
}

#[cfg(test)]
mod tests;
