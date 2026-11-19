//! The Domain's storage: an ordinary folder, `bio.quilt.dataspace/`, holding
//! a Package Store for each remote registry under `registries/<bucket>/`.
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
        let bytes = match storage.read_bytes(self.latest(namespace)).await {
            Ok(bytes) => bytes,
            Err(err) if err.is_not_found() => return Ok(None),
            Err(err) => return Err(err),
        };
        let hash = String::from_utf8_lossy(&bytes).trim().to_string();
        Ok(is_top_hash(&hash).then_some(hash))
    }

    /// Records in the latest index that `latest` names `hash` for
    /// `namespace`.
    pub async fn write_latest(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
        hash: &str,
    ) -> Res {
        let path = self.latest(namespace);
        let owner = path.parent().unwrap_or(&self.dir);
        storage.create_dir_all(owner).await?;
        // Staged outside `latest/`, where every file is a package's.
        let staged = self.dir.join(temp_name());
        storage
            .write_staged_byte_stream(&staged, ByteStream::from(hash.as_bytes().to_vec()))
            .await?;
        storage.rename(&staged, &path).await?;
        storage.sync_dir(owner).await;
        Ok(())
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
    /// bucket cannot be asked about every manifest, as when offline, or
    /// while another writer holds a package's lock.
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
        store: &RegistryStore,
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
            Manifest::from_path(&storage, &store.installed_manifest(namespace, &hash)).await?;
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
