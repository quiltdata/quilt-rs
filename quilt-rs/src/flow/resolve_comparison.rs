//! What each resolve choice costs, as a pure comparison.
//!
//! A diverged copy chooses a whole revision: its own, made the shared one, or
//! the published `latest`, replacing its own. This module answers what the
//! two differ in — the files, row by row on content hash, what the published
//! revision loses to this one, and the pending revisions nobody else holds —
//! from values already read.
//! [`InstalledPackage::resolve_comparison`](crate::InstalledPackage::resolve_comparison)
//! is the I/O shell that reads them.

use std::collections::HashMap;
use std::collections::HashSet;
use std::hash::BuildHasher;
use std::path::PathBuf;

use crate::flow::remote_delta;
use crate::lineage::CommitState;
use crate::manifest::Manifest;
use crate::manifest::normalized_entry_meta;

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
    /// Published rows that making this revision `latest` drops: keys it lacks,
    /// or lists with another content hash or entry `meta`. A key only this
    /// revision lists drops nothing.
    pub lost: usize,
    /// Whether making this revision `latest` replaces the published `user_meta`.
    pub user_meta_differs: bool,
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
    let mine: HashMap<_, _> = current
        .rows
        .iter()
        .map(|row| (&row.logical_key, row))
        .collect();
    let lost = published
        .rows
        .iter()
        .filter(|theirs| {
            mine.get(&theirs.logical_key).is_none_or(|row| {
                row.hash != theirs.hash
                    || (row.meta != theirs.meta
                        && normalized_entry_meta(row.meta.as_ref())
                            != normalized_entry_meta(theirs.meta.as_ref()))
            })
        })
        .count();
    // Normalized as entry `meta` is, so a header with none and one with `{}` agree.
    let user_meta_differs = normalized_entry_meta(current.header.user_meta.as_ref())
        != normalized_entry_meta(published.header.user_meta.as_ref());
    ResolveComparison {
        published_message: published.header.message.clone(),
        differing,
        lost,
        user_meta_differs,
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

    fn with_entry_meta(mut row: ManifestRow, meta: Option<serde_json::Value>) -> ManifestRow {
        row.meta = meta;
        row
    }

    fn with_user_meta(mut manifest: Manifest, meta: Option<serde_json::Value>) -> Manifest {
        manifest.header.user_meta = meta;
        manifest
    }

    #[test]
    fn keys_only_this_revision_lists_lose_nothing() {
        let current = manifest_of(vec![row("a", b"1"), row("new", b"2")]);
        let published = manifest_of(vec![row("a", b"1")]);

        let comparison = compare_for_resolve(&current, &published, None, &HashSet::new());

        assert_eq!(comparison.lost, 0);
        assert!(!comparison.user_meta_differs);
    }

    #[test]
    fn a_published_key_lacked_or_held_otherwise_is_lost() {
        let current = manifest_of(vec![
            row("same", b"1"),
            row("content", b"mine"),
            with_entry_meta(row("meta", b"3"), Some(serde_json::json!({"k": "mine"}))),
        ]);
        let published = manifest_of(vec![
            row("same", b"1"),
            row("content", b"theirs"),
            with_entry_meta(row("meta", b"3"), Some(serde_json::json!({"k": "theirs"}))),
            row("gone", b"4"),
        ]);

        let comparison = compare_for_resolve(&current, &published, None, &HashSet::new());

        assert_eq!(comparison.lost, 3, "content, meta and gone");
    }

    #[test]
    fn null_absent_and_empty_entry_meta_are_one() {
        let current = manifest_of(vec![
            with_entry_meta(row("a", b"1"), None),
            with_entry_meta(row("b", b"2"), Some(serde_json::Value::Null)),
        ]);
        let published = manifest_of(vec![
            with_entry_meta(row("a", b"1"), Some(serde_json::json!({}))),
            with_entry_meta(row("b", b"2"), None),
        ]);

        let comparison = compare_for_resolve(&current, &published, None, &HashSet::new());

        assert_eq!(comparison.lost, 0);
    }

    #[test]
    fn other_published_user_meta_is_replaced() {
        let current = with_user_meta(manifest_of(vec![]), Some(serde_json::json!({"a": 1})));
        let published = with_user_meta(
            manifest_of(vec![]),
            Some(serde_json::json!({"a": 1, "task": "registered"})),
        );

        let comparison = compare_for_resolve(&current, &published, None, &HashSet::new());

        assert!(comparison.user_meta_differs);
        assert_eq!(comparison.lost, 0);
    }

    #[test]
    fn no_user_meta_and_empty_user_meta_agree() {
        let current = with_user_meta(manifest_of(vec![]), None);
        let published = with_user_meta(manifest_of(vec![]), Some(serde_json::json!({})));

        let comparison = compare_for_resolve(&current, &published, None, &HashSet::new());

        assert!(!comparison.user_meta_differs);
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
