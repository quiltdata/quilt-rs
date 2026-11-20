//! The Domain's storage: an ordinary folder, `bio.quilt.dataspace/`, holding
//! a Package Store for each remote registry under `registries/<bucket>/`, and
//! the local registry under `registries/@local/`.
//!
//! A [`LocalDomain`](crate::LocalDomain) still keeps its packages in
//! `.quilt/` ([`DomainPaths`](crate::paths::DomainPaths)); nothing moves here
//! until existing state is imported.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use aws_sdk_s3::primitives::ByteStream;

use crate::Error;
use crate::Res;
use crate::flow::GcReport;
use crate::flow::PackageStore;
use crate::flow::list_dirs;
use crate::flow::list_files;
use crate::flow::locked_namespaces;
use crate::flow::row_objects;
use crate::io::remote::Remote;
use crate::io::storage::LockGuard;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;
use crate::io::storage::temp_name;
use crate::manifest::Manifest;
use quilt_uri::Host;
use quilt_uri::Namespace;
use quilt_uri::S3Uri;
use quilt_uri::UriError;
use quilt_uri::check_bucket;
use quilt_uri::is_top_hash;
use quilt_uri::paths::get_manifest_key;

/// The Domain's folder name in the OS's local data folder.
pub const DOMAIN_DIR_NAME: &str = "bio.quilt.dataspace";

const REGISTRIES_DIR: &str = "registries";
const LATEST_DIR: &str = "latest";
const PACKAGES_DIR: &str = "packages";
const INSTALLED_DIR: &str = "installed";
const OBJECTS_DIR: &str = "objects";
const LOCKS_DIR: &str = "locks";
/// No bucket name holds `@`, so no bucket's store can take this folder.
const LOCAL_DIR: &str = "@local";
const NAME_FILE: &str = "registry.json";
const NAME_LOCK: &str = "registry.lock";

/// The Domain's storage folder, holding a store per registry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DomainRoot {
    root: PathBuf,
}

impl DomainRoot {
    #[must_use]
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Self { root: root.into() }
    }

    /// The Package Store of `bucket`. A name that is no bucket name, even
    /// under S3's legacy rule, is refused, so no bucket can name a folder
    /// outside `registries/` or one of the `@` stores beside the buckets. So
    /// is a legacy name with a capital letter or a trailing `.`, whose folder
    /// a drive that ignores case, or Windows, would share with another
    /// bucket's, and on Windows a name its folder would take for a device.
    pub fn registry(&self, bucket: &str) -> Res<RegistryStore> {
        check_bucket(bucket)?;
        if bucket.bytes().any(|b| b.is_ascii_uppercase()) || bucket.ends_with('.') {
            return Err(UriError::Bucket(format!(
                "a bucket named with capital letters or a trailing `.` has no Package Store; got {bucket:?}"
            ))
            .into());
        }
        if cfg!(windows) && is_windows_device_name(bucket) {
            return Err(UriError::Bucket(format!(
                "Windows reserves {bucket:?} for a device, so the bucket has no Package Store here"
            ))
            .into());
        }
        Ok(RegistryStore {
            bucket: bucket.to_string(),
            dir: self.root.join(REGISTRIES_DIR).join(bucket),
        })
    }

    /// This machine's local registry, under `registries/@local/`.
    #[must_use]
    pub fn local(&self) -> LocalRegistry {
        LocalRegistry {
            dir: self.root.join(REGISTRIES_DIR).join(LOCAL_DIR),
        }
    }
}

/// Whether Windows takes a file named `name` for a device: `con`, `prn`,
/// `aux`, `nul`, `com0` to `com9` or `lpt0` to `lpt9`, alone or before a
/// `.`, in any case.
fn is_windows_device_name(name: &str) -> bool {
    let stem = name
        .split('.')
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    matches!(stem.as_str(), "con" | "prn" | "aux" | "nul")
        || (stem.len() == 4
            && (stem.starts_with("com") || stem.starts_with("lpt"))
            && stem.as_bytes()[3].is_ascii_digit())
}

/// One bucket's Package Store: a cache of the bucket, and the local commits
/// not yet pushed to it. No other bucket's store shares its objects.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryStore {
    bucket: String,
    dir: PathBuf,
}

impl RegistryStore {
    #[must_use]
    pub fn bucket(&self) -> &str {
        &self.bucket
    }

    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The file of the latest index holding the hash `latest` was last known
    /// to name for `namespace`.
    #[must_use]
    pub fn latest(&self, namespace: &Namespace) -> PathBuf {
        package_path(&self.dir.join(LATEST_DIR), namespace, "")
    }

    /// A manifest fetched from the bucket.
    #[must_use]
    pub fn cached_manifest(&self, hash: &str) -> PathBuf {
        self.dir.join(PACKAGES_DIR).join(hash)
    }

    /// Where `namespace`'s manifests installed or committed here go,
    /// unpushed commits included.
    #[must_use]
    pub fn installed_manifests_dir(&self, namespace: &Namespace) -> PathBuf {
        package_path(&self.dir.join(INSTALLED_DIR), namespace, "")
    }

    #[must_use]
    pub fn installed_manifest(&self, namespace: &Namespace, hash: &str) -> PathBuf {
        self.installed_manifests_dir(namespace).join(hash)
    }

    #[must_use]
    pub fn package_lock(&self, namespace: &Namespace) -> PathBuf {
        package_path(&self.dir.join(LOCKS_DIR), namespace, ".lock")
    }

    /// The hash the latest index holds for `namespace`, or `None` when it
    /// holds none: a cache that is missing or holds no top hash is unknown,
    /// not an error.
    pub async fn read_latest(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
    ) -> Res<Option<String>> {
        let hash = read_tag(storage, &self.latest(namespace)).await?;
        Ok(hash.filter(|hash| is_top_hash(hash)))
    }

    /// Records in the latest index that `latest` names `hash` for
    /// `namespace`.
    pub async fn write_latest(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
        hash: &str,
    ) -> Res {
        write_tag(storage, &self.dir, &self.latest(namespace), hash).await
    }

