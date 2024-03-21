use std::path::PathBuf;

const MANIFEST_DIR: &str = ".quilt/packages";
const TAGS_DIR: &str = ".quilt/named_packages";
const OBJECTS_DIR: &str = ".quilt/objects";
const LINEAGE_FILE: &str = ".quilt/data.json";
const INSTALLED_DIR: &str = ".quilt/installed";

pub fn tag_key(namespace: &str, tag: &str) -> String {
    format!("{}/{}/{}", TAGS_DIR, namespace, tag)
}

fn parquet_manifest_filename(top_hash: &str) -> String {
    format!("1220{}.parquet", top_hash)
}

pub fn get_manifest_key(hash: &str) -> String {
    format!("{}/{}", MANIFEST_DIR, parquet_manifest_filename(hash))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DomainPaths {
    root_dir: PathBuf,
}

impl DomainPaths {
    pub fn new(root_dir: PathBuf) -> Self {
        DomainPaths { root_dir }
    }

    /// Path to the installed manifest
    pub fn installed_manifest(&self, namespace: &str, hash: &str) -> PathBuf {
        self.installed_manifests(namespace).join(hash)
    }

    /// Directory for storing installed manifests
    pub fn installed_manifests(&self, namespace: &str) -> PathBuf {
        self.root_dir.join(INSTALLED_DIR).join(namespace)
    }

    /// Path to the lineage file
    pub fn lineage(&self) -> PathBuf {
        self.root_dir.join(LINEAGE_FILE)
    }

    /// Path to the manifest cached in semi-temporary directory
    pub fn manifest_cache(&self, bucket: &str, hash: &str) -> PathBuf {
        self.root_dir.join(MANIFEST_DIR).join(bucket).join(hash)
    }

    /// Directory for storing pristine hashed files
    pub fn objects_dir(&self) -> PathBuf {
        self.root_dir.join(OBJECTS_DIR)
    }

    /// Directory for storing installed files that can be modified
    pub fn working_dir(&self, namespace: &str) -> PathBuf {
        self.root_dir.join(namespace)
    }
}
