//! What the Python module calls: the dataspace operations quilt3 needs,
//! each over [`quilt_rs::dataspace`], with its result as JSON values the
//! module hands Python as dicts and lists.
//!
//! The Domain and the credentials folder are found as the `quilt` CLI finds
//! them, so quilt3, `QuiltSync` and the CLI write one Domain.

use std::path::Path;
use std::path::PathBuf;

use serde_json::Value;
use serde_json::json;
use thiserror::Error;

use quilt_rs::dataspace::commit::CommitError;
use quilt_rs::dataspace::commit::commit as commit_package;
use quilt_rs::dataspace::contract::Access;
use quilt_rs::dataspace::contract::Contract;
use quilt_rs::dataspace::contract::ContractError;
use quilt_rs::dataspace::contract::Entry;
use quilt_rs::dataspace::contract::PackageKey;
use quilt_rs::dataspace::contract::Registry;
use quilt_rs::dataspace::contract::UnknownAccess;
use quilt_rs::dataspace::create::Create;
use quilt_rs::dataspace::create::CreateError;
use quilt_rs::dataspace::create::create as create_package;
use quilt_rs::dataspace::home::default_dataspace as chosen_dataspace;
use quilt_rs::dataspace::install::Install;
use quilt_rs::dataspace::install::InstallError;
use quilt_rs::dataspace::install::InstallRead;
use quilt_rs::dataspace::install::install as install_package;
use quilt_rs::dataspace::list::stored;
use quilt_rs::dataspace::lock::LockError;
use quilt_rs::dataspace::lock::Writer;
use quilt_rs::dataspace::read::ReadOutcome;
use quilt_rs::domain::DomainRoot;
use quilt_rs::domain::LocalRegistry;
use quilt_rs::domain::locate;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::storage::LocalStorage;
use quilt_uri::Namespace;
use quilt_uri::UriError;

#[cfg(test)]
mod tests;

/// The credentials folder's name in the OS's local data folder: the one the
/// `quilt` CLI and `QuiltSync` keep, so one login serves all three.
pub const HOME_DIR_NAME: &str = "com.quiltdata.quilt-sync";

