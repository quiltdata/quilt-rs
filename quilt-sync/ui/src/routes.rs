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
//! [`use_query`] decodes on the way in, once, as does the backend's own parse of a
//! `back` parameter (`src-tauri/src/engine/routes.rs`), so the encoding is invisible to
//! everything downstream.

use quilt_uri::Namespace;

/// The installed-package screen for one package.
///
/// `namespace` goes in the query string because `installed_package` reads it
/// with [`use_query`], and a bare path leaves it empty. `filter=unmodified`
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

/// What a deep link found out about the package it landed on, when it found
/// anything. `pages/remote_package.rs` writes it into the package address and
/// the package page reads it back.
///
/// It lives in the address rather than in page state because it belongs to the
/// address: it stands while the address carries it, so every address the page
/// builds for the same package must carry it on — see [`keeping_outcome`].
///
/// # One or the other
///
/// An install answers either a different revision or a package with no
/// remote, never both, so one value holds whichever it was. An address that
/// somehow says both reads as [`DeepLinkOutcome::LocalOnly`]: with no remote there
/// is nothing to compare the requested revision against.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DeepLinkOutcome {
    /// The link asked for a revision other than the installed one.
    Mismatch(RevisionMismatch),
    /// The package is installed with no remote, so the link's revision could
    /// not be checked.
    LocalOnly,
}

impl DeepLinkOutcome {
    /// `localOnly` decides first, then `mismatch` — see [`RevisionMismatch`].
    pub fn from_query(query: &Query) -> Option<Self> {
        if query.get("localOnly").is_some() {
            return Some(Self::LocalOnly);
        }
        RevisionMismatch::from_query(query).map(Self::Mismatch)
    }

    /// The mismatch, when that is the outcome.
    pub fn mismatch(&self) -> Option<&RevisionMismatch> {
        match self {
            Self::Mismatch(mismatch) => Some(mismatch),
            Self::LocalOnly => None,
        }
    }
}

/// The requested top hash and the remote it lives on.
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
    fn from_query(query: &Query) -> Option<Self> {
        Some(Self {
            hash: query.get("mismatch")?,
            bucket: query.get("mrbucket").unwrap_or_default(),
            catalog: query.get("mrcatalog"),
        })
    }
}

