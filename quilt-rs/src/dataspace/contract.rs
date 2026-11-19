//! `data.toml`, a dataspace's contract.
//!
//! An entry is spelled one of three ways: a plain string is a folder, whose
//! package is implied; a `quilt+s3://` URI string is a package; a table gives
//! any subset of the fields. [`Contract::read`] expands each to one [`Entry`]
//! and validates the expanded entries together, so a contract is read whole or
//! refused.

use std::collections::BTreeMap;
use std::fmt;
use std::path::Component;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use serde::Deserialize;
use serde::de::IgnoredAny;
use thiserror::Error;

use crate::LogicalKey;
use crate::LogicalKeyError;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;
use quilt_uri::Host;
use quilt_uri::MIN_HASH_PREFIX_LEN;
use quilt_uri::Namespace;
use quilt_uri::RevisionPointer;
use quilt_uri::S3PackageUri;
use quilt_uri::TOP_HASH_LEN;
use quilt_uri::Tag;
use quilt_uri::UriError;
use quilt_uri::check_bucket;
use quilt_uri::is_top_hash;

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

/// Where a package's names, revisions and tags live.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[non_exhaustive]
pub enum Registry {
    /// An S3 bucket, by name.
    S3(String),
}

/// A package by its registry and name, without revision, path or catalog.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct PackageKey {
    pub registry: Registry,
    pub package: Namespace,
}

