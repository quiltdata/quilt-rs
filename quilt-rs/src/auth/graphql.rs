//! Hand-rolled GraphQL calls against the registry's `/graphql` endpoint.
//!
//! The role surface lives here — `me`, `switchRole`, and the role-scoped
//! `buckets` list — plus `package`'s revision list and, from the registry's
//! search, every package's `latest` hash. None of these has a REST
//! equivalent, which is the only reason quilt-rs speaks GraphQL at all;
//! everything else in the auth layer stays on the REST endpoints. Deliberately a few typed
//! documents over the existing `HttpClient` rather than a generated client.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;
use tracing::debug;
use tracing::warn;

use crate::Res;
use crate::error::RoleError;
use crate::io::remote::PublishedRevision;
use crate::io::remote::client::HttpClient;
use quilt_uri::Host;
use quilt_uri::Namespace;
use quilt_uri::is_top_hash;

const ME_QUERY: &str = "query { me { role { name } roles { name } } }";

const SWITCH_ROLE_MUTATION: &str = "\
mutation($roleName: String!) { \
switchRole(roleName: $roleName) { \
__typename \
... on Me { role { name } roles { name } } \
... on InvalidInput { errors { message } } \
... on OperationError { message } \
} }";

#[derive(Serialize)]
struct GraphQlRequest<'a, V: Serialize> {
    query: &'a str,
    variables: V,
}

#[derive(Deserialize)]
struct GraphQlResponse<T> {
    data: Option<T>,
    #[serde(default)]
    errors: Vec<GraphQlTopLevelError>,
}

#[derive(Deserialize)]
struct GraphQlTopLevelError {
    message: String,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
pub(super) struct MyRole {
    pub(super) name: String,
}

#[derive(Deserialize, Debug, PartialEq, Eq)]
pub(super) struct Me {
    pub(super) role: MyRole,
    pub(super) roles: Vec<MyRole>,
}

#[derive(Deserialize)]
struct MeData {
    me: Option<Me>,
}

fn graphql_url(registry: &url::Host) -> String {
    format!("https://{registry}/graphql")
}

/// POST a document and unwrap the `data` envelope, mapping a top-level
/// `errors` array onto [`RoleError::GraphQl`].
async fn execute<T: DeserializeOwned, V: Serialize + Send + Sync>(
    http_client: &impl HttpClient,
    registry: &url::Host,
    query: &str,
    variables: V,
    access_token: &str,
) -> Res<T> {
    let request = GraphQlRequest { query, variables };
    let response: GraphQlResponse<T> = http_client
        .post_json_auth(&graphql_url(registry), &request, access_token)
        .await?;

    if !response.errors.is_empty() {
        let joined = response
            .errors
            .into_iter()
            .map(|e| e.message)
            .collect::<Vec<_>>()
            .join("; ");
        return Err(RoleError::GraphQl(joined).into());
    }

    response
        .data
        .ok_or_else(|| RoleError::GraphQl("registry returned no data".to_string()).into())
}

/// Two host params on purpose: `registry` is where the request goes, while
/// `host` is the catalog host the user knows and the one
/// [`RoleError::NotAuthenticated`] must name. The registry host cannot
/// stand in for it — they are different names for different services.
pub(super) async fn query_me(
    http_client: &impl HttpClient,
    registry: &url::Host,
    host: &Host,
    access_token: &str,
) -> Res<Me> {
    let data: MeData = execute(http_client, registry, ME_QUERY, (), access_token).await?;
    data.me
        .ok_or_else(|| RoleError::NotAuthenticated(host.to_owned()))
        .map_err(Into::into)
}

#[derive(Serialize)]
struct SwitchRoleVariables<'a> {
    #[serde(rename = "roleName")]
    role_name: &'a str,
}

/// `union SwitchRoleResult = Me | InvalidInput | OperationError`, discriminated
/// on `__typename`. Both failure arms collapse to one client-facing error —
/// the distinction between "bad role name" and "server refused" is not
/// actionable differently in the UI.
#[derive(Deserialize)]
#[serde(tag = "__typename")]
enum SwitchRoleResult {
    Me(Me),
    InvalidInput { errors: Vec<InputError> },
    OperationError { message: String },
}