/// Why a call did nothing, or stopped.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    #[error("this OS has no local data folder; name the {0} folder")]
    NoDataFolder(&'static str),

    #[error("no dataspace was named, and no default dataspace was chosen")]
    NoDefaultDataspace,

    #[error("{0} is not a package of the dataspace's contract")]
    NotDeclared(String),

    #[error("{0} names more than one package of the dataspace's contract; give its URI")]
    Ambiguous(String),

    #[error(transparent)]
    Access(#[from] UnknownAccess),

    #[error(transparent)]
    Uri(#[from] UriError),

    #[error(transparent)]
    Contract(#[from] ContractError),

    #[error(transparent)]
    Lock(#[from] LockError),

    #[error(transparent)]
    Install(#[from] InstallError),

    #[error(transparent)]
    Commit(#[from] CommitError),

    #[error(transparent)]
    Create(#[from] CreateError),

    #[error(transparent)]
    Quilt(#[from] quilt_rs::Error),
}

pub type Result<T> = std::result::Result<T, Error>;

/// The Domain: `explicit`, else `QUILT_DOMAIN`, else
/// [`DOMAIN_DIR_NAME`](quilt_rs::domain::DOMAIN_DIR_NAME) in the OS's local
/// data folder, as [`locate`] finds it.
pub fn domain(explicit: Option<&Path>) -> Result<DomainRoot> {
    locate(explicit)
        .map(DomainRoot::new)
        .ok_or(Error::NoDataFolder("Domain"))
}

/// The application folder, which holds the credentials and the choice of a
/// default dataspace: `explicit`, else [`HOME_DIR_NAME`] in the OS's local
/// data folder.
pub fn home(explicit: Option<&Path>) -> Result<PathBuf> {
    match explicit {
        Some(home) => Ok(home.to_path_buf()),
        None => dirs::data_local_dir()
            .map(|dir| dir.join(HOME_DIR_NAME))
            .ok_or(Error::NoDataFolder("credentials")),
    }
}

/// The dataspace a call with none named uses: the default dataspace chosen
/// for the application whose folder is `home`, as `quilt home` chooses it;
/// `None` where none was chosen.
pub async fn default_dataspace(home: &Path) -> Result<Option<PathBuf>> {
    Ok(chosen_dataspace(&LocalStorage::new(), home).await?)
}

/// `given`, else the default dataspace chosen for `home`.
pub async fn dataspace(given: Option<PathBuf>, home: &Path) -> Result<PathBuf> {
    match given {
        Some(dataspace) => Ok(dataspace),
        None => default_dataspace(home)
            .await?
            .ok_or(Error::NoDefaultDataspace),
    }
}

/// The registries the Domain holds locally, as a contract spells each: a
/// bucket's Package Store as `s3://<bucket>`, then the local registry as
/// `quilt+local://<name>`, where it was ever named. Nothing is written.
pub async fn list_registries(domain: &DomainRoot) -> Result<Vec<String>> {
    let storage = LocalStorage::new();
    let mut registries: Vec<String> = Vec::new();
    if domain.root().is_dir() {
        for store in domain.stores(&storage).await? {
            registries.push(Registry::S3(store.bucket().to_string()).spelled());
        }
        if let Some(names) = domain.local().read_names(&storage).await? {
            registries.push(Registry::Local(names.name).spelled());
        }
    }
    Ok(registries)
}

/// Every package the Domain's stores hold, each as `{"package": <URI>,
/// "registry": ..., "name": ..., "latest": <hash or null>}`. Nothing is
/// written.
pub async fn list_stored(domain: &DomainRoot) -> Result<Vec<Value>> {
    if !domain.root().is_dir() {
        return Ok(Vec::new());
    }
    Ok(stored(&LocalStorage::new(), domain)
        .await?
        .into_iter()
        .map(|package| {
            json!({
                "package": package.key.to_string(),
                "registry": package.key.registry.spelled(),
                "name": package.key.package.to_string(),
                "latest": package.latest,
            })
        })
        .collect())
}

/// What [`install`] is asked to do.
#[derive(Clone, Debug)]
pub struct InstallRequest {
    pub uri: String,
    /// `read`, `write`, `replace` or `sync`.
    pub access: String,
    pub paths: Vec<String>,
    pub folder: Option<PathBuf>,
}

/// Installs a package into `dataspace`, as `quilt install --dataspace`
/// does: adds or merges its entry under the access asked, then reads it. A
/// local package is reached through `domain`'s local registry, where the
/// Domain exists.
pub async fn install(
    remote: &impl Remote,
    domain: &DomainRoot,
    dataspace: &Path,
    request: InstallRequest,
) -> Result<Value> {
    let local = domain.root().is_dir().then(|| domain.local());
    let writer = writer(local.as_ref()).await?;
    let mut install = Install::new(request.uri);
    install.access = request.access.parse::<Access>()?;
    install.paths = request.paths;
    install.folder = request.folder;
    let installed = install_package(
        &LocalStorage::new(),
        remote,
        local.as_ref(),
        dataspace,
        &writer,
        &install,
    )
    .await?;
    let read = &installed.read;
    let mut value = entry_json(&installed.entry);
    value["declared"] = installed.declared.into();
    value["outcome"] = outcome(read).into();
    value["completed"] = read.completed().into();
    value["revision"] = read.lock_base().into();
    value["error"] = match read {
        InstallRead::Failed(err) => err.to_string().into(),
        _ => Value::Null,
    };
    Ok(value)
}

/// What [`create`] is asked to do.
#[derive(Clone, Debug)]
pub struct CreateRequest {
    pub uri: String,
    /// `sync`, `write` or `replace`.
    pub access: String,
    pub folder: Option<PathBuf>,
    pub message: Option<String>,
}

/// Creates a package in `dataspace`, as a build does (25.14): declares it,
/// with no revision, under the access asked, and under `sync` commits its
/// working folder as its first commit, kept in `domain`'s store for the
/// bucket, or in its local registry. Nothing is pushed.
pub async fn create(
    remote: &impl Remote,
    domain: &DomainRoot,
    home: &Path,
    dataspace: &Path,
    request: CreateRequest,
) -> Result<Value> {
    let local = domain.local();
    let writer = writer(Some(&local)).await?;
    let mut create = Create::new(request.uri);
    create.access = request.access.parse::<Access>()?;
    create.folder = request.folder;
    create.message = request.message;
    let user_rules = home.join(".quiltignore");
    let created = Box::pin(create_package(
        &LocalStorage::new(),
        remote,
        domain,
        dataspace,
        &writer,
        &create,
        &user_rules,
    ))
    .await?;
    let mut value = entry_json(&created.entry);
    value["commit"] = created.commit.into();
    Ok(value)
}

/// Commits `package`'s working folder in `dataspace` as a local commit, kept
/// in `domain`, as `quilt commit --dataspace` does. `package` is a URI, or a
/// name the contract gives one package.
pub async fn commit(
    remote: &impl Remote,
    domain: &DomainRoot,
    home: &Path,
    dataspace: &Path,
    package: &str,
    message: &str,
) -> Result<Value> {
    let storage = LocalStorage::new();
    let local = domain.local();
    let writer = writer(Some(&local)).await?;
    let contract = Contract::read(&storage, dataspace, writer.local_registry_name()).await?;
    let key = package_key(&contract, package)?;
    let user_rules = home.join(".quiltignore");
    let committed = Box::pin(commit_package(
        &storage,
        remote,
        domain,
        dataspace,
        &writer,
        &key,
        message,
        &user_rules,
    ))
    .await?;
    let mut value = entry_json(&committed.entry);
    value["commit"] = committed.commit.into();
    let (base, commits) = committed
        .locked
        .map_or((None, Vec::new()), |locked| (locked.base, locked.commits));
    value["base"] = base.into();
    value["commits"] = commits.into();
    Ok(value)
}

/// The writer of a lock on this machine, naming `local`'s name, or none
/// where there is no local registry, as the CLI's dataspace commands have it.
async fn writer(local: Option<&LocalRegistry>) -> Result<Writer> {
    let name = match local {
        Some(local) => local.names(&LocalStorage::new()).await?.name,
        None => String::new(),
    };
    Ok(Writer::here(name)?)
}

/// The key of the package `package` names in `contract`: a URI as it is, a
/// name as the one package of the contract with it.
fn package_key(contract: &Contract, package: &str) -> Result<PackageKey> {
    if package.contains("://") {
        return Ok(package.parse()?);
    }
    let namespace = Namespace::try_from(package)?;
    let mut keys: Vec<PackageKey> = contract
        .entries()
        .iter()
        .filter(|entry| entry.package == namespace)
        .map(Entry::key)
        .collect();
    keys.dedup();
    match keys.len() {
        1 => Ok(keys.remove(0)),
        0 => Err(Error::NotDeclared(package.to_string())),
        _ => Err(Error::Ambiguous(package.to_string())),
    }
}

fn entry_json(entry: &Entry) -> Value {
    json!({
        "package": entry.key().to_string(),
        "access": entry.access.to_string(),
        "folder": entry.folder.display().to_string(),
    })
}

/// The install's read, named as `quilt install --json` names it.
fn outcome(read: &InstallRead) -> &'static str {
    match read {
        InstallRead::NotRead => "not_read",
        InstallRead::Unchanged => "unchanged",
        InstallRead::Ran(ReadOutcome::Read(_)) if read.completed() => "read",
        InstallRead::Ran(ReadOutcome::Read(_)) => "incomplete",
        InstallRead::Ran(ReadOutcome::Blocked { .. }) => "blocked",
        InstallRead::Unpushed => "unpushed",
        InstallRead::Failed(_) => "failed",
        _ => "unknown",
    }
}
