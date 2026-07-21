//!
//! Incapsulated knowlegde about directory structure of the files in `.quilt`, packages and working directories.

use std::path::Path;
use std::path::PathBuf;

#[cfg(test)]
use tempfile::TempDir;
use tracing::error;

use crate::Res;
use crate::io::storage::Storage;
use crate::lineage::Home;
use quilt_uri::Host;
use quilt_uri::ManifestUri;
use quilt_uri::Namespace;

pub const AUTH_CLIENT: &str = "client.json";
pub const AUTH_CREDENTIALS: &str = "credentials.json";
pub const AUTH_DIR: &str = ".auth";
pub const AUTH_TOKENS: &str = "tokens.json";

/// Name of the `.quilt` directory that holds Quilt's bookkeeping inside a
/// data dir (lineage, installed manifests, cached manifests, objects).
pub const DOT_QUILT_DIR: &str = ".quilt";

/// List authenticated host directories under the given data directory.
///
/// Returns a sorted list of directory names found in `<data_dir>/.auth/`.
// TODO: Also include registries from data.json/Lineage file.
pub fn list_auth_hosts(data_dir: &Path) -> Vec<String> {
    let auth_dir = data_dir.join(AUTH_DIR);
    let mut hosts: Vec<String> = Vec::new();
    if auth_dir.exists()
        && let Ok(entries) = std::fs::read_dir(&auth_dir)
    {
        for entry in entries.flatten() {
            if entry.file_type().is_ok_and(|t| t.is_dir())
                && let Some(name) = entry.file_name().to_str()
            {
                hosts.push(name.to_string());
            }
        }
    }
    hosts.sort();
    hosts
}

// Paths below are relative to `DOT_QUILT_DIR`; every accessor joins them onto
// `dot_quilt_dir()`, so `DOT_QUILT_DIR` stays the single source of the prefix
// and a rename propagates without touching these.
const LINEAGE_FILE: &str = "data.json";

const INSTALLED_DIR: &str = "installed";
// Local cache directory under `<data_dir>/.quilt`. Distinct from the S3 key
// prefix of the same name in `quilt-uri::paths` — sharing the literal
// value today is incidental, the two contracts can evolve independently.
const MANIFEST_DIR: &str = "packages";
const OBJECTS_DIR: &str = "objects";

pub use quilt_uri::paths::get_manifest_key;
pub use quilt_uri::paths::tag_key;

/// Path to the package home directory within the home directory
pub fn package_home(home: &Home, namespace: &Namespace) -> PathBuf {
    home.join(namespace.to_string())
}

