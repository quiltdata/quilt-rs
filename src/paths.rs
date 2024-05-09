use std::path::PathBuf;

use crate::io::storage::Storage;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
use crate::Error;

const MANIFEST_DIR: &str = ".quilt/packages";
const TAGS_DIR: &str = ".quilt/named_packages";

const OBJECTS_DIR: &str = ".quilt/objects";
const LINEAGE_FILE: &str = ".quilt/data.json";
const INSTALLED_DIR: &str = ".quilt/installed";

/// Where do we store tagged "packages". Files that contain packages' hashes.
pub fn tag_key(namespace: &Namespace, tag: &str) -> String {
    format!("{}/{}/{}", TAGS_DIR, namespace, tag)
}

fn parquet_manifest_filename(top_hash: &str) -> String {
    format!("1220{}.parquet", top_hash)
}

/// What is the path to the PARQUET manifest based on its `hash`
pub fn get_manifest_key(hash: &str) -> String {
    format!("{}/{}", MANIFEST_DIR, parquet_manifest_filename(hash))
}

/// What is the path to the JSONL manifest based on its `hash`
pub fn get_manifest_key_legacy(hash: &str) -> String {
    format!("{}/{}", MANIFEST_DIR, hash)
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

    /// Path to the installed manifest
    // TODO: pass `ManifestUri`
    pub fn installed_manifest(&self, namespace: &Namespace, hash: &str) -> PathBuf {
        self.installed_manifests(namespace).join(hash)
    }

    /// Directory for storing installed manifests
    pub fn installed_manifests(&self, namespace: &Namespace) -> PathBuf {
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
    pub fn manifest_cache(&self, bucket: &str, hash: &str) -> PathBuf {
        self.root_dir.join(MANIFEST_DIR).join(bucket).join(hash)
    }

    /// Directory for storing pristine hashed files
    pub fn objects_dir(&self) -> PathBuf {
        self.root_dir.join(OBJECTS_DIR)
    }

    /// Path to the pristine hashed file
    pub fn object(&self, hash: &[u8]) -> PathBuf {
        self.objects_dir().join(hex::encode(hash))
    }

    /// Directory for storing installed files that can be modified
    pub fn working_dir(&self, namespace: &Namespace) -> PathBuf {
        self.root_dir.join(namespace.to_string())
    }

    /// What directories are essential when we initiate `LocalDomain`
    pub fn required_local_domain_paths(&self) -> Vec<PathBuf> {
        vec![
            self.root_dir.join(INSTALLED_DIR),
            self.objects_dir(),
            self.root_dir.join(MANIFEST_DIR),
        ]
    }

    /// What directories are essential when we initiate `InstalledPackage`
    pub fn required_installed_package_paths(&self, namespace: &Namespace) -> Vec<PathBuf> {
        let mut paths = vec![];
        paths.extend(self.required_local_domain_paths());
        paths.extend(vec![
            self.working_dir(namespace),
            self.installed_manifests(namespace),
        ]);
        paths
    }
}

pub async fn copy_cached_to_installed(
    paths: &DomainPaths,
    storage: &impl Storage,
    manifest_uri: &ManifestUri,
) -> Result<(), Error> {
    storage
        .copy(
            paths.manifest_cache(&manifest_uri.bucket, &manifest_uri.hash),
            paths.installed_manifest(&manifest_uri.namespace, &manifest_uri.hash),
        )
        .await?;
    Ok(())
}

/// Takes list of the required paths and create directories
pub async fn scaffold_paths(storage: &impl Storage, paths: Vec<PathBuf>) -> Result<(), Error> {
    for path in paths {
        storage.create_dir_all(&path).await?
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_required_paths() {
        let paths = DomainPaths::new(PathBuf::from("foo/bar"));
        let scaffolded_paths = paths.required_local_domain_paths();
        assert_eq!(
            scaffolded_paths,
            vec![
                PathBuf::from("foo/bar/.quilt/installed"),
                PathBuf::from("foo/bar/.quilt/objects"),
                PathBuf::from("foo/bar/.quilt/packages"),
            ]
        )
    }
}
