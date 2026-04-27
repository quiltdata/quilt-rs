//! Pure string path helpers encoding the remote's well-known directory
//! layout. No filesystem I/O lives here.

use crate::Namespace;

// S3 key prefix inside a remote bucket. Distinct from the on-disk
// cache directory of the same name in `quilt-rs::paths` — sharing the
// literal value today is incidental, the two contracts can evolve
// independently (e.g. a future `manifests/v2/<hash>` remote layout).
const MANIFEST_DIR: &str = ".quilt/packages";
const TAGS_DIR: &str = ".quilt/named_packages";

/// Where do we store tagged "packages". Files that contain packages' hashes.
pub fn tag_key(namespace: &Namespace, tag: &str) -> String {
    format!("{TAGS_DIR}/{namespace}/{tag}")
}

/// What is the path to the JSONL manifest based on its `hash`
pub fn get_manifest_key_legacy(hash: &str) -> String {
    format!("{MANIFEST_DIR}/{hash}")
}
