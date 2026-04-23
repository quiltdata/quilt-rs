//! Pure string path helpers encoding the remote's well-known directory
//! layout. No filesystem I/O lives here.

use crate::Namespace;

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
