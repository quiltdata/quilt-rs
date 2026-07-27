//! Hand-rolled GraphQL calls against the registry's `/graphql` endpoint.
//!
//! Only the role surface lives here — `me`, `switchRole`, and the
//! role-scoped `buckets` list. These three have no REST equivalent, which
//! is the only reason quilt-rs speaks GraphQL at all; everything else in
//! the auth layer stays on the REST endpoints. Deliberately a few typed
//! documents over the existing `HttpClient` rather than a generated client.

// The documents below are exercised by this module's tests but have no
// non-test caller yet — the `Auth` methods that drive role switching arrive
// with the public role API. Drop this attribute once they do.
#![allow(dead_code)]

use serde::Deserialize;
use serde::Serialize;
use serde::de::DeserializeOwned;

use crate::Res;
use crate::error::RoleError;
use crate::io::remote::client::HttpClient;
use quilt_uri::Host;

const ME_QUERY: &str = "query { me { role { name } roles { name } } }";

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
}
