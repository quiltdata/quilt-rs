//! The list's own re-arrangements: filter, group, sort.
//!
//! All three are the UI's by `2026-08-06-data-requirements.md` §3 — "a
//! re-arrangement of data it already holds" — so nothing here fetches, and
//! nothing here is a second opinion about data the backend already settled.
//!
//! Plain functions over plain data, deliberately: they carry the ordering rules
//! §3.1 fixes, and those are worth testing without a DOM.

use std::collections::BTreeMap;

use quilt_uri::Namespace;

use super::GROUP_BUCKET;
use super::GROUP_CATALOG;
#[cfg(test)]
use super::GROUP_NONE;
use super::GROUP_PREFIX;
#[cfg(test)]
use super::SORT_CHANGED;
use super::SORT_NAME;

/// One list row's own data, as the list arranges it.
///
/// A struct rather than the tuple this replaced: two of its fields are sort
/// and group keys, and `row.1` says nothing at a call site.
///
/// Everything that settles — the state, whether it is still the light phase's
/// guess — lives in `PackageStore` and is read from there. This carries only
/// what the payload fixes for the life of the read.
#[derive(Clone, Debug, PartialEq)]
pub struct ListRowData {
    pub namespace: Namespace,
    pub changed_at: Option<f64>,
    /// `None` is a local-only package: its own group, and first on the bucket
    /// axis (§3.1), because local packages are the ones missing a bucket and
    /// burying them hides what most needs setup.
    pub bucket: Option<String>,
    /// The catalog the package points at, for the row's tooltip and the
    /// catalog axis.
    pub host: Option<String>,
}

/// Case-insensitive substring over the namespace, and nothing else (R5).
///
/// The empty state tells the user search covers package names; matching a
/// bucket or a state as well would be a contract nothing on screen states.
pub fn filter_packages(rows: Vec<ListRowData>, query: &str) -> Vec<ListRowData> {
    let needle = query.trim().to_lowercase();
    if needle.is_empty() {
        return rows;
    }
    rows.into_iter()
        .filter(|row| row.namespace.to_string().to_lowercase().contains(&needle))
        .collect()
}

/// §3.1's two axes. Stable in both, so rows that tie keep the order the
/// backend sent — which for `Changed` is already newest-first, and for a tie
/// on `Name` cannot happen, namespaces being unique.
pub fn sort_within(rows: &mut [ListRowData], sort_by: &str) {
    match sort_by {
        SORT_NAME => rows.sort_by_key(|a| a.namespace.to_string().to_lowercase()),
        // Newest first. `None` is "nothing has ever been written here", which
        // sorts last rather than as epoch zero — the difference is invisible in
        // the ordering but not in what the code says it means.
        _ => rows.sort_by(|a, b| match (b.changed_at, a.changed_at) {
            (Some(b), Some(a)) => b.total_cmp(&a),
            (Some(_), None) => std::cmp::Ordering::Greater,
            (None, Some(_)) => std::cmp::Ordering::Less,
            (None, None) => std::cmp::Ordering::Equal,
        }),
    }
}

/// A run of rows drawn under one heading.
///
/// `PartialEq` because the list holds the arrangement in a `Memo`: an edit that
/// arranges to the same rows never reaches the keyed lists below.
#[derive(Clone, Debug, PartialEq)]
pub struct PackageGroup {
    /// `None` only for the single group the `None` axis produces — the one
    /// case where the list draws no heading at all.
    pub title: Option<String>,
    pub rows: Vec<ListRowData>,
}

/// The heading for the packages with no remote, on the bucket and catalog axes.
const LOCAL_ONLY: &str = "Local only";

/// The heading for the packages with a bucket and no catalog host, on the
/// catalog axis. Neutral words: such a package is supported, not misconfigured.
/// Before every host on that axis, since adding a catalog is what a reader most
/// likely came to do there.
const NO_CATALOG: &str = "No catalog";

/// A row's group on the axis: its rank, then its heading. Groups are ordered
/// by rank first, so the fixed headings land where §3.1 puts them whatever
/// their letters, and by heading within a rank.
///
/// Bucket: `Local only` first, because local packages are the ones missing a
/// bucket and burying them hides what most needs setup; then `s3://` buckets.
/// The payload's `bucket` field is bare, so the scheme is added only here.
///
/// Catalog: `Local only`, then `No catalog`, then each catalog host as the
/// row's tooltip names it. `Local only` leads as it does on the bucket axis.
///
/// Prefix: the namespace up to the first `/`; a namespace with no `/` is its
/// own prefix.
fn group_key(row: &ListRowData, group_by: &str) -> (u8, String) {
    match (group_by, &row.bucket, &row.host) {
        (GROUP_BUCKET | GROUP_CATALOG, None, _) => (0, LOCAL_ONLY.to_string()),
        (GROUP_BUCKET, Some(bucket), _) => (1, format!("s3://{bucket}")),
        (GROUP_CATALOG, Some(_), None) => (1, NO_CATALOG.to_string()),
        (GROUP_CATALOG, Some(_), Some(host)) => (2, host.clone()),
        _ => (0, row.namespace.prefix().to_string()),
    }
}

