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

use quilt_uri::Namespace;

/// Flip locally to work on the rebuilt package screen. Never commit it true.
///
/// An in-code flag rather than a setting, because the screen is not ready to be
/// offered: a row in Settings tells a reader the unfinished page exists, and a
/// stored value outlives the build that wrote it. The one cost is that a flag in
/// the source can be committed on by accident, which `main.rs`'s
/// `the_unfinished_package_page_is_off` is the guard against.
///
/// Here in the library rather than in the binary that routes on it, so the main
/// page's queue can send a Resolve to the page `/installed-package` renders.
pub const UNFINISHED_PACKAGE_PAGE: bool = false;

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
pub fn package_page_href(namespace: &Namespace) -> String {
    let namespace = namespace.to_string();
    let namespace = urlencoding::encode(&namespace);
    format!("/installed-package?namespace={namespace}&filter=unmodified")
}

/// The package screen with the resolve mode asked for. The plain address
/// plus `resolve=1`, so leaving is exactly dropping the parameter.
pub fn resolve_href(namespace: &Namespace) -> String {
    format!("{}&resolve=1", package_page_href(namespace))
}

/// A deep link that asked for a revision other than the installed one: the
/// requested top hash and the remote it lives on. `pages/remote_package.rs`
/// writes it into the package address and the package page reads it back.
///
/// It lives in the address rather than in page state because it is the
/// address's news: it stands while the address carries it, so every address the
/// page builds for the same package must carry it on — see [`keeping_mismatch`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RevisionMismatch {
    pub hash: String,
    pub bucket: String,
    /// The requested revision's catalog origin, when the link named one.
    pub catalog: Option<String>,
}

impl RevisionMismatch {
    /// `mismatch` alone decides. A missing `mrbucket` reads as empty, as v1
    /// has always read it, so the lookup fails and the band shows the hash.
    pub fn from_query(query: &leptos_router::params::ParamsMap) -> Option<Self> {
        Some(Self {
            hash: query.get("mismatch")?,
            bucket: query.get("mrbucket").unwrap_or_default(),
            catalog: query.get("mrcatalog"),
        })
    }
}

/// `href` with the deep link's mismatch parameters after it, or unchanged when
/// there is none. The one place they are written, so a site that builds an
/// address for the same package cannot drop the band by forgetting one.
pub fn keeping_mismatch(href: String, mismatch: Option<&RevisionMismatch>) -> String {
    let Some(RevisionMismatch {
        hash,
        bucket,
        catalog,
    }) = mismatch
    else {
        return href;
    };
    let hash = urlencoding::encode(hash);
    let bucket = urlencoding::encode(bucket);
    let href = format!("{href}&mismatch={hash}&mrbucket={bucket}");
    match catalog {
        Some(catalog) => format!("{href}&mrcatalog={}", urlencoding::encode(catalog)),
        None => href,
    }
}

/// Where the [Sign in] button goes. `pages/login.rs` reads both parameters from
/// the query string; `back` is where login returns the user afterwards.
///
/// `back` is `/`, not `/main`: `/` renders whichever main page is switched on, so
/// it comes back here for a reader who has this one, and it is a route the
/// backend can read. OAuth login parses `back` in `routes::Paths` — an address
/// only the client router knows is not a way back at all.
///
/// Shared, not page-local: the roster's Accounts card, the queue's own
/// `[Sign in]` (§4.3) and the v2 package header all send a reader to the same
/// place, and a second copy of the format string is a copy that drifts.
pub fn sign_in_href(host: &str) -> String {
    let back = urlencoding::encode("/");
    format!("/login?host={host}&back={back}")
}

/// The commit screen for one package.
pub fn commit_href(namespace: &Namespace) -> String {
    let namespace = namespace.to_string();
    let namespace = urlencoding::encode(&namespace);
    format!("/commit?namespace={namespace}")
}

