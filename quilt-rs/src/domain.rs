//! The Domain's storage: an ordinary folder, `bio.quilt.dataspace/`, holding
//! a Package Store for each remote registry under `registries/<bucket>/`.
//!
//! A [`LocalDomain`](crate::LocalDomain) still keeps its packages in
//! `.quilt/` ([`DomainPaths`](crate::paths::DomainPaths)); nothing moves here
//! until existing state is imported.

use std::path::Path;
use std::path::PathBuf;

use crate::Res;
use crate::flow::PackageStore;
use quilt_uri::Namespace;
use quilt_uri::check_bucket;

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
    /// outside `registries/` or one of the `@` stores beside the buckets.
    pub fn registry(&self, bucket: &str) -> Res<RegistryStore> {
        check_bucket(bucket)?;
        Ok(RegistryStore {
            bucket: bucket.to_string(),
            dir: self.root.join(REGISTRIES_DIR).join(bucket),
        })
    }
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

    use std::collections::BTreeMap;

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