/// §3.1's axes, each already in the order the list draws them (see
/// `group_key`). No axis re-orders rows WITHIN a group — that is
/// `sort_within`'s job, and the caller applies it per group.
pub fn group_packages(rows: Vec<ListRowData>, group_by: &str) -> Vec<PackageGroup> {
    if ![GROUP_BUCKET, GROUP_CATALOG, GROUP_PREFIX].contains(&group_by) {
        return vec![PackageGroup { title: None, rows }];
    }
    let mut by_key: BTreeMap<(u8, String), Vec<ListRowData>> = BTreeMap::new();
    for row in rows {
        by_key
            .entry(group_key(&row, group_by))
            .or_default()
            .push(row);
    }
    by_key
        .into_iter()
        .map(|((_, title), rows)| PackageGroup {
            title: Some(title),
            rows,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(namespace: &str, changed_at: Option<f64>, bucket: Option<&str>) -> ListRowData {
        ListRowData {
            namespace: Namespace::try_from(namespace).expect("a namespace"),
            changed_at,
            bucket: bucket.map(str::to_string),
            host: None,
        }
    }

    fn remote(namespace: &str, changed_at: Option<f64>, host: Option<&str>) -> ListRowData {
        ListRowData {
            host: host.map(str::to_string),
            ..row(namespace, changed_at, Some("bucket"))
        }
    }

    fn titles(groups: &[PackageGroup]) -> Vec<Option<&str>> {
        groups.iter().map(|g| g.title.as_deref()).collect()
    }

    #[test]
    fn search_matches_a_namespace_case_insensitively_anywhere_in_the_string() {
        // R5: case-insensitive substring. "Search covers the names of packages
        // installed on this machine" is the promise the empty state makes, so the
        // match is on the namespace and on nothing else.
        let rows = vec![
            row("user/Plate-07", None, Some("s3://a")),
            row("team/other", None, Some("s3://b")),
        ];

        let hit = filter_packages(rows, "plate");

        assert_eq!(hit.len(), 1);
        assert_eq!(hit[0].namespace.to_string(), "user/Plate-07");
    }

    #[test]
    fn search_does_not_match_a_bucket_the_description_never_promised() {
        // The empty state tells the user search covers package names. Matching a
        // bucket too would be a second contract nothing on screen states.
        let rows = vec![row("user/alpha", None, Some("s3://plate-bucket"))];

        assert_eq!(filter_packages(rows, "plate"), [] as [ListRowData; 0]);
    }

    #[test]
    fn a_blank_or_whitespace_query_filters_nothing() {
        let rows = vec![row("user/alpha", None, None), row("team/beta", None, None)];

        assert_eq!(filter_packages(rows.clone(), "").len(), 2);
        assert_eq!(filter_packages(rows, "   ").len(), 2);
    }

    #[test]
    fn sort_changed_is_newest_first() {
        // §3.1's default. The fixture's given order is NON-MONOTONIC, so any
        // re-ordering in either direction fails this — a fixture already in the
        // asserted order would pass against an implementation that does nothing.
        let mut rows = vec![
            row("user/middle", Some(5_000.0), None),
            row("user/newest", Some(9_000.0), None),
            row("user/oldest", Some(1_000.0), None),
        ];

        sort_within(&mut rows, SORT_CHANGED);

        let names: Vec<String> = rows.iter().map(|r| r.namespace.to_string()).collect();
        assert_eq!(names, vec!["user/newest", "user/middle", "user/oldest"]);
    }

    #[test]
    fn a_package_that_has_never_changed_sorts_last_and_not_first() {
        // `changed_at: None` means nothing has ever been written to the package.
        // Treating it as 0 would work by accident; treating it as newest would put
        // the least interesting rows at the top, so it is worth an assertion.
        //
        // A two-element slice makes exactly one `sort_by` comparator call,
        // `cmp(v[1], v[0])`, so which of the two `None`-vs-`Some` match arms that
        // call reaches depends on the input order: `[old, never]` reaches
        // `(Some(_), None)`, and `[never, old]` reaches `(None, Some(_))`. Sorting
        // both orders here — rather than only one — is what makes both arms load-
        // bearing on this test instead of leaving one of them unexercised by an
        // accident of which element started first.
        let mut old_first = vec![
            row("user/old", Some(1_000.0), None),
            row("user/never", None, None),
        ];
        sort_within(&mut old_first, SORT_CHANGED);
        assert_eq!(old_first[0].namespace.to_string(), "user/old");
        assert_eq!(old_first[1].namespace.to_string(), "user/never");

        let mut never_first = vec![
            row("user/never", None, None),
            row("user/old", Some(1_000.0), None),
        ];
        sort_within(&mut never_first, SORT_CHANGED);
        assert_eq!(never_first[0].namespace.to_string(), "user/old");
        assert_eq!(never_first[1].namespace.to_string(), "user/never");
    }

    #[test]
    fn sort_name_is_case_insensitive_and_ascending() {
        // Case-sensitive ordering would put every capitalised namespace above
        // every lower-case one, which reads as a random shuffle to anyone who did
        // not know the rule.
        let mut rows = vec![
            row("user/beta", None, None),
            row("user/Alpha", None, None),
            row("user/gamma", None, None),
        ];

        sort_within(&mut rows, SORT_NAME);

        let names: Vec<String> = rows.iter().map(|r| r.namespace.to_string()).collect();
        assert_eq!(names, vec!["user/Alpha", "user/beta", "user/gamma"]);
    }

    #[test]
    fn the_bucket_axis_puts_local_only_first_then_buckets_alphabetically() {
        // §3.1: "`Local only` sorts first — local packages are the ones missing a
        // bucket, so burying them hides what most needs setup — then `s3://`
        // buckets alphabetically."
        //
        // Bare bucket names, exactly as the payload carries them
        // (`src-tauri/src/engine/commands/main_page.rs:541`) — never `s3://`-prefixed;
        // the scheme is the heading's, not the field's. The fixture's given order
        // is neither alphabetical nor local-first, so an implementation that
        // preserved input order, or sorted without the local-first rule, fails.
        let rows = vec![
            row("user/z", None, Some("zebra")),
            row("user/l", None, None),
            row("user/a", None, Some("apple")),
        ];

        let groups = group_packages(rows, GROUP_BUCKET);

        let titles: Vec<Option<&str>> = groups.iter().map(|g| g.title.as_deref()).collect();
        assert_eq!(
            titles,
            vec![Some("Local only"), Some("s3://apple"), Some("s3://zebra")]
        );
    }

    #[test]
    fn the_prefix_axis_groups_on_the_first_namespace_segment() {
        // §4.4: "The prefix axis needs no field: it is `namespace` up to the first
        // `/`." Two buckets under one prefix must land in ONE group — that is the
        // whole point of the axis, and it is what a bucket-keyed implementation
        // would get wrong while passing every other assertion here.
        let rows = vec![
            row("team/one", None, Some("a")),
            row("user/x", None, Some("b")),
            row("team/two", None, Some("c")),
        ];

        let groups = group_packages(rows, GROUP_PREFIX);

        let titles: Vec<Option<&str>> = groups.iter().map(|g| g.title.as_deref()).collect();
        assert_eq!(titles, vec![Some("team"), Some("user")]);
        assert_eq!(groups[0].rows.len(), 2, "both team packages in one group");
    }

    #[test]
    fn the_none_axis_is_one_unnamed_group_holding_everything() {
        // Not "no groups": the renderer walks groups either way, and the single
        // untitled group is what tells it to draw no heading.
        let rows = vec![row("user/a", None, None), row("user/b", None, Some("x"))];

        let groups = group_packages(rows, GROUP_NONE);

        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].title, None, "no heading for the ungrouped list");
        assert_eq!(groups[0].rows.len(), 2);
    }

    #[test]
    fn the_catalog_axis_lists_local_only_then_no_catalog_then_hosts() {
        // The packages with no bucket at all, then the ones with a bucket and
        // no catalog, then hosts alphabetically. The given order is none of
        // these.
        let rows = vec![
            remote("user/z", None, Some("zebra.quilt.test")),
            row("user/l", None, None),
            remote("user/n", None, None),
            remote("user/a", None, Some("apple.quilt.test")),
            remote("user/a2", None, Some("apple.quilt.test")),
        ];

        let groups = group_packages(rows, GROUP_CATALOG);

        assert_eq!(
            titles(&groups),
            vec![
                Some("Local only"),
                Some("No catalog"),
                Some("apple.quilt.test"),
                Some("zebra.quilt.test"),
            ]
        );
        assert_eq!(groups[2].rows.len(), 2, "both apple packages in one group");
    }

    #[test]
    fn a_host_that_sorts_before_the_fixed_titles_still_comes_after_them() {
        // "Local only" and "No catalog" lead whatever a host is called, not
        // where their own letters would put them. A digit sorts before a
        // capital, so this host would come first on letters alone.
        let rows = vec![
            remote("user/a", None, Some("0.quilt.test")),
            row("user/l", None, None),
            remote("user/n", None, None),
        ];

        let groups = group_packages(rows, GROUP_CATALOG);

        assert_eq!(
            titles(&groups),
            vec![Some("Local only"), Some("No catalog"), Some("0.quilt.test")]
        );
    }

    #[test]
    fn the_catalog_axis_leaves_sorting_to_sort_within() {
        let rows = vec![
            remote("user/old", Some(1_000.0), Some("quilt.test")),
            remote("user/new", Some(9_000.0), Some("quilt.test")),
            remote("user/Beta", Some(5_000.0), None),
            remote("user/alpha", Some(2_000.0), None),
        ];

        let mut groups = group_packages(rows, GROUP_CATALOG);
        for group in &mut groups {
            sort_within(&mut group.rows, SORT_CHANGED);
        }

        let names: Vec<Vec<String>> = groups
            .iter()
            .map(|g| g.rows.iter().map(|r| r.namespace.to_string()).collect())
            .collect();
        assert_eq!(
            names,
            vec![
                vec!["user/Beta", "user/alpha"],
                vec!["user/new", "user/old"],
            ]
        );
    }
}