    /// Every package the latest index holds a hash for.
    pub async fn latest_index(
        &self,
        storage: &(impl Storage + Sync),
    ) -> Res<BTreeMap<Namespace, String>> {
        let mut index = BTreeMap::new();
        for owner_dir in list_dirs(storage, &self.dir.join(LATEST_DIR)).await? {
            let mut entries = storage.read_dir(&owner_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                if !entry.file_type().await?.is_file() {
                    continue;
                }
                let Some(namespace) = namespace_of(&entry.path()) else {
                    continue;
                };
                if let Some(hash) = self.read_latest(storage, &namespace).await? {
                    index.insert(namespace, hash);
                }
            }
        }
        Ok(index)
    }

    /// Replaces the latest index with what the bucket's registry lists now,
    /// and returns it: a package the registry no longer lists leaves the
    /// index. Nothing changes if the listing fails.
    pub async fn refresh_latest_index(
        &self,
        storage: &(impl Storage + Sync),
        remote: &impl Remote,
        host: Option<&Host>,
    ) -> Res<BTreeMap<Namespace, String>> {
        let listed: BTreeMap<Namespace, String> = remote
            .latest_hashes(host, &self.bucket)
            .await?
            .into_iter()
            .collect();
        let known = self.latest_index(storage).await?;
        for (namespace, hash) in &listed {
            if known.get(namespace) != Some(hash) {
                self.write_latest(storage, namespace, hash).await?;
            }
        }
        for namespace in known.keys().filter(|known| !listed.contains_key(*known)) {
            if let Err(err) = storage.remove_file(self.latest(namespace)).await
                && err.kind() != std::io::ErrorKind::NotFound
            {
                return Err(err.into());
            }
        }
        Ok(listed)
    }

    /// Clears what the bucket has: every manifest in `packages/` and
    /// `installed/` whose top hash the bucket holds, and every object that
    /// no other manifest uses. A manifest the bucket lacks, such as an
    /// unpushed commit, is kept with its objects. Clears nothing if the
    /// bucket cannot be asked about every manifest, as when offline, if
    /// there is no manifest to ask about, or while another writer holds the
    /// lock of a package the store already
    /// has; a package first written meanwhile is not protected, as in
    /// `flow::gc`.
    pub async fn clear_published(
        &self,
        storage: &(impl Storage + Sync),
        remote: &impl Remote,
        host: Option<&Host>,
    ) -> Res<GcReport> {
        let _held = self
            .lock_every_package(storage, &self.installed_packages(storage).await?)
            .await?;
        // Before the manifests: an object filed after them is no candidate.
        let mut unused: BTreeMap<String, (PathBuf, u64)> = list_files(storage, &self.objects_dir())
            .await?
            .into_iter()
            .map(|(name, path, len)| (name, (path, len)))
            .collect();

        // Listed again under the locks: a writer that finished before they
        // were taken may have added a package.
        let mut manifests = Vec::new();
        for package in self.installed_packages(storage).await? {
            manifests.extend(list_files(storage, &package).await?);
        }
        manifests.extend(list_files(storage, &self.dir.join(PACKAGES_DIR)).await?);
        manifests.retain(|(hash, _, _)| is_top_hash(hash));

        let hashes: BTreeSet<&String> = manifests.iter().map(|(hash, _, _)| hash).collect();
        // With no answer from the bucket, nothing shows it can be reached.
        if hashes.is_empty() {
            return Ok(GcReport::default());
        }
        let mut published = BTreeSet::new();
        for hash in hashes {
            let manifest = S3Uri {
                bucket: self.bucket.clone(),
                key: get_manifest_key(hash),
                version: None,
            };
            if remote.exists(host, &manifest).await? {
                published.insert(hash.clone());
            }
        }
        let (cleared, kept): (Vec<_>, Vec<_>) = manifests
            .into_iter()
            .partition(|(hash, _, _)| published.contains(hash));

        // Every copy, even of one hash: the top hash leaves out a row's
        // physical key, so two copies can name different objects.
        for (_, path, _) in &kept {
            if unused.is_empty() {
                break;
            }
            for row in Manifest::from_path(storage, path).await?.rows {
                for name in row_objects(&row.hash, &row.physical_key) {
                    unused.remove(&name);
                }
            }
        }

        let mut report = GcReport::default();
        for (_, path, len) in cleared {
            storage.remove_file(&path).await?;
            report.cached_manifests += 1;
            report.bytes += len;
        }
        for (path, len) in unused.into_values() {
            storage.remove_file(&path).await?;
            report.objects += 1;
            report.bytes += len;
        }
        Ok(report)
    }

    /// The folders under `installed/`, one per package.
    async fn installed_packages(&self, storage: &(impl Storage + Sync)) -> Res<Vec<PathBuf>> {
        let mut packages = Vec::new();
        for owner in list_dirs(storage, &self.dir.join(INSTALLED_DIR)).await? {
            packages.extend(list_dirs(storage, &owner).await?);
        }
        Ok(packages)
    }

    /// Try-locks every package in `packages`, the `installed/` folders, and
    /// every package with a lock file, as `flow::gc` does: a writer files
    /// objects before the manifest that names them, so clearing beside one
    /// could drop them.
    async fn lock_every_package(
        &self,
        storage: &(impl Storage + Sync),
        packages: &[PathBuf],
    ) -> Res<Vec<LockGuard>> {
        let mut namespaces = locked_namespaces(storage, &self.dir.join(LOCKS_DIR)).await?;
        namespaces.extend(packages.iter().filter_map(|package| namespace_of(package)));
        let mut held = Vec::new();
        for namespace in namespaces {
            match storage
                .try_lock_exclusive(self.package_lock(&namespace))
                .await?
            {
                Some(guard) => held.push(guard),
                None => return Err(Error::PackageBusy(namespace)),
            }
        }
        Ok(held)
    }
}

/// The registry on this machine for packages with no bucket, laid out as a
/// bucket's store is, but a registry in its own right: `latest/` holds its
/// tags and `packages/` its revisions. Nothing in it is cleared on its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalRegistry {
    dir: PathBuf,
}

/// The name a local registry keeps, and the names it had before, which
/// still name it.
#[derive(Clone, Debug, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
pub struct LocalNames {
    pub name: String,
    pub aliases: Vec<String>,
}