/// The merge screen for one package.
pub fn merge_href(namespace: &Namespace) -> String {
    let namespace = namespace.to_string();
    let namespace = urlencoding::encode(&namespace);
    format!("/merge?namespace={namespace}")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns(text: &str) -> Namespace {
        Namespace::try_from(text).expect("a namespace")
    }

    /// `back` is `/`, not `/main`: `/` renders whichever main page is switched
    /// on, and it is a route the backend's own `routes::Paths` can read.
    #[test]
    fn the_sign_in_link_comes_back_to_the_page_the_reader_was_on() {
        assert_eq!(
            sign_in_href("open.quiltdata.com"),
            "/login?host=open.quiltdata.com&back=%2F"
        );
    }

    /// The ordinary shape, pinned whole rather than by substring: a substring
    /// match cannot tell a missing namespace from a present one.
    #[test]
    fn a_plain_namespace_keeps_its_slash_readable() {
        assert_eq!(
            package_page_href(&ns("org/pkg")),
            "/installed-package?namespace=org%2Fpkg&filter=unmodified"
        );
        assert_eq!(commit_href(&ns("org/pkg")), "/commit?namespace=org%2Fpkg");
        assert_eq!(merge_href(&ns("org/pkg")), "/merge?namespace=org%2Fpkg");
    }

    /// The mode is the plain address plus one parameter, so leaving it is
    /// exactly dropping that parameter; the namespace stays one parameter.
    #[test]
    fn the_resolve_mode_is_the_plain_address_plus_one_parameter() {
        assert_eq!(
            resolve_href(&ns("org/pkg")),
            "/installed-package?namespace=org%2Fpkg&filter=unmodified&resolve=1"
        );
        assert_eq!(
            resolve_href(&ns("team/a&b")),
            "/installed-package?namespace=team%2Fa%26b&filter=unmodified&resolve=1"
        );
    }

    /// The defect this module exists for. `&` starts a new parameter and `#` a
    /// fragment, so either one raw ends the namespace early and the rest of the
    /// address means something the caller never wrote.
    #[test]
    fn a_namespace_cannot_end_the_parameter_early() {
        assert_eq!(
            commit_href(&ns("team/a&b")),
            "/commit?namespace=team%2Fa%26b",
            "`&` would otherwise start a second parameter"
        );
        assert_eq!(
            commit_href(&ns("team/a#b")),
            "/commit?namespace=team%2Fa%23b",
            "`#` would otherwise truncate the query at a fragment"
        );
    }

    /// `=` is **not** one of the above, and the distinction is worth a test of
    /// its own rather than a line in the one before it: a parser splits a pair
    /// on its *first* `=`, so a later one stays in the value and a raw
    /// `team/a=b` already round-tripped. It is encoded because encoding the
    /// whole value is what makes that true of every parser rather than of the
    /// two this app happens to use — not because it was losing data.
    #[test]
    fn an_equals_is_encoded_for_uniformity_not_because_it_split() {
        assert_eq!(
            commit_href(&ns("team/a=b")),
            "/commit?namespace=team%2Fa%3Db"
        );
    }

    /// `filter` has to survive whatever the namespace contains — a raw `&` in
    /// the namespace used to be indistinguishable from the separator before it.
    #[test]
    fn the_filter_stays_its_own_parameter() {
        assert_eq!(
            package_page_href(&ns("team/a&filter=all")),
            "/installed-package?namespace=team%2Fa%26filter%3Dall&filter=unmodified"
        );
    }

    fn mismatch(catalog: Option<&str>) -> RevisionMismatch {
        RevisionMismatch {
            hash: "c41d8f02".to_string(),
            bucket: "quilt-lab".to_string(),
            catalog: catalog.map(ToString::to_string),
        }
    }

    /// The deep link's three parameters ride after whatever the address
    /// already says, encoded as the namespace is, and nothing is added for an
    /// address that carried none.
    #[test]
    fn a_mismatch_rides_on_the_package_address() {
        assert_eq!(
            keeping_mismatch(
                package_page_href(&ns("org/pkg")),
                Some(&mismatch(Some("https://open.quilt.bio")))
            ),
            "/installed-package?namespace=org%2Fpkg&filter=unmodified\
             &mismatch=c41d8f02&mrbucket=quilt-lab&mrcatalog=https%3A%2F%2Fopen.quilt.bio"
        );
        assert_eq!(
            keeping_mismatch(resolve_href(&ns("org/pkg")), Some(&mismatch(None))),
            "/installed-package?namespace=org%2Fpkg&filter=unmodified&resolve=1\
             &mismatch=c41d8f02&mrbucket=quilt-lab"
        );
        assert_eq!(
            keeping_mismatch(package_page_href(&ns("org/pkg")), None),
            package_page_href(&ns("org/pkg"))
        );
    }

    /// What the router decodes is what was carried: `mismatch` alone decides,
    /// and a missing bucket reads as empty, as v1 has always read it. In the
    /// browser, because `ParamsMap` unescapes through JavaScript.
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn the_query_reads_back_what_was_carried() {
        let mut query = leptos_router::params::ParamsMap::new();
        assert_eq!(RevisionMismatch::from_query(&query), None);

        query.insert("mismatch", "c41d8f02".to_string());
        query.insert("mrbucket", "quilt-lab".to_string());
        assert_eq!(RevisionMismatch::from_query(&query), Some(mismatch(None)));

        query.insert("mrcatalog", "https://open.quilt.bio".to_string());
        assert_eq!(
            RevisionMismatch::from_query(&query),
            Some(mismatch(Some("https://open.quilt.bio")))
        );
    }
}