/// `href` with the deep link's outcome after it, or unchanged when there is none.
/// The one place its parameters are written, so a site that builds an address
/// for the same package cannot drop the band by forgetting one.
pub fn keeping_outcome(href: String, outcome: Option<&DeepLinkOutcome>) -> String {
    let mismatch = match outcome {
        None => return href,
        Some(DeepLinkOutcome::LocalOnly) => return format!("{href}&localOnly=1"),
        Some(DeepLinkOutcome::Mismatch(mismatch)) => mismatch,
    };
    let RevisionMismatch {
        hash,
        bucket,
        catalog,
    } = mismatch;
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
/// A page passes its own address as `back`, so the reader comes back to it.
/// The main page passes `/`, not `/main`: `/` renders whichever main page is
/// switched on. OAuth login parses `back` in the backend's `routes::Paths`, so
/// `back` must be a route that enum knows: an address only the client router
/// knows is not a way back at all.
pub fn sign_in_href(host: &str, back: &str) -> String {
    let back = urlencoding::encode(back);
    format!("/login?host={host}&back={back}")
}

/// The commit screen for one package.
pub fn commit_href(namespace: &Namespace) -> String {
    let namespace = namespace.to_string();
    let namespace = urlencoding::encode(&namespace);
    format!("/commit?namespace={namespace}")
}

/// The commit screen as *Create new revision* opens it: with the message
/// focused, because the reader came to write one. Every other way in focuses
/// nothing.
pub fn new_revision_href(namespace: &Namespace) -> String {
    format!("{}&focus=message", commit_href(namespace))
}

/// The merge screen for one package.
pub fn merge_href(namespace: &Namespace) -> String {
    let namespace = namespace.to_string();
    let namespace = urlencoding::encode(&namespace);
    format!("/merge?namespace={namespace}")
}

/// A page's query string, each value decoded once.
///
/// `leptos_router`'s `use_query_map` decodes each value twice: the browser's
/// `URLSearchParams` decodes it, then `ParamsMap::insert` unescapes it again.
/// A value that is itself an encoded address, like the login page's `back`
/// pointing at a deep link, then loses its own encoding: `quilt%2Bs3` comes
/// back as `quilt+s3`, unencoded, and its `#` and `&` split it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Query(Vec<(String, String)>);

impl Query {
    /// Parse a query string, with or without its leading `?`.
    #[must_use]
    pub fn parse(search: &str) -> Self {
        let Ok(params) = web_sys::UrlSearchParams::new_with_str(search) else {
            return Self::default();
        };
        let mut pairs = Vec::new();
        let entries = js_sys::try_iter(&params).ok().flatten();
        for pair in entries.into_iter().flatten().flatten() {
            let pair = js_sys::Array::from(&pair);
            if let (Some(key), Some(value)) = (pair.get(0).as_string(), pair.get(1).as_string()) {
                pairs.push((key, value));
            }
        }
        Self(pairs)
    }

    /// The last value given for `key`, as `ParamsMap::get` answers.
    #[must_use]
    pub fn get(&self, key: &str) -> Option<String> {
        self.0
            .iter()
            .rev()
            .find(|(k, _)| k == key)
            .map(|(_, v)| v.clone())
    }
}

/// The current query, decoded once; see [`Query`]. Every page reads its
/// query through it, never through `use_query_map`.
#[must_use]
pub fn use_query() -> leptos::prelude::Memo<Query> {
    use leptos::prelude::*;
    // The route's own URL, as `use_query_map` reads it: it stays put while the
    // router leaves this route, so a page does not read an address it is not.
    let url = leptos_router::hooks::use_url();
    Memo::new(move |_| url.with(|url| Query::parse(url.search())))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A `back` that is itself an encoded address keeps its encoding.
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn a_query_value_is_decoded_once() {
        let relay = "/remote-package?uri=quilt%2Bs3%3A%2F%2Fb%23package%3Da%2Fb%26catalog%3Dc";
        let search = format!("?host=quilt.test&back={}", urlencoding::encode(relay));
        let query = Query::parse(&search);
        assert_eq!(query.get("back").as_deref(), Some(relay));
        assert_eq!(query.get("host").as_deref(), Some("quilt.test"));
        assert_eq!(query.get("missing"), None);
    }

    fn ns(text: &str) -> Namespace {
        Namespace::try_from(text).expect("a namespace")
    }

    /// `back` is the page's address encoded once, as a query value.
    #[test]
    fn the_sign_in_link_comes_back_to_the_page_the_reader_was_on() {
        assert_eq!(
            sign_in_href("quilt.test", "/"),
            "/login?host=quilt.test&back=%2F"
        );
        let back = package_page_href(&ns("org/pkg"));
        let href = sign_in_href("quilt.test", &back);
        assert_eq!(
            href,
            "/login?host=quilt.test&back=%2Finstalled-package%3Fnamespace%3Dorg%252Fpkg%26filter%3Dunmodified"
        );
    }

    /// What the login page reads back is the page's address as it was.
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn the_login_page_reads_back_the_page_address() {
        let back = package_page_href(&ns("org/pkg"));
        let href = sign_in_href("quilt.test", &back);
        let query = Query::parse(href.trim_start_matches("/login"));
        assert_eq!(query.get("back"), Some(back));
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
        assert_eq!(
            new_revision_href(&ns("org/pkg")),
            "/commit?namespace=org%2Fpkg&focus=message"
        );
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

    fn mismatch(catalog: Option<&str>) -> DeepLinkOutcome {
        DeepLinkOutcome::Mismatch(RevisionMismatch {
            hash: "c41d8f02".to_string(),
            bucket: "quilt-lab".to_string(),
            catalog: catalog.map(ToString::to_string),
        })
    }

    /// The deep link's three parameters ride after whatever the address
    /// already says, encoded as the namespace is, and nothing is added for an
    /// address that carried none.
    #[test]
    fn a_mismatch_rides_on_the_package_address() {
        assert_eq!(
            keeping_outcome(
                package_page_href(&ns("org/pkg")),
                Some(&mismatch(Some("https://quilt.test")))
            ),
            "/installed-package?namespace=org%2Fpkg&filter=unmodified\
             &mismatch=c41d8f02&mrbucket=quilt-lab&mrcatalog=https%3A%2F%2Fquilt.test"
        );
        assert_eq!(
            keeping_outcome(resolve_href(&ns("org/pkg")), Some(&mismatch(None))),
            "/installed-package?namespace=org%2Fpkg&filter=unmodified&resolve=1\
             &mismatch=c41d8f02&mrbucket=quilt-lab"
        );
        assert_eq!(
            keeping_outcome(package_page_href(&ns("org/pkg")), None),
            package_page_href(&ns("org/pkg"))
        );
    }

    /// A package with no remote gets one flag, after whatever the address
    /// already says, as the mismatch does.
    #[test]
    fn local_only_rides_on_the_package_address() {
        assert_eq!(
            keeping_outcome(
                package_page_href(&ns("org/pkg")),
                Some(&DeepLinkOutcome::LocalOnly)
            ),
            "/installed-package?namespace=org%2Fpkg&filter=unmodified&localOnly=1"
        );
        assert_eq!(
            keeping_outcome(
                resolve_href(&ns("org/pkg")),
                Some(&DeepLinkOutcome::LocalOnly)
            ),
            "/installed-package?namespace=org%2Fpkg&filter=unmodified&resolve=1&localOnly=1"
        );
    }

    /// What the address carries is what is read back: `mismatch` alone
    /// decides, and a missing bucket reads as empty, as v1 has always read
    /// it. In the browser, because [`Query`] parses through `URLSearchParams`.
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn the_query_reads_back_what_was_carried() {
        let href = package_page_href(&ns("org/pkg"));
        let search = |href: &str| Query::parse(href.split_once('?').expect("a query").1);
        assert_eq!(DeepLinkOutcome::from_query(&search(&href)), None);

        let outcome = mismatch(None);
        let carried = keeping_outcome(href.clone(), Some(&outcome));
        assert_eq!(
            DeepLinkOutcome::from_query(&search(&carried)),
            Some(outcome)
        );

        let outcome = mismatch(Some("https://quilt.test"));
        let carried = keeping_outcome(href.clone(), Some(&outcome));
        assert_eq!(
            DeepLinkOutcome::from_query(&search(&carried)),
            Some(outcome)
        );

        let bucketless = format!("{href}&mismatch=c41d8f02");
        assert_eq!(
            DeepLinkOutcome::from_query(&search(&bucketless)),
            Some(DeepLinkOutcome::Mismatch(RevisionMismatch {
                hash: "c41d8f02".to_string(),
                bucket: String::new(),
                catalog: None,
            }))
        );
    }

    /// The flag alone is the outcome, and it wins over a mismatch in the same
    /// address: with no remote there is nothing to compare against.
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn local_only_reads_back_and_wins_over_a_mismatch() {
        assert_eq!(
            DeepLinkOutcome::from_query(&Query::parse("?localOnly=1")),
            Some(DeepLinkOutcome::LocalOnly)
        );
        assert_eq!(
            DeepLinkOutcome::from_query(&Query::parse(
                "?localOnly=1&mismatch=c41d8f02&mrbucket=quilt-lab"
            )),
            Some(DeepLinkOutcome::LocalOnly)
        );
    }
}