/// Helper for getting paths.
/// We heavily rely on where we put files,
/// and this struct contains info of the directory structure .
///
/// TODO: Avoid `DomainPaths::default()` — the empty root produces relative
/// `.quilt/...` paths that read like magic and only work by coincidence with
/// in-memory storage. Prefer an explicit root via `DomainPaths::new(...)`.
/// Once the remaining test call sites are converted, drop the `Default` derive.
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

    /// Path to the `.quilt` bookkeeping directory under the root.
    pub fn dot_quilt_dir(&self) -> PathBuf {
        self.root_dir.join(DOT_QUILT_DIR)
    }

    /// Path to the installed manifest.
    ///
    /// Takes `(namespace, hash)` rather than `&ManifestUri` because the
    /// installed manifest may belong to either a remote-backed package
    /// (where a `ManifestUri` is available) or a local-only package
    /// (created via `flow::create`, where there is no bucket or origin).
    /// A local commit also produces a hash that has no remote
    /// counterpart yet.
    pub fn installed_manifest(&self, namespace: &Namespace, hash: &str) -> PathBuf {
        self.installed_manifests_dir(namespace).join(hash)
    }

    /// Directory for storing installed manifests
    pub fn installed_manifests_dir(&self, namespace: &Namespace) -> PathBuf {
        self.dot_quilt_dir()
            .join(INSTALLED_DIR)
            .join(namespace.to_string())
    }

    /// Path to the lineage file
    pub fn lineage(&self) -> PathBuf {
        self.dot_quilt_dir().join(LINEAGE_FILE)
    }

    /// Path to the manifest cached in semi-temporary directory
    pub fn cached_manifest(&self, uri: &ManifestUri) -> PathBuf {
        self.dot_quilt_dir()
            .join(MANIFEST_DIR)
            .join(&uri.bucket)
            .join(&uri.hash)
    }

    /// Directory for storing cached manifests for a bucket
    pub fn cached_manifests_dir(&self, bucket: &str) -> PathBuf {
        self.dot_quilt_dir().join(MANIFEST_DIR).join(bucket)
    }

    /// Directory for storing pristine hashed files
    pub fn objects_dir(&self) -> PathBuf {
        self.dot_quilt_dir().join(OBJECTS_DIR)
    }

    /// Path to the pristine hashed file
    pub fn object(&self, hash: &[u8]) -> PathBuf {
        self.objects_dir().join(hex::encode(hash))
    }

    /// What directories are essential when we initiate `LocalDomain`
    fn required(&self) -> Vec<PathBuf> {
        vec![
            self.dot_quilt_dir().join(INSTALLED_DIR),
            self.objects_dir(),
            self.dot_quilt_dir().join(MANIFEST_DIR),
        ]
    }

    /// What directories are essential when we initiate `InstalledPackage`
    fn required_for_installing(&self, home: &Home, namespace: &Namespace) -> Vec<PathBuf> {
        let mut paths = vec![];
        paths.extend(self.required());
        paths.extend(vec![
            package_home(home, namespace),
            self.installed_manifests_dir(namespace),
        ]);
        paths
    }

    pub async fn scaffold_for_installing(
        &self,
        storage: &impl Storage,
        home: &Home,
        namespace: &Namespace,
    ) -> Res {
        scaffold_paths(storage, self.required_for_installing(home, namespace)).await
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
            paths.cached_manifest(manifest_uri),
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
        storage.create_dir_all(&path).await?;
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
        );
    }

    /// Pin the on-disk layout each `DomainPaths` accessor produces, so a
    /// drift in `DOT_QUILT_DIR` / `LINEAGE_FILE` / `INSTALLED_DIR` /
    /// `MANIFEST_DIR` / `OBJECTS_DIR` / `AUTH_DIR` fails here rather than
    /// silently changing behavior in every downstream test that builds
    /// paths through these helpers.
    #[test]
    fn test_domain_paths_layout() {
        let paths = DomainPaths::new(PathBuf::from("foo/bar"));
        let namespace = Namespace::from(("test", "package"));
        let host: Host = "example.com".parse().unwrap();
        let manifest_uri = ManifestUri {
            bucket: "my-bucket".to_string(),
            namespace: namespace.clone(),
            hash: "deadbeef".to_string(),
            origin: None,
        };

        assert_eq!(paths.dot_quilt_dir(), PathBuf::from("foo/bar/.quilt"));
        assert_eq!(paths.lineage(), PathBuf::from("foo/bar/.quilt/data.json"));
        assert_eq!(
            paths.installed_manifests_dir(&namespace),
            PathBuf::from("foo/bar/.quilt/installed/test/package"),
        );
        assert_eq!(
            paths.installed_manifest(&namespace, "deadbeef"),
            PathBuf::from("foo/bar/.quilt/installed/test/package/deadbeef"),
        );
        assert_eq!(
            paths.cached_manifests_dir("my-bucket"),
            PathBuf::from("foo/bar/.quilt/packages/my-bucket"),
        );
        assert_eq!(
            paths.cached_manifest(&manifest_uri),
            PathBuf::from("foo/bar/.quilt/packages/my-bucket/deadbeef"),
        );
        assert_eq!(paths.objects_dir(), PathBuf::from("foo/bar/.quilt/objects"));
        assert_eq!(
            paths.object(&[0xde, 0xad, 0xbe, 0xef]),
            PathBuf::from("foo/bar/.quilt/objects/deadbeef"),
        );
        assert_eq!(
            paths.auth_host(&host),
            PathBuf::from("foo/bar/.auth/example.com"),
        );
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
