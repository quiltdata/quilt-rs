//! Pure string path helpers encoding the remote's well-known directory
//! layout. No filesystem I/O lives here.

use crate::Namespace;

// S3 key prefix inside a remote bucket. Distinct from the on-disk
// cache directory of the same name in `quilt-rs::paths` — sharing the
// literal value today is incidental, the two contracts can evolve
// independently.
const MANIFEST_DIR: &str = ".quilt/packages";
const TAGS_DIR: &str = ".quilt/named_packages";

/// Where do we store tagged "packages". Files that contain packages' hashes.
#[must_use]
pub fn tag_key(namespace: &Namespace, tag: &str) -> String {
    format!("{TAGS_DIR}/{namespace}/{tag}")
}

/// What is the path to the JSONL manifest based on its `hash`
#[must_use]
pub fn get_manifest_key(hash: &str) -> String {
    format!("{MANIFEST_DIR}/{hash}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_tag_key() {
        let namespace = Namespace::from(("foo", "bar"));
        assert_eq!(
            tag_key(&namespace, "latest"),
            ".quilt/named_packages/foo/bar/latest"
        );
    }

    #[test]
    fn test_get_manifest_key() {
        assert_eq!(get_manifest_key("abc123"), ".quilt/packages/abc123");
    }
}
