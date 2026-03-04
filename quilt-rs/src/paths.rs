//!
//! Incapsulated knowlegde about directory structure of the files in `.quilt`, packages and working directories.

use std::path::PathBuf;

#[cfg(test)]
use tempfile::TempDir;
use tracing::error;

use crate::io::storage::Storage;
use crate::lineage::Home;
use crate::uri::Host;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
use crate::Res;

pub const AUTH_CREDENTIALS: &str = "credentials.json";
pub const AUTH_DIR: &str = ".auth";
pub const AUTH_TOKENS: &str = "tokens.json";

const LINEAGE_FILE: &str = ".quilt/data.json";

const INSTALLED_DIR: &str = ".quilt/installed";
const MANIFEST_DIR: &str = ".quilt/packages";
const OBJECTS_DIR: &str = ".quilt/objects";

const TAGS_DIR: &str = ".quilt/named_packages";

/// Where do we store tagged "packages". Files that contain packages' hashes.
pub fn tag_key(namespace: &Namespace, tag: &str) -> String {
    format!("{TAGS_DIR}/{namespace}/{tag}")
}

/// What is the path to the JSONL manifest based on its `hash`
pub fn get_manifest_key_legacy(hash: &str) -> String {
    format!("{MANIFEST_DIR}/{hash}")
}

/// Path to the package home directory within the home directory
pub fn package_home(home: &Home, namespace: &Namespace) -> PathBuf {
    home.join(namespace.to_string())
}

/// Helper for getting paths.
/// We heavily rely on where we put files,
/// and this struct contains info of the directory structure .
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct DomainPaths {
    root_dir: PathBuf,
}

impl DomainPaths {
    pub fn new(root_dir: PathBuf) -> Self {
        DomainPaths { root_dir }
    }

    pub fn auth_host(&self, host: &Host) -> PathBuf {
        self.root_dir
            .join(AUTH_DIR)
            .join(PathBuf::from(host.to_string()))
    }

    /// Path to the installed manifest
    // TODO: pass `ManifestUri`
    pub fn installed_manifest(&self, namespace: &Namespace, hash: &str) -> PathBuf {
        self.installed_manifests_dir(namespace).join(hash)
    }

    /// Directory for storing installed manifests
    pub fn installed_manifests_dir(&self, namespace: &Namespace) -> PathBuf {
        self.root_dir
            .join(INSTALLED_DIR)
            .join(namespace.to_string())
    }

    /// Path to the lineage file
    pub fn lineage(&self) -> PathBuf {
        self.root_dir.join(LINEAGE_FILE)
    }

    /// Path to the manifest cached in semi-temporary directory
    // TODO: pass `ManifestUri`
    pub fn cached_manifest(&self, bucket: &str, hash: &str) -> PathBuf {
        self.root_dir.join(MANIFEST_DIR).join(bucket).join(hash)
    }

    /// Directory for storing cached manifests for a bucket
    pub fn cached_manifests_dir(&self, bucket: &str) -> PathBuf {
        self.root_dir.join(MANIFEST_DIR).join(bucket)
    }

    /// Directory for storing pristine hashed files
    pub fn objects_dir(&self) -> PathBuf {
        self.root_dir.join(OBJECTS_DIR)
    }

    /// Path to the pristine hashed file
    pub fn object(&self, hash: &[u8]) -> PathBuf {
        self.objects_dir().join(hex::encode(hash))
    }

    /// What directories are essential when we initiate `LocalDomain`
    fn required(&self) -> Vec<PathBuf> {
        vec![
            self.root_dir.join(INSTALLED_DIR),
            self.objects_dir(),
            self.root_dir.join(MANIFEST_DIR),
        ]
    }

    /// What directories are essential when we initiate `InstalledPackage`
    fn required_for_installing(&self, home: &Home, namespace: &Namespace) -> Res<Vec<PathBuf>> {
        let mut paths = vec![];
        paths.extend(self.required());
        paths.extend(vec![
            package_home(home, namespace),
            self.installed_manifests_dir(namespace),
        ]);
        Ok(paths)
    }

    pub async fn scaffold_for_installing(
        &self,
        storage: &impl Storage,
        home: &Home,
        namespace: &Namespace,
    ) -> Res {
        scaffold_paths(storage, self.required_for_installing(home, namespace)?).await
    }

    /// What directories are essential when we work with cached manifests
    fn required_for_caching(&self, bucket: &str) -> Vec<PathBuf> {
        let mut paths = vec![];
        paths.extend(self.required());
        paths.extend(vec![self.cached_manifests_dir(bucket)]);
        paths
    }

    pub async fn scaffold_for_caching(&self, storage: &impl Storage, bucket: &str) -> Res {
        scaffold_paths(storage, self.required_for_caching(bucket)).await
    }

    #[cfg(test)]
    pub fn from_temp_dir() -> Res<(Self, TempDir)> {
        let temp_dir = TempDir::new()?;
        Ok((DomainPaths::new(temp_dir.path().to_path_buf()), temp_dir))
    }
}

pub async fn copy_cached_to_installed(
    paths: &DomainPaths,
    storage: &impl Storage,
    manifest_uri: &ManifestUri,
) -> Res {
    match storage
        .copy(
            paths.cached_manifest(&manifest_uri.bucket, &manifest_uri.hash),
            paths.installed_manifest(&manifest_uri.namespace, &manifest_uri.hash),
        )
        .await
    {
        Ok(_) => Ok(()),
        Err(e) => {
            error!(
                "Failed to copy cached manifest to installed location for manifest_uri {}: {}",
                manifest_uri, e
            );
            Err(e)
        }
    }
}

/// Takes list of the required paths and create directories
async fn scaffold_paths(storage: &impl Storage, paths: Vec<PathBuf>) -> Res {
    for path in paths {
        storage.create_dir_all(&path).await?
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    #[test]
    fn test_required_paths() {
        let paths = DomainPaths::new(PathBuf::from("foo/bar"));
        let scaffolded_paths = paths.required();
        assert_eq!(
            scaffolded_paths,
            vec![
                PathBuf::from("foo/bar/.quilt/installed"),
                PathBuf::from("foo/bar/.quilt/objects"),
                PathBuf::from("foo/bar/.quilt/packages"),
            ]
        )
    }

    #[test]
    fn test_package_home() -> Res {
        let home = Home::from("/home/user/quilt");
        let namespace = Namespace::from(("test", "package"));

        let pkg_home = package_home(&home, &namespace);
        assert_eq!(pkg_home, PathBuf::from("/home/user/quilt/test/package"));

        Ok(())
    }
}
