//! Hand-rolled GraphQL calls against the registry's `/graphql` endpoint.
//!
//! The role surface lives here — `me`, `switchRole`, and the role-scoped
//! `buckets` list — plus `package`'s revision list. None of these has a REST
//! equivalent, which is the only reason quilt-rs speaks GraphQL at all;
//! everything else in the auth layer stays on the REST endpoints. Deliberately a few typed
//! documents over the existing `HttpClient` rather than a generated client.

use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::Res;
use crate::error::RoleError;
use crate::io::remote::client::HttpClient;
use quilt_uri::Host;
use quilt_uri::Namespace;

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

#[derive(Deserialize)]
struct RevisionHash {
    hash: String,
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
/// pushed) lists nothing.
pub(super) async fn query_package_revisions(
    http_client: &impl HttpClient,
    registry: &url::Host,
    bucket: &str,
    namespace: &Namespace,
    access_token: &str,
) -> Res<Vec<String>> {
    let mut hashes = Vec::new();
    for number in 1.. {
        let variables = PackageRevisionsVariables {
            bucket,
            name: namespace.to_string(),
            number,
            per_page: REVISIONS_PER_PAGE,
        };
        let data: PackageRevisionsData = execute(
            http_client,
            registry,
            PACKAGE_REVISIONS_QUERY,
            variables,
            access_token,
        )
        .await?;
        let Some(package) = data.package else {
            break;
        };
        let RevisionPage { total, page } = package.revisions;
        // A page drops pointers that vanished before resolving, so a short or
        // even empty page is not the end: only `total` says whether more follow.
        hashes.extend(page.into_iter().map(|r| r.hash));
        if u64::from(number) * u64::from(REVISIONS_PER_PAGE) >= u64::from(total) {
            break;
        }
    }
    Ok(hashes)
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::Error;
    use crate::auth::test_utils::ACCESS_TOKEN;
    use crate::auth::test_utils::GraphQlTestHttpClient;
    use crate::auth::test_utils::get_host;
    use crate::auth::test_utils::get_registry_host;

    #[test(tokio::test)]
    async fn query_me_returns_active_and_available_roles() -> Res {
        let client = GraphQlTestHttpClient::default();
        let me = query_me(&client, &get_registry_host(), &get_host(), ACCESS_TOKEN).await?;

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
        let err = query_me(&client, &get_registry_host(), &get_host(), ACCESS_TOKEN)
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
        let err = query_me(&client, &get_registry_host(), &get_host(), ACCESS_TOKEN)
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

        assert!(buckets.is_empty());
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

        assert!(listed.is_empty());
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
}
