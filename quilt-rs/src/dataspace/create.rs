//! `create`: a new package declared in a dataspace's `data.toml`, with no
//! revision, in a bucket or the local registry, and its working folder taken
//! as its first commit (25.14's build). The commit has no base (25.6), so
//! its first publication, the next sync's push, expects no `latest` (25.9).

use std::path::Path;
use std::path::PathBuf;

use thiserror::Error;
use toml_edit::InlineTable;
use tracing::info;
use tracing::warn;

use crate::dataspace::EntryError;
use crate::dataspace::METADATA_FILE;
use crate::dataspace::contract::Access;
use crate::dataspace::contract::CONTRACT_FILE;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::EntryProblem;
use crate::dataspace::contract::PackageKey;
use crate::dataspace::contract::Registry;
use crate::dataspace::contract::normalize;
use crate::dataspace::contract::parse_uri;
use crate::dataspace::edit::ContractEdit;
use crate::dataspace::install::spelled;
use crate::dataspace::install::write_lock;
use crate::dataspace::lock::LOCK_FILE;
use crate::dataspace::lock::LockError;
use crate::dataspace::lock::Mismatch;
use crate::dataspace::lock::Writer;
use crate::dataspace::read_all::usable_lock;
use crate::dataspace::store::Store;
use crate::dataspace::store::name_local;
use crate::dataspace::sync::FolderCommit;
use crate::dataspace::sync::append_commit;
use crate::dataspace::sync::commit_folder;
use crate::dataspace::sync::rewrite_metadata;
use crate::domain::DomainRoot;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use quilt_uri::Host;
use quilt_uri::RevisionPointer;
use quilt_uri::Tag;
use quilt_uri::mask_user;

/// The message of a created package's first commit when none is given, as
/// today's create writes it.
pub const DEFAULT_MESSAGE: &str = "Created package";

/// What to create.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Create {
    /// The package, as a `quilt+s3://` or `quilt+local://` URI with no
    /// revision and no path.
    pub uri: String,
    /// The list the entry goes in: `sync` by default, or `write` or
    /// `replace`, which publish a folder rather than commit it. A new
    /// package has nothing to `read`.
    pub access: Access,
    /// The working folder, relative to the dataspace unless absolute: an
    /// existing folder to package, which may be outside the dataspace.
    /// `None` places it as the contract places a package (25.4).
    pub folder: Option<PathBuf>,
    /// The first commit's message; [`DEFAULT_MESSAGE`] where `None`.
    pub message: Option<String>,
}

impl Create {
    /// `uri`, under `sync`, in its default folder.
    #[must_use]
    pub fn new(uri: impl Into<String>) -> Self {
        Self {
            uri: uri.into(),
            access: Access::Sync,
            folder: None,
            message: None,
        }
    }
}

/// What [`create`] did.
#[derive(Debug)]
#[non_exhaustive]
pub struct Created {
    /// The package's entry, as the contract now expands it.
    pub entry: Entry,
    /// The first commit, with no base; `None` where the folder held nothing
    /// to commit, or the entry is not under `sync`.
    pub commit: Option<String>,
}

/// Why nothing was created. The contract and the lock are as they were.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum CreateError {
    #[error(transparent)]
    Contract(Box<ContractError>),

    #[error(transparent)]
    Lock(Box<LockError>),

    #[error(
        "{LOCK_FILE} was written for another {}; remove the lock, or fix it, to create a package",
        .0.iter().map(ToString::to_string).collect::<Vec<_>>().join(", ")
    )]
    Mismatch(Vec<Mismatch>),

    #[error("{uri:?} is not a package to create: {problem}")]
    Uri { uri: String, problem: EntryProblem },

    #[error("{uri:?} is not a package to create: {reason}")]
    Named { uri: String, reason: &'static str },

    #[error(
        "a new package has no revision to `read`; create it under `sync`, `write` or `replace`"
    )]
    Read,

    #[error(
        "{0} is in {CONTRACT_FILE} or {LOCK_FILE} already; install it, or edit {CONTRACT_FILE}"
    )]
    Declared(Box<PackageKey>),

    #[error("{0} has a `latest` in its registry already; install it instead")]
    Exists(Box<PackageKey>),

    #[error("the package is in a local registry this machine does not hold")]
    LocalRegistry,

    #[error("the folder {} holds the Domain, which is never packaged", .0.display())]
    HoldsDomain(PathBuf),

    #[error("another process is changing {0} in the Domain; try again")]
    Busy(Box<PackageKey>),

    #[error(transparent)]
    Entry(#[from] EntryError),
}

impl From<ContractError> for CreateError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<LockError> for CreateError {
    fn from(err: LockError) -> Self {
        Self::Lock(Box::new(err))
    }
}

impl From<crate::Error> for CreateError {
    fn from(err: crate::Error) -> Self {
        Self::Entry(err.into())
    }
}