impl LocalNames {
    /// Whether `name` is the registry's name or one of its aliases.
    #[must_use]
    pub fn answers_to(&self, name: &str) -> bool {
        self.name == name || self.aliases.iter().any(|alias| alias == name)
    }
}

/// What a publish to the local registry did with `latest`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LocalPublish {
    /// `latest` names the revision: it was the base, or already the
    /// revision.
    Advanced,
    /// `latest` had moved from the base, to the hash held here, or `None`
    /// where the package has no `latest`. The revision is kept and the tag
    /// left.
    Diverged(Option<String>),
}

impl LocalRegistry {
    #[must_use]
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// The file holding the hash `namespace`'s `latest` names.
    #[must_use]
    pub fn latest(&self, namespace: &Namespace) -> PathBuf {
        package_path(&self.dir.join(LATEST_DIR), namespace, "")
    }

    /// A revision published to the registry.
    #[must_use]
    pub fn revision(&self, hash: &str) -> PathBuf {
        self.dir.join(PACKAGES_DIR).join(hash)
    }

    /// Where `namespace`'s manifests installed or committed here go,
    /// unpublished commits included.
    #[must_use]
    pub fn installed_manifests_dir(&self, namespace: &Namespace) -> PathBuf {
        package_path(&self.dir.join(INSTALLED_DIR), namespace, "")
    }

    #[must_use]
    pub fn installed_manifest(&self, namespace: &Namespace, hash: &str) -> PathBuf {
        self.installed_manifests_dir(namespace).join(hash)
    }

    #[must_use]
    pub fn package_lock(&self, namespace: &Namespace) -> PathBuf {
        package_path(&self.dir.join(LOCKS_DIR), namespace, ".lock")
    }

    /// The hash `namespace`'s `latest` names, or `None` where it has none.
    /// Unlike a bucket's cached index, a tag holding no top hash is an
    /// error: it is the registry's own, and nothing else can replace it.
    pub async fn read_latest(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
    ) -> Res<Option<String>> {
        let path = self.latest(namespace);
        match read_tag(storage, &path).await? {
            Some(hash) if !is_top_hash(&hash) => Err(UriError::Package(format!(
                "{} holds no top hash: {hash:?}",
                path.display()
            ))
            .into()),
            tag => Ok(tag),
        }
    }

    /// The registry's names. The first call names it from this machine's
    /// host name, its other characters made `-`, and that name is kept.
    pub async fn names(&self, storage: &(impl Storage + Sync)) -> Res<LocalNames> {
        if let Some(names) = self.read_names(storage).await? {
            return Ok(names);
        }
        let _held = self.lock_names(storage).await?;
        self.names_held(storage).await
    }

    /// Renames the registry to `name`, keeping its old name as an alias. A
    /// registry not yet named takes `name` as its first.
    pub async fn rename(&self, storage: &(impl Storage + Sync), name: &str) -> Res<LocalNames> {
        check_name(name)?;
        let _held = self.lock_names(storage).await?;
        let Some(mut names) = self.read_names(storage).await? else {
            let names = LocalNames {
                name: name.to_string(),
                aliases: Vec::new(),
            };
            self.write_names(storage, &names).await?;
            return Ok(names);
        };
        if names.name != name {
            let old = std::mem::replace(&mut names.name, name.to_string());
            names.aliases.retain(|alias| alias != name);
            names.aliases.push(old);
            self.write_names(storage, &names).await?;
        }
        Ok(names)
    }

    /// [`names`](Self::names), with the names' lock held.
    async fn names_held(&self, storage: &(impl Storage + Sync)) -> Res<LocalNames> {
        if let Some(names) = self.read_names(storage).await? {
            return Ok(names);
        }
        let host = hostname::get()
            .ok()
            .and_then(|host| host.into_string().ok())
            .unwrap_or_default();
        let name = name_from_host(&host).ok_or_else(|| {
            std::io::Error::other(format!(
                "can't name the local registry after this machine's host name {host:?}; give it a name with a rename"
            ))
        })?;
        let names = LocalNames {
            name,
            aliases: Vec::new(),
        };
        self.write_names(storage, &names).await?;
        Ok(names)
    }

    /// Stores the revision `hash`, committed here for `namespace`, and moves
    /// `latest` to it only if `latest` still names `base`, `None` being no
    /// `latest` at all, or names `hash` already. All of it is done holding
    /// the package's lock, so the caller must not hold it.
    pub async fn publish(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
        base: Option<&str>,
        hash: &str,
    ) -> Res<LocalPublish> {
        if let Some(hash) = std::iter::once(hash).chain(base).find(|h| !is_top_hash(h)) {
            return Err(UriError::Package(format!("not a top hash: {hash:?}")).into());
        }
        let _held = storage.lock_exclusive(self.package_lock(namespace)).await?;
        let latest = self.read_latest(storage, namespace).await?;
        let from_base = latest.as_deref() == base;
        // A publish from the base writes even over a revision of the same
        // hash, which leaves out folder metadata, so the one published is
        // the one committed; any other only keeps a revision not yet stored.
        if from_base || !storage.exists(self.revision(hash)).await {
            let manifest = storage
                .read_bytes(self.installed_manifest(namespace, hash))
                .await?;
            storage
                .write_byte_stream(self.revision(hash), ByteStream::from(manifest))
                .await?;
        }

        // `hash` already being `latest` is a publish that landed before.
        if !from_base && latest.as_deref() != Some(hash) {
            self.sync_up_from(storage, &self.dir).await;
            return Ok(LocalPublish::Diverged(latest));
        }
        if from_base {
            // The revision this publish wrote, and the folders holding it,
            // are durable before `latest` names it.
            self.sync_objects(storage, hash).await?;
            self.sync_up_from(storage, &self.dir).await;
        }
        if latest.as_deref() != Some(hash) {
            write_tag(storage, &self.dir, &self.latest(namespace), hash).await?;
        }
        self.sync_up_from(storage, &self.dir.join(LATEST_DIR)).await;
        Ok(LocalPublish::Advanced)
    }

