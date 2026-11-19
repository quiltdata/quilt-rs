//! The Domain's storage: an ordinary folder, `bio.quilt.dataspace/`, holding
//! a Package Store for each remote registry under `registries/<bucket>/`.
//!
//! A [`LocalDomain`](crate::LocalDomain) still keeps its packages in
//! `.quilt/` ([`DomainPaths`](crate::paths::DomainPaths)); nothing moves here
//! until existing state is imported.

use std::collections::BTreeMap;
use std::path::Path;
use std::path::PathBuf;

use aws_sdk_s3::primitives::ByteStream;

use crate::Res;
use crate::flow::PackageStore;
use crate::flow::list_dirs;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;
use quilt_uri::Host;
use quilt_uri::Namespace;
use quilt_uri::UriError;
use quilt_uri::check_bucket;
use quilt_uri::is_top_hash;

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
        if let Some(owner) = path.parent() {
            storage.create_dir_all(owner).await?;
        }
        storage
            .write_byte_stream(&path, ByteStream::from(hash.as_bytes().to_vec()))
            .await
    }

    /// Every package the latest index holds a hash for.
    pub async fn latest_index(
        &self,
        storage: &(impl Storage + Sync),
    ) -> Res<BTreeMap<Namespace, String>> {
        let mut index = BTreeMap::new();
        for owner_dir in list_dirs(storage, &self.dir.join(LATEST_DIR)).await? {
            let owner = owner_dir.file_name().unwrap_or_default().to_string_lossy();
            let mut entries = storage.read_dir(&owner_dir).await?;
            while let Some(entry) = entries.next_entry().await? {
                let name = entry.file_name().to_string_lossy().into_owned();
                // A package may be named `.foo`; a write's temp file may not.
                if name.starts_with(".tmp-") || !entry.file_type().await?.is_file() {
                    continue;
                }
                let Ok(namespace) = Namespace::try_from(format!("{owner}/{name}")) else {
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
    /// `latest` is updated, new packages added, a dotted name among them, and
    /// a package the registry no longer has dropped.
    #[test(tokio::test)]
    async fn a_refreshed_index_holds_what_the_registry_lists() -> Res {
        let dir = tempfile::tempdir()?;
        let store = DomainRoot::new(dir.path()).registry("bucket")?;
        let storage = LocalStorage::new();
        let remote = MockRemote::default();
        let (moved, added, dotted, gone) = (
            Namespace::from(("acme", "moved")),
            Namespace::from(("acme", "added")),
            Namespace::from(("acme", ".dotted")),
            Namespace::from(("other", "gone")),
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