/// The canonical URI: `quilt+s3://bucket#package=owner/name`.
impl fmt::Display for PackageKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let Registry::S3(bucket) = &self.registry;
        let uri = S3PackageUri {
            catalog: None,
            bucket: bucket.clone(),
            namespace: self.package.clone(),
            revision: RevisionPointer::Tag(Tag::Latest),
            path: None,
        };
        write!(f, "{uri}")
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
    fn parse(text: &str) -> Result<Self, EntryProblem> {
        match text.strip_suffix('/') {
            Some(folder) => LogicalKey::new(folder).map(Self::Folder),
            None => LogicalKey::new(text).map(Self::File),
        }
        .map_err(EntryProblem::Path)
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
    fn union(declared: &[Option<&PackagePath>]) -> Option<Self> {
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
}

impl Contract {
    /// Reads `data.toml` in the folder `dataspace`. Folders are compared as
    /// the file system holding it compares names.
    pub async fn read(
        storage: &(impl Storage + Sync),
        dataspace: &Path,
    ) -> Result<Self, ContractError> {
        let unreadable = |path: &Path, source: crate::Error| ContractError::Read {
            path: path.to_path_buf(),
            source: Box::new(source),
        };
        let root = std::path::absolute(dataspace)
            .map(|root| normalize(&root))
            .map_err(|err| unreadable(dataspace, err.into()))?;
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
        Self::parse(&bytes, &root, case)
    }

    /// One entry per package and access, in the order of their keys.
    #[must_use]
    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// Parses a contract held in `dataspace`, an absolute path.
    pub(crate) fn parse(
        bytes: &[u8],
        dataspace: &Path,
        case: FolderCase,
    ) -> Result<Self, ContractError> {
        let raw: RawContract = toml::from_slice(bytes)?;
        let folders = Folders::new(dataspace, case);
        let defaults = Defaults::new(&raw)?;
        let lists = [
            (Access::Read, raw.packages.read),
            (Access::Write, raw.packages.write),
            (Access::Replace, raw.packages.replace),
            (Access::Sync, raw.packages.sync),
        ];
        let mut packages: BTreeMap<PackageKey, Vec<Declared>> = BTreeMap::new();
        for (access, list) in lists {
            for (index, value) in list.into_iter().enumerate() {
                let declared = expand(value, access, &defaults, &folders).map_err(|problem| {
                    ContractError::Entry {
                        access,
                        index,
                        problem,
                    }
                })?;
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
                    package: key.clone(),
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
        Ok(Self { entries })
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
        package: PackageKey,
        problem: PackageProblem,
    },

    #[error("the working folders of {first} and {second} overlap")]
    FoldersOverlap {
        first: Box<PackageKey>,
        second: Box<PackageKey>,
    },
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

    #[error("the local registry is not supported yet")]
    LocalRegistry,

    #[error("a registry is `s3://<bucket>`; got {0:?}")]
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

    #[error("its folder is, holds or sits in the dataspace's contract")]
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
    // Read where hooks are run.
    #[serde(rename = "hooks")]
    _hooks: Option<IgnoredAny>,
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

struct Defaults {
    registry: Option<Registry>,
    catalog: Option<Host>,
    prefix: Option<String>,
    suffix: Option<String>,
}

impl Defaults {
    fn new(raw: &RawContract) -> Result<Self, ContractError> {
        let registry = raw
            .registry
            .as_deref()
            .map(parse_registry)
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

fn expand(
    value: toml::Value,
    access: Access,
    defaults: &Defaults,
    folders: &Folders,
) -> Result<Declared, EntryProblem> {
    let raw = match value {
        toml::Value::String(uri) if uri.starts_with("quilt+") || uri.contains("://") => RawEntry {
            uri: Some(uri),
            ..RawEntry::default()
        },
        toml::Value::String(folder) => {
            let first = Path::new(&folder).components().find_map(|part| match part {
                Component::Normal(name) => Some(name),
                _ => None,
            });
            if first.is_some_and(|name| name.to_string_lossy().ends_with("@local")) {
                return Err(EntryProblem::LocalFolder);
            }
            RawEntry {
                folder: Some(folder),
                ..RawEntry::default()
            }
        }
        table @ toml::Value::Table(_) => table.try_into().map_err(EntryProblem::Table)?,
        other => return Err(EntryProblem::Spelling(other.type_str())),
    };

    let uri = raw.uri.as_deref().map(parse_uri).transpose()?;
    let registry = raw.registry.as_deref().map(parse_registry).transpose()?;
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
            (
                agree("registry", registry, Some(Registry::S3(uri.bucket)))?,
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
    let catalog = catalog.or_else(|| defaults.catalog.clone());
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

fn refuse_local(text: &str) -> Result<(), EntryProblem> {
    if text.starts_with("quilt+local:") {
        return Err(EntryProblem::LocalRegistry);
    }
    Ok(())
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

fn parse_uri(text: &str) -> Result<S3PackageUri, EntryProblem> {
    refuse_local(text)?;
    S3PackageUri::try_from(text).map_err(EntryProblem::Uri)
}

fn parse_registry(text: &str) -> Result<Registry, EntryProblem> {
    refuse_local(text)?;
    match text.strip_prefix("s3://") {
        Some(bucket) if check_bucket(bucket).is_ok() => Ok(Registry::S3(bucket.to_string())),
        _ => Err(EntryProblem::Registry(text.to_string())),
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
    /// The contract file's path, by its names.
    contract: Vec<String>,
}

impl<'a> Folders<'a> {
    fn new(root: &'a Path, case: FolderCase) -> Self {
        let mut folders = Self {
            root,
            case,
            contract: Vec::new(),
        };
        folders.contract = folders.names(&root.join(CONTRACT_FILE));
        folders
    }

    /// `folder`, relative to the dataspace unless absolute.
    fn resolve(&self, folder: &str) -> Result<PathBuf, EntryProblem> {
        self.checked(normalize(&self.root.join(folder)))
    }

    /// Where a package goes when its entry names no folder: under its bucket
    /// only when the entry states one.
    fn placed(&self, stated: bool, key: &PackageKey) -> Result<PathBuf, EntryProblem> {
        let mut folder = self.root.to_path_buf();
        if stated {
            let Registry::S3(bucket) = &key.registry;
            folder.push(bucket);
        }
        folder.push(key.package.prefix());
        folder.push(key.package.name());
        self.checked(folder)
    }

    /// Refuses a folder that is, holds or sits in the contract file.
    fn checked(&self, folder: PathBuf) -> Result<PathBuf, EntryProblem> {
        if overlap(&self.names(&folder), &self.contract) {
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

/// `path` with its `.` and `..` resolved by name, never by the file system.
fn normalize(path: &Path) -> PathBuf {
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