    /// Syncs the objects the revision `hash` names in `objects/`, which a
    /// commit files without syncing, before `latest` names them.
    async fn sync_objects(&self, storage: &(impl Storage + Sync), hash: &str) -> Res {
        // Windows syncs only a file opened for writing, and its folders not
        // at all, as `Storage::sync_dir` does not.
        if !cfg!(unix) {
            return Ok(());
        }
        let objects = self.objects_dir();
        let names: BTreeSet<String> = Manifest::from_path(storage, &self.revision(hash))
            .await?
            .rows
            .iter()
            .flat_map(|row| row_objects(&row.hash, &row.physical_key))
            .collect();
        for name in names {
            let object = objects.join(name);
            if storage.exists(&object).await {
                storage.open_file(&object).await?.sync_all().await?;
            }
        }
        storage.sync_dir(&objects).await;
        Ok(())
    }

    /// Syncs `folder` and each folder above it up to the Domain's root: the
    /// registry holds the only copy, so folders any write may have made,
    /// this one or another's, must survive a crash too.
    async fn sync_up_from(&self, storage: &(impl Storage + Sync), folder: &Path) {
        let root = self.dir.ancestors().nth(2).unwrap_or(&self.dir);
        for folder in folder
            .ancestors()
            .take_while(|folder| folder.starts_with(root))
        {
            storage.sync_dir(folder).await;
        }
    }

    async fn lock_names(&self, storage: &(impl Storage + Sync)) -> Res<LockGuard> {
        storage.lock_exclusive(self.dir.join(NAME_LOCK)).await
    }

    async fn read_names(&self, storage: &(impl Storage + Sync)) -> Res<Option<LocalNames>> {
        match storage.read_bytes(self.dir.join(NAME_FILE)).await {
            Ok(bytes) => Ok(Some(serde_json::from_slice(&bytes)?)),
            Err(err) if err.is_not_found() => Ok(None),
            Err(err) => Err(err),
        }
    }

    async fn write_names(&self, storage: &(impl Storage + Sync), names: &LocalNames) -> Res {
        let body = ByteStream::from(serde_json::to_vec(names)?);
        storage
            .write_byte_stream(self.dir.join(NAME_FILE), body)
            .await?;
        self.sync_up_from(storage, &self.dir).await;
        Ok(())
    }
}

impl PackageStore for LocalRegistry {
    fn objects_dir(&self) -> PathBuf {
        self.dir.join(OBJECTS_DIR)
    }

    fn manifests_dir(&self, namespace: &Namespace) -> PathBuf {
        self.installed_manifests_dir(namespace)
    }
}

/// `host` held to a local registry name's characters, each other one made
/// `-`; `None` where that is no name or holds no letter or digit, as for an
/// empty host name or one written in another script.
fn name_from_host(host: &str) -> Option<String> {
    let name: String = host
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c
            } else {
                '-'
            }
        })
        .collect();
    check_name(&name).is_ok().then_some(name)
}

/// A name a local registry may keep: a valid name in a URI, holding at
/// least one letter or digit.
fn check_name(name: &str) -> Res {
    quilt_uri::check_local_registry_name(name)?;
    if !name.bytes().any(|b| b.is_ascii_alphanumeric()) {
        return Err(UriError::Package(format!(
            "a local registry's name holds a letter or digit; got {name:?}"
        ))
        .into());
    }
    Ok(())
}

/// The namespace `<owner>/<name>` that the last two parts of `path` name, or
/// `None` where they name none.
fn namespace_of(path: &Path) -> Option<Namespace> {
    let part = |path: Option<&Path>| {
        path.and_then(Path::file_name)
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    };
    Namespace::try_from(format!("{}/{}", part(path.parent()), part(Some(path)))).ok()
}

impl PackageStore for RegistryStore {
    fn objects_dir(&self) -> PathBuf {
        self.dir.join(OBJECTS_DIR)
    }

    fn manifests_dir(&self, namespace: &Namespace) -> PathBuf {
        self.installed_manifests_dir(namespace)
    }
}

/// `<dir>/<owner>/<name><suffix>`, one folder per part on every OS.
fn package_path(dir: &Path, namespace: &Namespace, suffix: &str) -> PathBuf {
    dir.join(namespace.prefix())
        .join(format!("{}{suffix}", namespace.name()))
}

/// What the tag file at `path` holds, trimmed, or `None` where it is missing.
async fn read_tag(storage: &(impl Storage + Sync), path: &Path) -> Res<Option<String>> {
    match storage.read_bytes(path).await {
        Ok(bytes) => Ok(Some(String::from_utf8_lossy(&bytes).trim().to_string())),
        Err(err) if err.is_not_found() => Ok(None),
        Err(err) => Err(err),
    }
}