/// Creates the package `request` names in the dataspace in the folder
/// `dataspace`, which gets a `data.toml` where it has none: declares it,
/// with no revision, under `request.access`, then, under `sync`, commits
/// what its working folder holds as its first commit, with no base. Nothing
/// is pushed: the next sync publishes the commit, expecting no `latest`.
///
/// The registry is written only where it is not the contract's default, as
/// an install writes it; a `folder` given is written as given, so an
/// existing folder is packaged where it is, inside the dataspace or not, and
/// nothing is copied. A folder that holds `domain` is refused (25.14).
///
/// A package the contract or the lock already names is refused, and so is
/// one whose registry has a `latest`, as far as the Domain knows: the local
/// registry's tag, or the bucket's latest index. The bucket is not asked,
/// so a package someone published meanwhile is merged by the next sync, as
/// a base-less entry is (25m). A name outside quilt3's rule, letters,
/// digits, `_` and `-`, is refused (25.14).
///
/// The commit is taken as the tick takes one, with the contract's ignore
/// policy and the user-wide `user_rules`, and `metadata.toml` as the
/// package's metadata; the commit is kept in `domain`'s store for the
/// bucket, or in its local registry. `data.toml` is written first, and the
/// lock last, as an install writes it. A folder with nothing to commit
/// leaves the package declared and out of the lock, for the next sync.
pub async fn create(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    domain: &DomainRoot,
    dataspace: &Path,
    writer: &Writer,
    request: &Create,
    user_rules: &Path,
) -> Result<Created, CreateError> {
    if request.access == Access::Read {
        return Err(CreateError::Read);
    }
    let local = domain.local();
    let mut edit = ContractEdit::open(storage, dataspace).await?;
    let mut contract = edit.original(writer.local_registry_name())?;
    let (mut lock, mut writer) = usable_lock(storage, &contract, writer)
        .await?
        .map_err(CreateError::Mismatch)?;
    let matched = lock.header.data_toml_hash == contract.data_toml_hash();
    name_local(storage, Some(&local), &mut contract, &mut lock, &mut writer).await?;

    let (key, catalog) = wanted(storage, domain, &writer, &request.uri).await?;
    let named = contract.entries().iter().any(|entry| entry.key() == key);
    if named || lock.packages.contains_key(&key) {
        return Err(CreateError::Declared(Box::new(key)));
    }
    let latest = match &key.registry {
        Registry::S3(bucket) => {
            domain
                .registry(bucket)?
                .read_latest(storage, &key.package)
                .await?
        }
        Registry::Local(_) => local.read_latest(storage, &key.package).await?,
    };
    if latest.is_some() {
        return Err(CreateError::Exists(Box::new(key)));
    }

    let stated = contract.default_registry() != Some(&key.registry);
    let mut declaration = InlineTable::new();
    if stated {
        declaration.insert("registry", key.registry.spelled().into());
    }
    if let Some(catalog) = catalog
        && contract.default_catalog() != Some(&catalog)
    {
        declaration.insert("catalog", catalog.to_string().into());
    }
    declaration.insert("package", key.package.to_string().into());
    if let Some(folder) = &request.folder {
        declaration.insert("folder", spelled(folder).into());
    }
    edit.add(request.access, declaration);
    let mut edited = edit.edited(writer.local_registry_name())?;
    name_local(storage, Some(&local), &mut edited, &mut lock, &mut writer).await?;
    let entry = edited
        .entries()
        .iter()
        .find(|entry| entry.key() == key)
        .cloned()
        .ok_or_else(|| ContractError::Edit(format!("{key} was not declared")))?;
    if normalize(domain.root()).starts_with(&entry.folder) {
        return Err(CreateError::HoldsDomain(entry.folder));
    }
    edit.write(storage).await?;

    let mut commit = None;
    if entry.access == Access::Sync && storage.exists(&entry.folder).await {
        let store = Store::of(storage, domain, &entry).await?;
        let Some(_held) = storage
            .try_lock_exclusive(store.package_lock(&entry.package))
            .await?
        else {
            return Err(CreateError::Busy(Box::new(key)));
        };
        let message = request.message.as_deref().unwrap_or(DEFAULT_MESSAGE);
        let committed = commit_folder(
            storage,
            remote,
            &store,
            &edited,
            &entry,
            user_rules,
            (None, &Manifest::default()),
            message,
        )
        .await?;
        if let Some(FolderCommit {
            hash,
            metadata_text,
        }) = committed
        {
            append_commit(&mut lock, &key, &hash);
            if let Err(err) =
                rewrite_metadata(storage, &store, &entry, &hash, metadata_text.as_deref()).await
            {
                warn!("{METADATA_FILE} was left as it is: {err}");
            }
            commit = Some(hash);
        }
    }
    write_lock(storage, &mut lock, &edited, &writer, matched).await?;
    info!("✔️ Created {key} in {}", edited.dataspace().display());
    Ok(Created { entry, commit })
}

/// The package `uri` names, under the local registry's current name, and its
/// catalog: with no revision or path, and named as quilt3 names a new
/// package.
async fn wanted(
    storage: &(impl Storage + Sync),
    domain: &DomainRoot,
    writer: &Writer,
    uri: &str,
) -> Result<(PackageKey, Option<Host>), CreateError> {
    let masked = || mask_user(uri);
    let parsed =
        parse_uri(uri, writer.local_registry_name()).map_err(|problem| CreateError::Uri {
            uri: masked(),
            problem,
        })?;
    let refused = |reason| CreateError::Named {
        uri: masked(),
        reason,
    };
    if parsed.revision != RevisionPointer::Tag(Tag::Latest) {
        return Err(refused("a new package has no revision"));
    }
    if parsed.path.is_some() {
        return Err(refused("a new package is created whole, with no path"));
    }
    let new_name = |part: &str| {
        !part.is_empty()
            && part
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
    };
    if !new_name(parsed.namespace.prefix()) || !new_name(parsed.namespace.name()) {
        return Err(refused(
            "a new package's owner and name hold only letters, digits, `_` and `-`",
        ));
    }
    let mut registry = parsed.registry;
    if let Registry::Local(name) = &registry {
        let names = domain.local().names(storage).await?;
        if !names.answers_to(name) {
            return Err(CreateError::LocalRegistry);
        }
        registry = Registry::Local(names.name);
    }
    let key = PackageKey {
        registry,
        package: parsed.namespace,
    };
    Ok((key, parsed.catalog))
}

#[cfg(test)]
mod tests;
