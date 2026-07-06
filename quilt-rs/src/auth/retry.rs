//! Classification of auth-endpoint errors for the single-retry policy.

use tracing::info;
use tracing::warn;

use crate::Error;
use crate::Res;
use crate::error::LoginError;
use quilt_uri::Host;

/// Returns true when an error from the Connect **token endpoint** means the
/// user must log in again.
///
/// Includes HTTP 400 because RFC 6749 §5.2 specifies that a revoked or
/// expired refresh token produces `400 invalid_grant`, not 401.
pub(super) fn is_token_auth_error(e: &Error) -> bool {
    matches!(
        e,
        Error::Reqwest(re) if re.status().is_some_and(|s| s == 400 || s == 401 || s == 403)
    )
}

/// Returns true when an error from the registry **credentials endpoint** means
/// the user must log in again.
///
/// Only 401/403 — a 400 from the registry means a malformed request (client
/// bug), not an auth failure, so it should propagate rather than prompt login.
pub(super) fn is_credentials_auth_error(e: &Error) -> bool {
    matches!(
        e,
        Error::Reqwest(re) if re.status().is_some_and(|s| s == 401 || s == 403)
    )
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