/// Points the tag file at `path` at `hash`, staged in `store_dir`, outside
/// the tag's folder, where every file is a package's.
async fn write_tag(
    storage: &(impl Storage + Sync),
    store_dir: &Path,
    path: &Path,
    hash: &str,
) -> Res {
    let owner = path.parent().unwrap_or(store_dir);
    storage.create_dir_all(owner).await?;
    let staged = store_dir.join(temp_name());
    storage
        .write_staged_byte_stream(&staged, ByteStream::from(hash.as_bytes().to_vec()))
        .await?;
    storage.rename(&staged, path).await?;
    storage.sync_dir(owner).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use bytes::Bytes;
    use test_log::test;

    use crate::LogicalKey;
    use crate::flow::WorkflowCheck;
    use crate::flow::record;
    use crate::io::manifest::RowContent;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::LocalStorage;
    use crate::lineage::PackageLineage;
    use crate::manifest::Manifest;
    use crate::manifest::ManifestRow;
    use crate::object_hash::ObjectHash;
    use quilt_uri::S3Uri;
    use quilt_uri::paths::tag_key;

    fn hash(n: u8) -> String {
        format!("{n:02x}").repeat(32)
    }

    /// Sets the bucket's `latest` for `namespace` to `contents`.
    async fn tag(remote: &MockRemote, bucket: &str, namespace: &Namespace, contents: &str) -> Res {
        let uri = S3Uri {
            bucket: bucket.to_string(),
            key: tag_key(namespace, "latest"),
            version: None,
        };
        remote
            .put_object(None, &uri, contents.as_bytes().to_vec())
            .await
    }

    /// The index holds what the registry lists after a refresh: a moved
    /// `latest` is updated, new packages added, and a package the registry no
    /// longer has dropped, names that look like a temp file's among them.
    #[test(tokio::test)]
    async fn a_refreshed_index_holds_what_the_registry_lists() -> Res {
        let dir = tempfile::tempdir()?;
        let store = DomainRoot::new(dir.path()).registry("bucket")?;
        let storage = LocalStorage::new();
        let remote = MockRemote::default();
        let (moved, added, dotted, gone) = (
            Namespace::from(("acme", "moved")),
            Namespace::from(("acme", "added")),
            Namespace::from(("acme", ".tmp-dotted")),
            Namespace::from(("other", ".tmp-gone")),
        );
        store.write_latest(&storage, &moved, &hash(1)).await?;
        store.write_latest(&storage, &gone, &hash(2)).await?;
        tag(&remote, "bucket", &moved, &hash(3)).await?;
        tag(&remote, "bucket", &added, &hash(4)).await?;
        tag(&remote, "bucket", &dotted, &hash(5)).await?;
        tag(&remote, "other-bucket", &gone, &hash(2)).await?;

        let refreshed = store.refresh_latest_index(&storage, &remote, None).await?;

        let expected = BTreeMap::from([
            (moved.clone(), hash(3)),
            (added, hash(4)),
            (dotted, hash(5)),
        ]);
        assert_eq!(refreshed, expected);
        assert_eq!(store.latest_index(&storage).await?, expected);
        assert_eq!(store.read_latest(&storage, &moved).await?, Some(hash(3)));
        assert_eq!(store.read_latest(&storage, &gone).await?, None);
        Ok(())
    }

    /// A listing that fails partway changes nothing: the index is not left
    /// holding part of the registry.
    #[test(tokio::test)]
    async fn a_failed_listing_leaves_the_index_alone() -> Res {
        let dir = tempfile::tempdir()?;
        let store = DomainRoot::new(dir.path()).registry("bucket")?;
        let storage = LocalStorage::new();
        let remote = MockRemote::default();
        let (kept, listed) = (
            Namespace::from(("acme", "kept")),
            Namespace::from(("acme", "listed")),
        );
        store.write_latest(&storage, &kept, &hash(1)).await?;
        tag(&remote, "bucket", &listed, &hash(2)).await?;
        let unreadable = Namespace::from(("zzz", "unreadable"));
        tag(&remote, "bucket", &unreadable, &hash(3)).await?;
        remote.deny_reads(&format!("s3://bucket/{}", tag_key(&unreadable, "latest")));

        assert!(
            store
                .refresh_latest_index(&storage, &remote, None)
                .await
                .is_err()
        );
        assert_eq!(
            store.latest_index(&storage).await?,
            BTreeMap::from([(kept, hash(1))])
        );
        Ok(())
    }

    /// Records `files` as a revision of `namespace` in `store`; returns its
    /// top hash and its rows' object paths, in key order.
    async fn commit(
        store: &impl PackageStore,
        namespace: &Namespace,
        files: &[(&str, &'static [u8])],
    ) -> Res<(String, Vec<PathBuf>)> {
        let storage = LocalStorage::new();
        let mut rows = Vec::new();
        let mut content = BTreeMap::new();
        for (name, bytes) in files {
            let key = LogicalKey::new(*name)?;
            rows.push(ManifestRow {
                logical_key: key.clone(),
                ..ManifestRow::default()
            });
            content.insert(key, RowContent::Bytes(Bytes::from_static(bytes)));
        }
        let manifest = Manifest {
            rows,
            ..Manifest::default()
        };
        let (_, recorded) = record(
            store,
            &storage,
            &MockRemote::default(),
            WorkflowCheck::Warn { host: None },
            namespace,
            PackageLineage::default(),
            manifest,
            &content,
        )
        .await?;
        let hash = recorded.hash().expect("a revision").to_string();
        let written =
            Manifest::from_path(&storage, &store.manifests_dir(namespace).join(&hash)).await?;
        let objects = written
            .rows
            .iter()
            .map(|row| store.object_path(&row.hash))
            .collect();
        Ok((hash, objects))
    }

    /// Marks the manifest `hash` as held by the bucket.
    async fn publish(remote: &MockRemote, store: &RegistryStore, hash: &str) -> Res {
        let uri = S3Uri {
            bucket: store.bucket().to_string(),
            key: quilt_uri::paths::get_manifest_key(hash),
            version: None,
        };
        remote.put_object(None, &uri, b"{}".to_vec()).await
    }

    /// The bucket holds one revision and lacks another, the unpushed commit.
    /// Clearing drops the published manifest, installed and cached, and the
    /// object only it used; the commit keeps its manifest and every object
    /// it uses, one shared with the published revision.
    #[test(tokio::test)]
    async fn clearing_keeps_unpushed_work_and_drops_what_the_bucket_has() -> Res {
        let dir = tempfile::tempdir()?;
        let store = DomainRoot::new(dir.path()).registry("bucket")?;
        let storage = LocalStorage::new();
        let remote = MockRemote::default();
        let namespace = Namespace::from(("acme", "demo"));
        let (pushed, pushed_objects) = commit(
            &store,
            &namespace,
            &[("only.txt", b"pushed"), ("shared.txt", b"both")],
        )
        .await?;
        let (unpushed, unpushed_objects) = commit(
            &store,
            &namespace,
            &[("mine.txt", b"local"), ("shared.txt", b"both")],
        )
        .await?;
        publish(&remote, &store, &pushed).await?;
        storage
            .create_dir_all(store.cached_manifest(&pushed).parent().unwrap())
            .await?;
        storage
            .copy(
                store.installed_manifest(&namespace, &pushed),
                store.cached_manifest(&pushed),
            )
            .await?;

        let report = store.clear_published(&storage, &remote, None).await?;

        assert!(!store.installed_manifest(&namespace, &pushed).exists());
        assert!(!store.cached_manifest(&pushed).exists());
        assert!(!pushed_objects[0].exists(), "used only by what was pushed");
        assert!(store.installed_manifest(&namespace, &unpushed).exists());
        for object in &unpushed_objects {
            assert!(object.exists(), "{} is the commit's", object.display());
        }
        assert_eq!((report.cached_manifests, report.objects), (2, 1));
        Ok(())
    }

    /// Two unpushed copies of one revision whose rows name different objects
    /// for the same bytes, as a row carried forward under an older object
    /// name does: clearing keeps both objects.
    #[test(tokio::test)]
    async fn clearing_keeps_every_object_each_copy_of_a_commit_names() -> Res {
        let dir = tempfile::tempdir()?;
        let store = DomainRoot::new(dir.path()).registry("bucket")?;
        let storage = LocalStorage::new();
        let (first, second) = (
            Namespace::from(("acme", "a")),
            Namespace::from(("acme", "b")),
        );
        let (hash, objects) = commit(&store, &first, &[("a.txt", b"a")]).await?;
        let older = store.objects_dir().join("older-name");
        std::fs::copy(&objects[0], &older)?;
        let mut copy =
            Manifest::from_path(&storage, &store.installed_manifest(&first, &hash)).await?;
        copy.rows[0].physical_key = crate::paths::file_url(&older)?;
        let copy_path = store.installed_manifest(&second, &hash);
        std::fs::create_dir_all(copy_path.parent().unwrap())?;
        std::fs::write(&copy_path, copy.try_to_jsonlines()?)?;

        store
            .clear_published(&storage, &MockRemote::default(), None)
            .await?;

        assert!(objects[0].exists());
        assert!(older.exists(), "the second copy's object is kept");
        Ok(())
    }

    /// Objects with no manifest, as a record killed before its manifest
    /// leaves, are kept when there is nothing to ask the bucket about: an
    /// unreachable bucket would go unnoticed.
    #[test(tokio::test)]
    async fn clearing_a_store_with_no_manifest_clears_nothing() -> Res {
        let dir = tempfile::tempdir()?;
        let store = DomainRoot::new(dir.path()).registry("bucket")?;
        let storage = LocalStorage::new();
        let orphan = store.objects_dir().join("orphan");
        std::fs::create_dir_all(store.objects_dir())?;
        std::fs::write(&orphan, b"x")?;

        let report = store
            .clear_published(&storage, &MockRemote::default(), None)
            .await?;

        assert!(orphan.exists());
        assert!(report.is_empty());
        Ok(())
    }

    /// When the bucket cannot be asked about a manifest, nothing is cleared:
    /// what it holds is unknown.
    #[test(tokio::test)]
    async fn clearing_when_the_bucket_cannot_answer_clears_nothing() -> Res {
        let dir = tempfile::tempdir()?;
        let store = DomainRoot::new(dir.path()).registry("bucket")?;
        let storage = LocalStorage::new();
        let remote = MockRemote::default();
        let namespace = Namespace::from(("acme", "demo"));
        let (pushed, pushed_objects) = commit(&store, &namespace, &[("a.txt", b"a")]).await?;
        let (unknown, _) = commit(&store, &namespace, &[("b.txt", b"b")]).await?;
        publish(&remote, &store, &pushed).await?;
        remote.deny_reads(&format!(
            "s3://bucket/{}",
            quilt_uri::paths::get_manifest_key(&unknown)
        ));

        assert!(
            store
                .clear_published(&storage, &remote, None)
                .await
                .is_err()
        );
        assert!(store.installed_manifest(&namespace, &pushed).exists());
        assert!(pushed_objects[0].exists());
        Ok(())
    }

    /// A package whose lock another writer holds may be filing objects, so
    /// clearing refuses and leaves everything.
    #[test(tokio::test)]
    async fn clearing_beside_a_busy_package_clears_nothing() -> Res {
        let dir = tempfile::tempdir()?;
        let store = DomainRoot::new(dir.path()).registry("bucket")?;
        let storage = LocalStorage::new();
        let remote = MockRemote::default();
        let namespace = Namespace::from(("acme", "demo"));
        let (pushed, _) = commit(&store, &namespace, &[("a.txt", b"a")]).await?;
        publish(&remote, &store, &pushed).await?;
        let busy = Namespace::from(("acme", "busy"));
        let lock = store.package_lock(&busy);
        storage.create_dir_all(lock.parent().unwrap()).await?;
        let _held = storage.lock_exclusive(&lock).await?;

        let cleared = store.clear_published(&storage, &remote, None).await;

        assert!(matches!(cleared, Err(Error::PackageBusy(n)) if n == busy));
        assert!(store.installed_manifest(&namespace, &pushed).exists());
        Ok(())
    }

    /// An index file holding anything but a top hash says nothing.
    #[test(tokio::test)]
    async fn an_index_file_holding_no_top_hash_is_unknown() -> Res {
        let dir = tempfile::tempdir()?;
        let store = DomainRoot::new(dir.path()).registry("bucket")?;
        let storage = LocalStorage::new();
        let namespace = Namespace::from(("acme", "demo"));
        store
            .write_latest(&storage, &namespace, "not a hash")
            .await?;

        assert_eq!(store.read_latest(&storage, &namespace).await?, None);
        assert_eq!(store.latest_index(&storage).await?, BTreeMap::new());
        Ok(())
    }

    /// The layout quilt-specs#299 rules for a Package Store, structural
    /// because the layout is the contract the import and other clients read.
    #[test]
    fn a_registry_store_is_laid_out_under_its_bucket() -> Res {
        let root = DomainRoot::new("/data/bio.quilt.dataspace");
        let store = root.registry("my-bucket")?;
        let namespace = Namespace::from(("acme", "demo"));
        let bucket_dir = Path::new("/data/bio.quilt.dataspace/registries/my-bucket");

        assert_eq!(store.dir(), bucket_dir);
        assert_eq!(
            store.latest(&namespace),
            bucket_dir.join("latest/acme/demo")
        );
        assert_eq!(
            store.cached_manifest("deadbeef"),
            bucket_dir.join("packages/deadbeef")
        );
        assert_eq!(
            store.installed_manifest(&namespace, "deadbeef"),
            bucket_dir.join("installed/acme/demo/deadbeef")
        );
        assert_eq!(store.objects_dir(), bucket_dir.join("objects"));
        assert_eq!(
            store.package_lock(&namespace),
            bucket_dir.join("locks/acme/demo.lock")
        );
        Ok(())
    }

    /// `@local` is the local registry's, and a name holding a separator or
    /// `..` would leave `registries/`.
    #[test]
    fn a_name_no_bucket_can_have_names_no_store() {
        let root = DomainRoot::new("/data");
        for name in ["@local", "..", "a/b", r"a\b", ""] {
            assert!(root.registry(name).is_err(), "{name:?} is refused");
        }
    }

    /// Legacy buckets whose folder a drive would merge with `mybucket`'s,
    /// by ignoring case or, on Windows, a trailing dot, have no store; other
    /// legacy names have one.
    #[test]
    fn a_bucket_a_drive_would_merge_with_another_has_no_store() {
        let root = DomainRoot::new("/data");
        assert!(root.registry("MyBucket").is_err());
        assert!(root.registry("mybucket.").is_err());
        assert!(root.registry("my_legacy.bucket").is_ok());
    }

    /// The names Windows keeps for devices, with or without an extension,
    /// and none that only begin like one.
    #[test]
    fn windows_device_names_are_known() {
        for name in ["con", "nul", "aux.data", "com1", "lpt9.x", "prn"] {
            assert!(is_windows_device_name(name), "{name}");
        }
        for name in ["console", "com10", "comx", "nul-bucket", "my.con"] {
            assert!(!is_windows_device_name(name), "{name}");
        }
    }

    #[cfg(windows)]
    #[test]
    fn a_bucket_windows_takes_for_a_device_has_no_store_there() {
        let root = DomainRoot::new(r"C:\data");
        assert!(root.registry("con").is_err());
        assert!(root.registry("console").is_ok());
    }

    /// The layout the local registry is given beside the buckets' stores,
    /// structural because other clients and the import read it.
    #[test]
    fn the_local_registry_is_laid_out_under_at_local() {
        let local = DomainRoot::new("/data/bio.quilt.dataspace").local();
        let namespace = Namespace::from(("me", "analysis"));
        let dir = Path::new("/data/bio.quilt.dataspace/registries/@local");

        assert_eq!(local.dir(), dir);
        assert_eq!(local.latest(&namespace), dir.join("latest/me/analysis"));
        assert_eq!(local.revision("deadbeef"), dir.join("packages/deadbeef"));
        assert_eq!(
            local.installed_manifest(&namespace, "deadbeef"),
            dir.join("installed/me/analysis/deadbeef")
        );
        assert_eq!(local.objects_dir(), dir.join("objects"));
        assert_eq!(
            local.package_lock(&namespace),
            dir.join("locks/me/analysis.lock")
        );
    }

    /// A host name's characters outside a registry name's become `-`.
    #[test]
    fn a_host_name_is_held_to_registry_name_characters() {
        assert_eq!(
            name_from_host("Lab's MacBook_Pro.local"),
            Some("Lab-s-MacBook-Pro.local".to_string())
        );
        assert_eq!(name_from_host(""), None);
        assert_eq!(name_from_host(".."), None);
        assert_eq!(name_from_host("日本"), None);
    }

    /// The registry's first name comes from the host name, and is stored.
    #[test(tokio::test)]
    async fn the_local_registry_is_first_named_after_the_host() -> Res {
        let dir = tempfile::tempdir()?;
        let storage = LocalStorage::new();
        let host = hostname::get()?.into_string().expect("a UTF-8 host name");

        let first = DomainRoot::new(dir.path()).local().names(&storage).await?;
        assert_eq!(
            first,
            LocalNames {
                name: name_from_host(&host).expect("a host name"),
                aliases: Vec::new(),
            }
        );
        assert!(dir.path().join("registries/@local/registry.json").exists());
        Ok(())
    }

    /// A first rename names the registry, with no alias. A later one keeps
    /// the old name as an alias; renaming back to an alias makes it the name
    /// again and the one before it an alias.
    #[test(tokio::test)]
    async fn a_renamed_registry_still_answers_to_its_old_name() -> Res {
        let dir = tempfile::tempdir()?;
        let storage = LocalStorage::new();
        let local = DomainRoot::new(dir.path()).local();
        assert_eq!(
            local.rename(&storage, "first").await?,
            LocalNames {
                name: "first".to_string(),
                aliases: Vec::new(),
            }
        );

        let names = local.rename(&storage, "second").await?;
        assert_eq!(names.name, "second");
        assert!(names.answers_to("first") && names.answers_to("second"));
        assert!(!names.answers_to("third"));

        let names = local.rename(&storage, "first").await?;
        assert_eq!(names.name, "first");
        assert!(names.answers_to("second"));
        assert!(!names.aliases.contains(&"first".to_string()));
        assert_eq!(local.names(&storage).await?, names);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_name_outside_host_name_characters_is_refused() -> Res {
        let dir = tempfile::tempdir()?;
        let storage = LocalStorage::new();
        let local = DomainRoot::new(dir.path()).local();
        assert!(local.rename(&storage, "my registry").await.is_err());
        assert!(local.rename(&storage, "--").await.is_err());
        Ok(())
    }

    /// A publish advances `latest` only from the revision it names: the
    /// second publish from no `latest` finds it moved, keeps its revision and
    /// leaves the tag; one from the new `latest` advances it.
    #[test(tokio::test)]
    async fn a_local_publish_advances_latest_only_from_its_base() -> Res {
        let dir = tempfile::tempdir()?;
        let storage = LocalStorage::new();
        let local = DomainRoot::new(dir.path()).local();
        let namespace = Namespace::from(("me", "analysis"));
        let (one, _) = commit(&local, &namespace, &[("a.txt", b"one")]).await?;
        let (two, _) = commit(&local, &namespace, &[("a.txt", b"two")]).await?;
        let (three, _) = commit(&local, &namespace, &[("a.txt", b"three")]).await?;

        assert_eq!(
            local.publish(&storage, &namespace, None, &one).await?,
            LocalPublish::Advanced
        );
        assert_eq!(
            local.publish(&storage, &namespace, None, &two).await?,
            LocalPublish::Diverged(Some(one.clone()))
        );
        assert_eq!(
            local.read_latest(&storage, &namespace).await?,
            Some(one.clone())
        );
        assert!(
            local.revision(&two).exists(),
            "the diverged revision is kept"
        );
        assert_eq!(
            local.publish(&storage, &namespace, None, &one).await?,
            LocalPublish::Advanced,
            "a publish retried after it landed"
        );
        assert!(
            local
                .publish(&storage, &namespace, Some("abc123"), &three)
                .await
                .is_err(),
            "a hash prefix is not a base"
        );

        assert_eq!(
            local
                .publish(&storage, &namespace, Some(&one), &three)
                .await?,
            LocalPublish::Advanced
        );
        assert_eq!(
            local.read_latest(&storage, &namespace).await?,
            Some(three.clone())
        );
        assert!(local.revision(&one).exists() && local.revision(&three).exists());
        Ok(())
    }

    /// A local tag holding no top hash is damaged, not absent: a publish
    /// from no `latest` fails and leaves it.
    #[test(tokio::test)]
    async fn a_damaged_local_tag_is_not_taken_for_none() -> Res {
        let dir = tempfile::tempdir()?;
        let storage = LocalStorage::new();
        let local = DomainRoot::new(dir.path()).local();
        let namespace = Namespace::from(("me", "analysis"));
        let (hash, _) = commit(&local, &namespace, &[("a.txt", b"one")]).await?;
        let tag = local.latest(&namespace);
        std::fs::create_dir_all(tag.parent().unwrap())?;
        std::fs::write(&tag, "not a hash")?;

        assert!(
            local
                .publish(&storage, &namespace, None, &hash)
                .await
                .is_err()
        );
        assert_eq!(std::fs::read_to_string(&tag)?, "not a hash");
        Ok(())
    }

    /// A revision published again from the base is written again: its hash
    /// leaves out folder metadata, so the copy committed last is the one
    /// kept.
    #[test(tokio::test)]
    async fn a_revision_published_again_is_rewritten() -> Res {
        let dir = tempfile::tempdir()?;
        let storage = LocalStorage::new();
        let local = DomainRoot::new(dir.path()).local();
        let namespace = Namespace::from(("me", "analysis"));
        let (hash, _) = commit(&local, &namespace, &[("data/a.txt", b"one")]).await?;
        local.publish(&storage, &namespace, None, &hash).await?;
        let recommitted = local.installed_manifest(&namespace, &hash);
        // A copy that differs where the top hash does not look.
        let mut other = Manifest::from_path(&storage, &recommitted).await?;
        other.dir_meta = vec![crate::manifest::DirMeta::new(
            LogicalKey::new("data")?,
            serde_json::json!({ "copy": "other" }),
        )];
        let mut hasher = crate::manifest::TopHasher::new();
        hasher.append_header(&other.header)?;
        for row in &other.rows {
            hasher.append(row)?;
        }
        assert_eq!(hasher.finalize(), hash, "folder metadata is not hashed");
        let other = other.try_to_jsonlines()?;
        std::fs::write(&recommitted, &other)?;

        local
            .publish(&storage, &namespace, Some(&hash), &hash)
            .await?;

        assert_eq!(std::fs::read_to_string(local.revision(&hash))?, other);

        // One not from the base leaves a stored revision as it is.
        std::fs::write(&recommitted, b"from a stale base")?;
        local.publish(&storage, &namespace, None, &hash).await?;
        assert_eq!(std::fs::read_to_string(local.revision(&hash))?, other);
        Ok(())
    }

    /// A publish stores its revision, and checks and moves `latest`, only
    /// once it holds the package's lock, so two publishers from one base
    /// cannot both advance it.
    #[test(tokio::test)]
    async fn a_local_publish_waits_for_the_package_lock() -> Res {
        let dir = tempfile::tempdir()?;
        let storage = LocalStorage::new();
        let local = DomainRoot::new(dir.path()).local();
        let namespace = Namespace::from(("me", "analysis"));
        let (hash, _) = commit(&local, &namespace, &[("a.txt", b"one")]).await?;
        let held = storage
            .lock_exclusive(local.package_lock(&namespace))
            .await?;

        let publish = local.publish(&storage, &namespace, None, &hash);
        tokio::pin!(publish);
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(200), &mut publish)
                .await
                .is_err(),
            "the publish finished while another holder had the lock"
        );
        assert_eq!(local.read_latest(&storage, &namespace).await?, None);
        assert!(!local.revision(&hash).exists(), "the revision waits too");

        drop(held);
        assert_eq!(publish.await?, LocalPublish::Advanced);
        Ok(())
    }

    /// One package recorded into two buckets' stores: each store holds its
    /// own manifest and its own copy of the bytes.
    #[test(tokio::test)]
    async fn each_bucket_keeps_its_own_objects() -> Res {
        let dir = tempfile::tempdir()?;
        let root = DomainRoot::new(dir.path());
        let storage = LocalStorage::new();
        let namespace = Namespace::from(("acme", "demo"));
        let key = LogicalKey::new("a.txt")?;
        let files = BTreeMap::from([(key.clone(), RowContent::Bytes(Bytes::from_static(b"x")))]);
        let manifest = Manifest {
            rows: vec![ManifestRow {
                logical_key: key,
                hash: ObjectHash::default(),
                ..ManifestRow::default()
            }],
            ..Manifest::default()
        };

        for bucket in ["one", "two"] {
            let store = root.registry(bucket)?;
            let (_, recorded) = record(
                &store,
                &storage,
                &MockRemote::default(),
                WorkflowCheck::Warn { host: None },
                &namespace,
                PackageLineage::default(),
                manifest.clone(),
                &files,
            )
            .await?;
            let hash = recorded.hash().expect("a revision");

            let written =
                Manifest::from_path(&storage, &store.installed_manifest(&namespace, hash)).await?;
            let object = store.object_path(&written.rows[0].hash);
            assert!(object.starts_with(dir.path().join("registries").join(bucket)));
            assert_eq!(std::fs::read(&object)?, b"x");
        }
        Ok(())
    }
}