#[derive(Deserialize)]
struct InputError {
    message: String,
}

#[derive(Deserialize)]
struct SwitchRoleData {
    #[serde(rename = "switchRole")]
    switch_role: SwitchRoleResult,
}

/// Unlike [`query_me`], no `host` param: the result is non-null and the union
/// has no unauthenticated arm, so there is no catalog host to name.
pub(super) async fn mutate_switch_role(
    http_client: &impl HttpClient,
    registry: &url::Host,
    role_name: &str,
    access_token: &str,
) -> Res<Me> {
    let data: SwitchRoleData = execute(
        http_client,
        registry,
        SWITCH_ROLE_MUTATION,
        SwitchRoleVariables { role_name },
        access_token,
    )
    .await?;

    match data.switch_role {
        SwitchRoleResult::Me(me) => Ok(me),
        SwitchRoleResult::InvalidInput { errors } => {
            let joined = errors
                .into_iter()
                .map(|e| e.message)
                .collect::<Vec<_>>()
                .join("; ");
            Err(RoleError::SwitchRejected(joined).into())
        }
        SwitchRoleResult::OperationError { message } => {
            Err(RoleError::SwitchRejected(message).into())
        }
    }
}

/// `buckets` — the role-scoped list. Deliberately **not** `bucketConfigs`,
/// which is the admin-bypass list and would report buckets the active role
/// cannot reach. Read-presence only: it never distinguishes READ from
/// `READ_WRITE`, and on `ALLOW_ANONYMOUS_ACCESS` stacks or under an unmanaged
/// role it returns every in-stack bucket, so callers must treat it as an
/// optimistic hint rather than an authoritative access answer.
const BUCKETS_QUERY: &str = "query { buckets { name } }";

#[derive(Deserialize)]
struct BucketName {
    name: String,
}

#[derive(Deserialize)]
struct BucketsData {
    buckets: Vec<BucketName>,
}

pub(super) async fn query_buckets(
    http_client: &impl HttpClient,
    registry: &url::Host,
    access_token: &str,
) -> Res<Vec<String>> {
    let data: BucketsData = execute(http_client, registry, BUCKETS_QUERY, (), access_token).await?;
    Ok(data.buckets.into_iter().map(|b| b.name).collect())
}

/// `package.revisions` — one entry per timestamped pointer under
/// `.quilt/named_packages/<name>/`. `hash` only: the registry caps a
/// document's cost, and the pointer is all a caller needs.
const PACKAGE_REVISIONS_QUERY: &str = "\
query($bucket: String!, $name: String!, $number: Int!, $perPage: Int!) { \
package(bucket: $bucket, name: $name) { \
revisions { total page(number: $number, perPage: $perPage) { hash } } \
} }";

const REVISIONS_PER_PAGE: u32 = 100;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PackageRevisionsVariables<'a> {
    bucket: &'a str,
    name: String,
    number: u32,
    per_page: u32,
}

/// [`PACKAGE_REVISIONS_QUERY`] with each pointer's time, `modified`, which
/// the registry reads from the pointer's name. A message is left out: the
/// registry would read every revision's manifest for it.
const PACKAGE_REVISION_TIMES_QUERY: &str = "\
query($bucket: String!, $name: String!, $number: Int!, $perPage: Int!) { \
package(bucket: $bucket, name: $name) { \
revisions { total page(number: $number, perPage: $perPage) { hash modified } } \
} }";

#[derive(Deserialize)]
struct RevisionHash {
    hash: String,
    /// Asked for by [`PACKAGE_REVISION_TIMES_QUERY`] only.
    #[serde(default)]
    modified: Option<String>,
}

#[derive(Deserialize)]
struct RevisionPage {
    total: u32,
    page: Vec<RevisionHash>,
}

#[derive(Deserialize)]
struct PackageRevisions {
    revisions: RevisionPage,
}

