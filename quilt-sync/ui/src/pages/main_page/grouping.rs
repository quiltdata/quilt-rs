//! The list's own re-arrangements: filter, group, sort.
//!
//! All three are the UI's by `2026-08-06-data-requirements.md` §3 — "a
//! re-arrangement of data it already holds" — so nothing here fetches, and
//! nothing here is a second opinion about data the backend already settled.
//!
//! Plain functions over plain data, deliberately: they carry the ordering rules
//! §3.1 fixes, and those are worth testing without a DOM.

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
}
