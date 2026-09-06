//! The list's own re-arrangements: filter, group, sort.
//!
//! All three are the UI's by `2026-08-06-data-requirements.md` §3 — "a
//! re-arrangement of data it already holds" — so nothing here fetches, and
//! nothing here is a second opinion about data the backend already settled.
//!
//! Plain functions over plain data, deliberately: they carry the ordering rules
//! §3.1 fixes, and those are worth testing without a DOM.

#[cfg(test)]
use super::SORT_CHANGED;
use super::SORT_NAME;

/// One list row's own data, as the list arranges it.
///
/// A struct rather than the tuple this replaced: three fields, two of them
/// sort and group keys, and `row.1` says nothing at a call site.
///
/// Everything that settles — the state, whether it is still the light phase's
/// guess — lives in `PackageStore` and is read from there. This carries only
/// what the payload fixes for the life of the read.
#[derive(Clone, Debug, PartialEq)]
pub struct ListRowData {
    pub namespace: String,
    pub changed_at: Option<f64>,
    /// `None` is a local-only package: its own group, and first on the bucket
    /// axis (§3.1), because local packages are the ones missing a bucket and
    /// burying them hides what most needs setup.
    pub bucket: Option<String>,
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
        .filter(|row| row.namespace.to_lowercase().contains(&needle))
        .collect()
}

/// §3.1's two axes. Stable in both, so rows that tie keep the order the
/// backend sent — which for `Changed` is already newest-first, and for a tie
/// on `Name` cannot happen, namespaces being unique.
pub fn sort_within(rows: &mut [ListRowData], sort_by: &str) {
    match sort_by {
        SORT_NAME => rows.sort_by_key(|a| a.namespace.to_lowercase()),
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

#[cfg(test)]
mod tests {
    use super::*;

    fn row(namespace: &str, changed_at: Option<f64>, bucket: Option<&str>) -> ListRowData {
        ListRowData {
            namespace: namespace.to_string(),
            changed_at,
            bucket: bucket.map(str::to_string),
        }
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
        assert_eq!(hit[0].namespace, "user/Plate-07");
    }

    #[test]
    fn search_does_not_match_a_bucket_the_description_never_promised() {
        // The empty state tells the user search covers package names. Matching a
        // bucket too would be a second contract nothing on screen states.
        let rows = vec![row("user/alpha", None, Some("s3://plate-bucket"))];

        assert!(filter_packages(rows, "plate").is_empty());
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

        let names: Vec<&str> = rows.iter().map(|r| r.namespace.as_str()).collect();
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
        assert_eq!(old_first[0].namespace, "user/old");
        assert_eq!(old_first[1].namespace, "user/never");

        let mut never_first = vec![
            row("user/never", None, None),
            row("user/old", Some(1_000.0), None),
        ];
        sort_within(&mut never_first, SORT_CHANGED);
        assert_eq!(never_first[0].namespace, "user/old");
        assert_eq!(never_first[1].namespace, "user/never");
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

        let names: Vec<&str> = rows.iter().map(|r| r.namespace.as_str()).collect();
        assert_eq!(names, vec!["user/Alpha", "user/beta", "user/gamma"]);
    }
}