#[derive(Deserialize)]
struct PackageRevisionsData {
    package: Option<PackageRevisions>,
}

/// The hashes of every revision the registry lists for `namespace` in
/// `bucket`, page by page. A package the registry does not know (never
/// pushed) lists nothing; one that vanishes after the first page is an
/// error, since the pages already read are not its whole history.
pub(super) async fn query_package_revisions(
    http_client: &impl HttpClient,
    registry: &url::Host,
    bucket: &str,
    namespace: &Namespace,
    access_token: &str,
) -> Res<Vec<String>> {
    let rows = query_revision_rows(
        http_client,
        registry,
        bucket,
        namespace,
        access_token,
        PACKAGE_REVISIONS_QUERY,
    )
    .await?;
    Ok(rows.into_iter().map(|row| row.hash).collect())
}

/// [`query_package_revisions`], with the time each pointer is named for,
/// where the registry's answer can be read as one.
pub(super) async fn query_package_revision_history(
    http_client: &impl HttpClient,
    registry: &url::Host,
    bucket: &str,
    namespace: &Namespace,
    access_token: &str,
) -> Res<Vec<PublishedRevision>> {
    let rows = query_revision_rows(
        http_client,
        registry,
        bucket,
        namespace,
        access_token,
        PACKAGE_REVISION_TIMES_QUERY,
    )
    .await?;
    Ok(rows
        .into_iter()
        .map(|row| PublishedRevision {
            hash: row.hash,
            modified: row.modified.as_deref().and_then(parse_datetime),
            message: None,
        })
        .collect())
}

/// A registry `Datetime`: RFC 3339, or an ISO 8601 time with no offset,
/// which the registry writes in UTC.
fn parse_datetime(text: &str) -> Option<jiff::Timestamp> {
    text.parse::<jiff::Timestamp>().ok().or_else(|| {
        text.parse::<jiff::civil::DateTime>()
            .ok()
            .and_then(|time| jiff::tz::TimeZone::UTC.to_timestamp(time).ok())
    })
}

/// The rows `query` lists for `namespace` in `bucket`, page by page; see
/// [`query_package_revisions`].
async fn query_revision_rows(
    http_client: &impl HttpClient,
    registry: &url::Host,
    bucket: &str,
    namespace: &Namespace,
    access_token: &str,
    query: &str,
) -> Res<Vec<RevisionHash>> {
    let mut hashes = Vec::new();
    for number in 1.. {
        let variables = PackageRevisionsVariables {
            bucket,
            name: namespace.to_string(),
            number,
            per_page: REVISIONS_PER_PAGE,
        };
        let data: PackageRevisionsData =
            execute(http_client, registry, query, variables, access_token).await?;
        let Some(package) = data.package else {
            if number == 1 {
                break;
            }
            return Err(RoleError::GraphQl(format!(
                "package {namespace} vanished from the registry while listing page {number}"
            ))
            .into());
        };
        let RevisionPage { total, page } = package.revisions;
        // A page drops pointers that vanished before resolving, so a short or
        // even empty page is not the end: only `total` says whether more follow.
        hashes.extend(page);
        if u64::from(number) * u64::from(REVISIONS_PER_PAGE) >= u64::from(total) {
            break;
        }
    }
    Ok(hashes)
}

/// `searchPackages` with `latestOnly`: the registry's index of every
/// package's `latest` pointer. Only the page arm of each union is selected,
/// so older registries, whose `firstPage` is the page itself, accept the
/// same document.
const LATEST_HASHES_QUERY: &str = "\
query($bucket: String!, $size: Int!) { \
searchPackages(buckets: [$bucket], latestOnly: true) { \
__typename \
... on PackagesSearchResultSet { total firstPage(size: $size) { \
__typename ... on PackagesSearchResultSetPage { cursor hits { name hash } } \
} } } }";

const MORE_LATEST_HASHES_QUERY: &str = "\
query($after: String!, $size: Int!) { \
searchMorePackages(after: $after, size: $size) { \
__typename ... on PackagesSearchResultSetPage { cursor hits { name hash } } \
} }";

