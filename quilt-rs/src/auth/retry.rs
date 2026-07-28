//! Classification of auth-endpoint errors for the single-retry policy.

use tracing::info;
use tracing::warn;

use crate::Error;
use crate::Res;
use crate::error::LoginError;
use crate::error::RoleError;
use quilt_uri::Host;

/// Shared shape of every predicate below: a wire-level failure that carried
/// an HTTP response whose status the caller recognizes as an auth refusal.
fn has_status(e: &Error, accept: fn(u16) -> bool) -> bool {
    matches!(
        e,
        Error::Reqwest(re) if re.status().is_some_and(|s| accept(s.as_u16()))
    )
}

/// Returns true when an error from the Connect **token endpoint** means the
/// user must log in again.
///
/// Includes HTTP 400 because RFC 6749 §5.2 specifies that a revoked or
/// expired refresh token produces `400 invalid_grant`, not 401.
pub(super) fn is_token_auth_error(e: &Error) -> bool {
    has_status(e, |s| matches!(s, 400 | 401 | 403))
}

/// Returns true when an error from the registry **credentials endpoint** means
/// the user must log in again.
///
/// Only 401/403 — a 400 from the registry means a malformed request (client
/// bug), not an auth failure, so it should propagate rather than prompt login.
pub(super) fn is_credentials_auth_error(e: &Error) -> bool {
    has_status(e, |s| matches!(s, 401 | 403))
}

/// Returns true when an error from the registry **GraphQL endpoint** (the
/// role surface: `me`, `switchRole`, `buckets`) means the access token is no
/// longer accepted.
///
/// Same 401/403 shape as the credentials endpoint and for the same reason —
/// a 400 there is a malformed document, i.e. a client bug — but kept as its
/// own predicate because it classifies a different endpoint, and the two are
/// free to diverge as the registry's GraphQL error mapping evolves.
///
/// [`RoleError::NotAuthenticated`] counts too: GraphQL reports a refused
/// session in the body, as `200 {"data":{"me":null}}`, not as a status code.
/// It says exactly what a 401 says — this token no longer identifies a
/// user — so it must reach the same force-refresh-then-login path instead of
/// being handed back as a permanent failure nothing routes anywhere.
pub(super) fn is_role_auth_error(e: &Error) -> bool {
    matches!(e, Error::Role(RoleError::NotAuthenticated(_)))
        || has_status(e, |s| matches!(s, 401 | 403))
}

/// Extracts the HTTP status code from an `Error::Reqwest`, if the wire-level
/// error carried a response (network-level errors without a response return
/// `None`). Used to include the status as a structured field in retry logs.
pub(super) fn http_status(e: &Error) -> Option<u16> {
    match e {
        Error::Reqwest(re) => re.status().map(|s| s.as_u16()),
        _ => None,
    }
}

/// Classifies the outcome of a retry attempt against an auth endpoint.
///
/// - `Ok(_)` → transient error recovered, log at `info!`.
/// - `Err(e)` classified as auth → retry didn't help, upgrade to `LoginRequired`.
/// - `Err(e)` otherwise → propagate as-is (includes nested `LoginRequired`
///   from missing OAuth client state, IO errors, etc.).
pub(super) fn classify_retry_outcome<T>(
    result: Res<T>,
    is_auth_error: fn(&Error) -> bool,
    endpoint: &str,
    host: &Host,
) -> Res<T> {
    match result {
        Ok(v) => {
            info!(
                "✔️ Recovered from transient auth error on {} for {}",
                endpoint, host
            );
            Ok(v)
        }
        Err(e) if is_auth_error(&e) => {
            warn!(
                status = ?http_status(&e),
                "❌ Auth error on {} for {} persisted after retry, login required: {}",
                endpoint, host, e
            );
            Err(LoginError::Required(Some(host.to_owned())).into())
        }
        Err(e) => {
            warn!(
                status = ?http_status(&e),
                "❌ Failed to refresh via {} for {} on retry: {}",
                endpoint, host, e
            );
            Err(e)
        }
    }
}
