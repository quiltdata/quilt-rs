//! The addresses the app links to, built in one place.
//!
//! A namespace is user data — `org/name`, where `name` is whatever the package
//! was created as — and it reaches the router through a query string. Six call
//! sites used to interpolate it raw, and a namespace holding `&`, `#` or `=`
//! made a URL that says something other than what the caller meant: `team/a&b`
//! parses as `namespace=team/a` plus a stray `b`, and a `#` truncates the query
//! at the fragment. Percent-encoding here rather than at each call site is what
//! keeps a seventh call site from being the one that forgets.
//!
//! The router decodes on the way in, as does the backend's own parse of a
//! `back` parameter (`src-tauri/src/routes.rs`), so the encoding is invisible to
//! everything downstream.

/// The installed-package screen for one package.
///
/// `namespace` goes in the query string because `installed_package` reads it
/// with `use_query_map`, and a bare path leaves it empty. `filter=unmodified`
/// is part of the address rather than a default the page applies: it is what
/// every link to this screen has always carried.
///
/// The `[Get latest]` and `[Choose S3 bucket]` queue actions land here too,
/// because neither has a page of its own — v1 puts `Pull` in this page's status
/// banner and `SetRemote` in its toolbar.
pub fn package_page_href(namespace: &str) -> String {
    let namespace = urlencoding::encode(namespace);
    format!("/installed-package?namespace={namespace}&filter=unmodified")
}

/// The commit screen for one package.
pub fn commit_href(namespace: &str) -> String {
    let namespace = urlencoding::encode(namespace);
    format!("/commit?namespace={namespace}")
}

/// The merge screen for one package.
pub fn merge_href(namespace: &str) -> String {
    let namespace = urlencoding::encode(namespace);
    format!("/merge?namespace={namespace}")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The ordinary shape, pinned whole rather than by substring: a substring
    /// match cannot tell a missing namespace from a present one.
    #[test]
    fn a_plain_namespace_keeps_its_slash_readable() {
        assert_eq!(
            package_page_href("org/pkg"),
            "/installed-package?namespace=org%2Fpkg&filter=unmodified"
        );
        assert_eq!(commit_href("org/pkg"), "/commit?namespace=org%2Fpkg");
        assert_eq!(merge_href("org/pkg"), "/merge?namespace=org%2Fpkg");
    }

    /// The defect this module exists for. `&` starts a new parameter and `#` a
    /// fragment, so either one raw ends the namespace early and the rest of the
    /// address means something the caller never wrote.
    #[test]
    fn a_namespace_cannot_end_the_parameter_early() {
        assert_eq!(
            commit_href("team/a&b"),
            "/commit?namespace=team%2Fa%26b",
            "`&` would otherwise start a second parameter"
        );
        assert_eq!(
            commit_href("team/a#b"),
            "/commit?namespace=team%2Fa%23b",
            "`#` would otherwise truncate the query at a fragment"
        );
        assert_eq!(
            commit_href("team/a=b"),
            "/commit?namespace=team%2Fa%3Db",
            "`=` would otherwise land inside the value unescaped"
        );
    }

    /// `filter` has to survive whatever the namespace contains — a raw `&` in
    /// the namespace used to be indistinguishable from the separator before it.
    #[test]
    fn the_filter_stays_its_own_parameter() {
        assert_eq!(
            package_page_href("team/a&filter=all"),
            "/installed-package?namespace=team%2Fa%26filter%3Dall&filter=unmodified"
        );
    }
}