/// The registry's largest search page.
const SEARCH_PAGE_SIZE: usize = 100;

/// How far the registry's search counts and pages: its index's result
/// window. A count that reaches it may be short of the bucket's packages.
const SEARCH_WINDOW: usize = 10_000;

#[derive(Serialize)]
struct LatestHashesVariables<'a> {
    bucket: &'a str,
    size: usize,
}

#[derive(Serialize)]
struct MoreLatestHashesVariables<'a> {
    after: &'a str,
    size: usize,
}

#[derive(Deserialize)]
struct LatestHit {
    name: String,
    hash: String,
}

#[derive(Deserialize)]
#[serde(tag = "__typename")]
enum LatestPage {
    PackagesSearchResultSetPage {
        cursor: Option<String>,
        hits: Vec<LatestHit>,
    },
    #[serde(other)]
    Refused,
}

#[derive(Deserialize)]
#[serde(tag = "__typename")]
enum LatestSearch {
    PackagesSearchResultSet {
        total: i64,
        #[serde(rename = "firstPage")]
        first_page: LatestPage,
    },
    #[serde(other)]
    Refused,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct LatestHashesData {
    search_packages: LatestSearch,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct MoreLatestHashesData {
    search_more_packages: LatestPage,
}

/// Every package in `bucket` with the hash its `latest` names, as the
/// registry's search has indexed them, page by page. An error wherever the
/// listing cannot be shown whole: the search finds nothing, which is also
/// its answer for a bucket the role cannot read or it has not indexed; it
/// hides its count, as a secure search does; the count reaches its window;
/// or the pages hold fewer packages than it counted.
pub(super) async fn query_latest_hashes(
    http_client: &impl HttpClient,
    registry: &url::Host,
    bucket: &str,
    access_token: &str,
) -> Res<Vec<(Namespace, String)>> {
    let refused = || RoleError::GraphQl(format!("the registry did not list {bucket}'s packages"));
    let first: LatestHashesData = execute(
        http_client,
        registry,
        LATEST_HASHES_QUERY,
        LatestHashesVariables {
            bucket,
            size: SEARCH_PAGE_SIZE,
        },
        access_token,
    )
    .await?;
    let LatestSearch::PackagesSearchResultSet {
        total,
        first_page: mut page,
    } = first.search_packages
    else {
        return Err(refused().into());
    };
    let total = match usize::try_from(total) {
        Ok(total) if total < SEARCH_WINDOW => total,
        _ => return Err(refused().into()),
    };

    // Keyed by name: a package that moves between pages while they are read
    // is seen twice.
    let mut latest = BTreeMap::new();
    // No listing within the window takes more pages than this, so a cursor
    // that keeps going, as one that cycles, is cut off.
    for _ in 0..SEARCH_WINDOW / SEARCH_PAGE_SIZE {
        let LatestPage::PackagesSearchResultSetPage { cursor, hits } = page else {
            return Err(refused().into());
        };
        latest.extend(hits.into_iter().map(|hit| (hit.name, hit.hash)));
        let Some(after) = cursor else {
            return whole_listing(latest, total, bucket);
        };
        let more: MoreLatestHashesData = execute(
            http_client,
            registry,
            MORE_LATEST_HASHES_QUERY,
            MoreLatestHashesVariables {
                after: &after,
                size: SEARCH_PAGE_SIZE,
            },
            access_token,
        )
        .await?;
        page = more.search_more_packages;
    }
    Err(refused().into())
}

/// The listing, if it holds as many packages as the search counted, less
/// any name that is no package or `latest` that is no top hash.
fn whole_listing(
    latest: BTreeMap<String, String>,
    total: usize,
    bucket: &str,
) -> Res<Vec<(Namespace, String)>> {
    if latest.len() < total {
        return Err(RoleError::GraphQl(format!(
            "the registry listed {} of the {total} packages in {bucket}",
            latest.len()
        ))
        .into());
    }

    Ok(latest
        .into_iter()
        .filter_map(|(name, hash)| {
            let Ok(namespace) = Namespace::try_from(name.as_str()) else {
                debug!("Skipping {name}, which names no package");
                return None;
            };
            if !is_top_hash(&hash) {
                warn!("Skipping {name}, whose latest is no top hash");
                return None;
            }
            Some((namespace, hash))
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::Error;
    use crate::auth::test_utils::ACCESS_TOKEN;
    use crate::auth::test_utils::GraphQlTestHttpClient;
    use crate::auth::test_utils::SearchFault;
    use crate::auth::test_utils::get_registry_host;
    use quilt_uri::fixtures;

    #[test(tokio::test)]
    async fn query_me_returns_active_and_available_roles() -> Res {
        let client = GraphQlTestHttpClient::default();
        let me = query_me(
            &client,
            &get_registry_host(),
            &fixtures::host(),
            ACCESS_TOKEN,
        )
        .await?;

        assert_eq!(me.role.name, "ReadWrite");
        assert_eq!(
            me.roles.iter().map(|r| r.name.as_str()).collect::<Vec<_>>(),
            vec!["ReadWrite", "ReadOnly"]
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn query_me_maps_null_me_to_not_authenticated() {
        let client = GraphQlTestHttpClient {
            me_is_null: true,
            ..GraphQlTestHttpClient::default()
        };
        let err = query_me(
            &client,
            &get_registry_host(),
            &fixtures::host(),
            ACCESS_TOKEN,
        )
        .await
        .unwrap_err();

        assert!(
            matches!(err, Error::Role(RoleError::NotAuthenticated(_))),
            "expected NotAuthenticated, got {err:?}"
        );
    }

    #[test(tokio::test)]
    async fn query_me_surfaces_top_level_graphql_errors() {
        let client = GraphQlTestHttpClient {
            top_level_error: Some("field 'me' is deprecated".to_string()),
            ..GraphQlTestHttpClient::default()
        };
        let err = query_me(
            &client,
            &get_registry_host(),
            &fixtures::host(),
            ACCESS_TOKEN,
        )
        .await
        .unwrap_err();

        assert!(
            matches!(&err, Error::Role(RoleError::GraphQl(m)) if m.contains("deprecated")),
            "expected GraphQl error, got {err:?}"
        );
    }

    #[test(tokio::test)]
    async fn switch_role_returns_the_new_active_role() -> Res {
        let client = GraphQlTestHttpClient::default();
        let me =
            mutate_switch_role(&client, &get_registry_host(), "ReadOnly", ACCESS_TOKEN).await?;

        assert_eq!(me.role.name, "ReadOnly");
        Ok(())
    }

    #[test(tokio::test)]
    async fn switch_role_maps_invalid_input_to_switch_rejected() {
        let client = GraphQlTestHttpClient {
            switch_result: serde_json::json!({
                "__typename": "InvalidInput",
                "errors": [{"message": "no such role", "name": "InvalidRole"}],
            }),
            ..GraphQlTestHttpClient::default()
        };
        let err = mutate_switch_role(&client, &get_registry_host(), "Nope", ACCESS_TOKEN)
            .await
            .unwrap_err();

        assert!(
            matches!(&err, Error::Role(RoleError::SwitchRejected(m)) if m.contains("no such role")),
            "expected SwitchRejected, got {err:?}"
        );
    }

    #[test(tokio::test)]
    async fn switch_role_maps_operation_error_to_switch_rejected() {
        let client = GraphQlTestHttpClient {
            switch_result: serde_json::json!({
                "__typename": "OperationError",
                "message": "role is locked by SSO",
                "name": "RoleLocked",
            }),
            ..GraphQlTestHttpClient::default()
        };
        let err = mutate_switch_role(&client, &get_registry_host(), "ReadWrite", ACCESS_TOKEN)
            .await
            .unwrap_err();

        assert!(
            matches!(&err, Error::Role(RoleError::SwitchRejected(m)) if m.contains("locked")),
            "expected SwitchRejected, got {err:?}"
        );
    }

    #[test(tokio::test)]
    async fn query_buckets_returns_role_scoped_bucket_names() -> Res {
        let client = GraphQlTestHttpClient {
            buckets: vec!["readable-one", "readable-two"],
            ..GraphQlTestHttpClient::default()
        };
        let buckets = query_buckets(&client, &get_registry_host(), ACCESS_TOKEN).await?;

        assert_eq!(buckets, vec!["readable-one", "readable-two"]);
        Ok(())
    }

    #[test(tokio::test)]
    async fn query_buckets_returns_empty_when_the_role_reaches_nothing() -> Res {
        let client = GraphQlTestHttpClient {
            buckets: vec![],
            ..GraphQlTestHttpClient::default()
        };
        let buckets = query_buckets(&client, &get_registry_host(), ACCESS_TOKEN).await?;

        assert_eq!(buckets, [] as [String; 0]);
        Ok(())
    }

    /// A registry `Datetime` is read with its offset, or as UTC without one;
    /// anything else is no time.
    #[test]
    fn a_revision_time_is_read_with_or_without_an_offset() -> Result<(), jiff::Error> {
        let at = jiff::Timestamp::from_second(1_700_000_000)?;
        assert_eq!(parse_datetime("2023-11-14T22:13:20+00:00"), Some(at));
        assert_eq!(parse_datetime("2023-11-14T23:13:20+01:00"), Some(at));
        assert_eq!(parse_datetime("2023-11-14T22:13:20"), Some(at));
        assert_eq!(parse_datetime("yesterday"), None);
        Ok(())
    }

    /// 150 pointers, one of which vanished before it resolved: page 1 holds
    /// 100, page 2 the 49 left, and the short second page is not mistaken
    /// for the end — `total` is.
    #[test(tokio::test)]
    async fn query_package_revisions_reads_every_page() -> Res {
        let hashes: Vec<String> = (0..150).map(|i| format!("hash-{i:03}")).collect();
        let mut slots: Vec<Option<String>> = hashes.iter().cloned().map(Some).collect();
        slots[120] = None;
        let client = GraphQlTestHttpClient {
            package_revisions: Some(slots),
            ..GraphQlTestHttpClient::default()
        };
        let namespace: Namespace = ("team", "dataset").into();

        let listed = query_package_revisions(
            &client,
            &get_registry_host(),
            "quilt-bucket",
            &namespace,
            ACCESS_TOKEN,
        )
        .await?;

        let mut expected = hashes;
        expected.remove(120);
        assert_eq!(listed, expected);
        assert_eq!(
            *client.package_queries_seen.lock().unwrap(),
            vec![
                serde_json::json!({"bucket": "quilt-bucket", "name": "team/dataset", "number": 1, "perPage": 100}),
                serde_json::json!({"bucket": "quilt-bucket", "name": "team/dataset", "number": 2, "perPage": 100}),
            ]
        );
        Ok(())
    }

    /// A whole page whose pointers all vanished comes back empty, and the
    /// pages after it still hold revisions: only `total` ends the listing.
    #[test(tokio::test)]
    async fn query_package_revisions_reads_past_an_emptied_page() -> Res {
        let mut slots: Vec<Option<String>> =
            (0..250).map(|i| Some(format!("hash-{i:03}"))).collect();
        slots[100..200].fill(None);
        let client = GraphQlTestHttpClient {
            package_revisions: Some(slots),
            ..GraphQlTestHttpClient::default()
        };
        let namespace: Namespace = ("team", "dataset").into();

        let listed = query_package_revisions(
            &client,
            &get_registry_host(),
            "quilt-bucket",
            &namespace,
            ACCESS_TOKEN,
        )
        .await?;

        assert_eq!(listed.len(), 150);
        assert_eq!(listed.last().map(String::as_str), Some("hash-249"));
        Ok(())
    }

    /// A package deleted between pages answers page 2 with `package: null`.
    /// The 100 hashes already read are not the whole history, so the
    /// listing is refused rather than returned short.
    #[test(tokio::test)]
    async fn query_package_revisions_refuses_a_package_gone_mid_listing() {
        let client = GraphQlTestHttpClient {
            package_revisions: Some((0..150).map(|i| Some(format!("hash-{i:03}"))).collect()),
            package_gone_from_page: Some(2),
            ..GraphQlTestHttpClient::default()
        };
        let result = query_package_revisions(
            &client,
            &get_registry_host(),
            "quilt-bucket",
            &("team", "dataset").into(),
            ACCESS_TOKEN,
        )
        .await;

        assert!(
            matches!(&result, Err(Error::Role(RoleError::GraphQl(m))) if m.contains("team/dataset")),
            "expected GraphQl error, got {result:?}"
        );
        assert_eq!(client.package_queries_seen.lock().unwrap().len(), 2);
    }

    /// `package: null` is a package never pushed: nothing is listed.
    #[test(tokio::test)]
    async fn query_package_revisions_lists_nothing_for_no_such_package() -> Res {
        let client = GraphQlTestHttpClient::default();
        let listed = query_package_revisions(
            &client,
            &get_registry_host(),
            "quilt-bucket",
            &("team", "dataset").into(),
            ACCESS_TOKEN,
        )
        .await?;

        assert_eq!(listed, [] as [String; 0]);
        assert_eq!(client.package_queries_seen.lock().unwrap().len(), 1);
        Ok(())
    }

    #[test(tokio::test)]
    async fn query_package_revisions_surfaces_top_level_graphql_errors() {
        let client = GraphQlTestHttpClient {
            top_level_error: Some("query cost exceeds 42".to_string()),
            ..GraphQlTestHttpClient::default()
        };
        let err = query_package_revisions(
            &client,
            &get_registry_host(),
            "quilt-bucket",
            &("team", "dataset").into(),
            ACCESS_TOKEN,
        )
        .await
        .unwrap_err();

        assert!(
            matches!(&err, Error::Role(RoleError::GraphQl(m)) if m.contains("cost")),
            "expected GraphQl error, got {err:?}"
        );
    }

    fn top_hash(i: usize) -> String {
        format!("{i:064x}")
    }

    fn pointers(count: usize) -> Vec<(String, String)> {
        (0..count)
            .map(|i| (format!("team/pkg-{i:03}"), top_hash(i)))
            .collect()
    }

    async fn latest_hashes(client: &GraphQlTestHttpClient) -> Res<Vec<(Namespace, String)>> {
        query_latest_hashes(client, &get_registry_host(), "quilt-bucket", ACCESS_TOKEN).await
    }

    /// 250 packages come back in three pages, each asked for by the cursor
    /// the page before it gave.
    #[test(tokio::test)]
    async fn query_latest_hashes_reads_every_page() -> Res {
        let client = GraphQlTestHttpClient {
            latest_pointers: pointers(250),
            ..GraphQlTestHttpClient::default()
        };

        let listed = latest_hashes(&client).await?;

        let expected: Vec<(Namespace, String)> = pointers(250)
            .into_iter()
            .map(|(name, hash)| (Namespace::try_from(name.as_str()).unwrap(), hash))
            .collect();
        assert_eq!(listed, expected);
        assert_eq!(
            *client.search_queries_seen.lock().unwrap(),
            vec![
                serde_json::json!({"bucket": "quilt-bucket", "size": 100}),
                serde_json::json!({"after": "100", "size": 100}),
                serde_json::json!({"after": "200", "size": 100}),
            ]
        );
        Ok(())
    }

    /// The search stops paging at its window, here after 100 of 250: the
    /// 100 are not the bucket's packages, so the listing is refused.
    #[test(tokio::test)]
    async fn query_latest_hashes_refuses_a_listing_cut_short() {
        let client = GraphQlTestHttpClient {
            latest_pointers: pointers(250),
            search_window: 100,
            ..GraphQlTestHttpClient::default()
        };

        let result = latest_hashes(&client).await;

        assert!(
            matches!(&result, Err(Error::Role(RoleError::GraphQl(m))) if m.contains("100 of the 250")),
            "expected GraphQl error, got {result:?}"
        );
    }

    /// A registry with secure search reports its count as -1, and stops
    /// paging at its window all the same, so no listing can be shown whole.
    #[test(tokio::test)]
    async fn query_latest_hashes_refuses_a_registry_that_hides_its_count() {
        let client = GraphQlTestHttpClient {
            latest_pointers: pointers(150),
            search_fault: Some(SearchFault::HiddenTotal),
            ..GraphQlTestHttpClient::default()
        };

        let result = latest_hashes(&client).await;

        assert!(
            matches!(&result, Err(Error::Role(RoleError::GraphQl(m))) if m.contains("quilt-bucket")),
            "expected GraphQl error, got {result:?}"
        );
    }

    /// A cursor that would page past the window, here one that keeps asking
    /// for the first page, is followed no further than the window allows.
    #[test(tokio::test)]
    async fn query_latest_hashes_refuses_a_cursor_that_keeps_going() {
        let client = GraphQlTestHttpClient {
            latest_pointers: pointers(250),
            search_fault: Some(SearchFault::StuckCursor),
            ..GraphQlTestHttpClient::default()
        };

        let result = latest_hashes(&client).await;

        assert!(
            matches!(&result, Err(Error::Role(RoleError::GraphQl(m))) if m.contains("quilt-bucket")),
            "expected GraphQl error, got {result:?}"
        );
        assert_eq!(client.search_queries_seen.lock().unwrap().len(), 101);
    }

    /// The registry counts no further than its window, so a count that
    /// reaches it may be short of the bucket's packages, and nothing is paged.
    #[test(tokio::test)]
    async fn query_latest_hashes_refuses_a_count_at_the_window() {
        let client = GraphQlTestHttpClient {
            latest_pointers: pointers(10_000),
            ..GraphQlTestHttpClient::default()
        };

        let result = latest_hashes(&client).await;

        assert!(
            matches!(&result, Err(Error::Role(RoleError::GraphQl(m))) if m.contains("quilt-bucket")),
            "expected GraphQl error, got {result:?}"
        );
        assert_eq!(client.search_queries_seen.lock().unwrap().len(), 1);
    }

    /// The search finds nothing in an empty bucket, but also in one the
    /// role cannot read or the registry has not indexed.
    #[test(tokio::test)]
    async fn query_latest_hashes_refuses_a_search_that_finds_nothing() {
        let client = GraphQlTestHttpClient::default();

        let result = latest_hashes(&client).await;

        assert!(
            matches!(&result, Err(Error::Role(RoleError::GraphQl(m))) if m.contains("quilt-bucket")),
            "expected GraphQl error, got {result:?}"
        );
    }

    #[test(tokio::test)]
    async fn query_latest_hashes_refuses_a_failed_search() {
        let client = GraphQlTestHttpClient {
            latest_pointers: pointers(3),
            search_fault: Some(SearchFault::Refused),
            ..GraphQlTestHttpClient::default()
        };

        let result = latest_hashes(&client).await;

        assert!(
            matches!(&result, Err(Error::Role(RoleError::GraphQl(m))) if m.contains("quilt-bucket")),
            "expected GraphQl error, got {result:?}"
        );
    }

    /// A name that is no package, and a `latest` that is no top hash, are
    /// left out, as the bucket's own listing leaves them out.
    #[test(tokio::test)]
    async fn query_latest_hashes_skips_what_names_no_revision() {
        let client = GraphQlTestHttpClient {
            latest_pointers: vec![
                ("loose".to_string(), top_hash(1)),
                ("team/data".to_string(), top_hash(2)),
                ("team/torn".to_string(), "not-a-hash".to_string()),
            ],
            ..GraphQlTestHttpClient::default()
        };

        let result = latest_hashes(&client).await;

        assert_eq!(
            result.ok(),
            Some(vec![(("team", "data").into(), top_hash(2))])
        );
    }
}
