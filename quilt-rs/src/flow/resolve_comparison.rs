//! What each resolve choice costs, as a pure comparison.
//!
//! A diverged copy chooses a whole revision: its own, made the shared one, or
//! the published `latest`, replacing its own. This module answers what the
//! two differ in — the files, row by row on content hash, and the pending
//! revisions nobody else holds — from values already read.
//! [`InstalledPackage::resolve_comparison`](crate::InstalledPackage::resolve_comparison)
//! is the I/O shell that reads them.

use std::collections::HashSet;
use std::hash::BuildHasher;
use std::path::PathBuf;

use crate::flow::remote_delta;
use crate::lineage::CommitState;
use crate::manifest::Manifest;

/// What each resolve choice costs: this copy's revision against the
/// published `latest`. Paths are logical keys, sorted.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveComparison {
    /// The published revision's own message, from its manifest header.
    pub published_message: Option<String>,
    /// Keys one side lists and the other does not, or both list with
    /// different content hashes, sorted by path. Rows only: a metadata-only
    /// difference is none.
    pub differing: Vec<PathBuf>,
    /// Revisions in the pending commit chain the registry does not list —
    /// the ones a reset strands and nobody else holds.
    pub unpublished: usize,
}

/// Compares `current` with `published` and counts the revisions of `chain`
/// that `listed` (the registry's revisions) does not hold.
#[must_use]
pub fn compare_for_resolve<S: BuildHasher>(
    current: &Manifest,
    published: &Manifest,
    chain: Option<&CommitState>,
    listed: &HashSet<String, S>,
) -> ResolveComparison {
    // `remote_delta`'s map is keyed by path, so the set comes out sorted.
    let differing = remote_delta(current, published).into_keys().collect();
    let unpublished = chain.map_or(0, |c| {
        std::iter::once(&c.hash)
            .chain(&c.prev_hashes)
            .filter(|h| !listed.contains(*h))
            .count()
    });
    ResolveComparison {
        published_message: published.header.message.clone(),
        differing,
        unpublished,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use multihash::Multihash;

    use crate::manifest::ManifestRow;

    fn row(key: &str, hash_seed: &[u8]) -> ManifestRow {
        ManifestRow {
            logical_key: PathBuf::from(key),
            physical_key: format!("s3://b/{key}"),
            hash: Multihash::<256>::wrap(0x12, hash_seed)
                .unwrap()
                .try_into()
                .unwrap(),
            size: hash_seed.len() as u64,
            meta: None,
        }
    }

    fn manifest_of(rows: Vec<ManifestRow>) -> Manifest {
        Manifest {
            rows,
            ..Manifest::default()
        }
    }

    fn with_message(mut manifest: Manifest, message: Option<&str>) -> Manifest {
        manifest.header.message = message.map(str::to_owned);
        manifest
    }

    fn chain(hash: &str, prev_hashes: &[&str]) -> CommitState {
        CommitState {
            hash: hash.to_owned(),
            prev_hashes: prev_hashes.iter().map(|h| (*h).to_owned()).collect(),
            ..CommitState::default()
        }
    }

    fn listed(hashes: &[&str]) -> HashSet<String> {
        hashes.iter().map(|h| (*h).to_owned()).collect()
    }

    #[test]
    fn a_key_on_either_side_or_with_other_content_differs() {
        let current = manifest_of(vec![row("a", b"1"), row("b", b"2"), row("c", b"3")]);
        let published = manifest_of(vec![row("a", b"1"), row("b", b"other"), row("d", b"4")]);

        let comparison = compare_for_resolve(&current, &published, None, &HashSet::new());

        assert_eq!(
            comparison.differing,
            vec![PathBuf::from("b"), PathBuf::from("c"), PathBuf::from("d")]
        );
    }

    #[test]
    fn a_metadata_only_difference_marks_no_file() {
        let current = with_message(manifest_of(vec![row("a", b"1")]), Some("Mine"));
        let published = with_message(manifest_of(vec![row("a", b"1")]), Some("Theirs"));

        let comparison = compare_for_resolve(&current, &published, None, &HashSet::new());

        assert!(comparison.differing.is_empty(), "{comparison:?}");
    }

    #[test]
    fn the_published_message_is_the_published_header_s() {
        let current = with_message(manifest_of(vec![]), Some("Mine"));
        let sent = with_message(manifest_of(vec![]), Some("Sent"));
        let silent = with_message(manifest_of(vec![]), None);

        assert_eq!(
            compare_for_resolve(&current, &sent, None, &HashSet::new()).published_message,
            Some("Sent".to_owned())
        );
        assert_eq!(
            compare_for_resolve(&current, &silent, None, &HashSet::new()).published_message,
            None
        );
    }

    #[test]
    fn unpublished_is_the_chain_less_what_the_registry_lists() {
        let manifest = manifest_of(vec![]);
        let pending = chain("c3", &["c2", "c1"]);

        let comparison = compare_for_resolve(
            &manifest,
            &manifest,
            Some(&pending),
            &listed(&["c1", "base"]),
        );

        assert_eq!(comparison.unpublished, 2);
    }

    #[test]
    fn no_pending_commit_strands_nothing() {
        let manifest = manifest_of(vec![]);

        let comparison = compare_for_resolve(&manifest, &manifest, None, &listed(&["base"]));

        assert_eq!(comparison.unpublished, 0);
    }
}
