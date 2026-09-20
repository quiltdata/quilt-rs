//! The v2 package page's data.
//!
//! Beside `main_page.rs` and borrowing from it; `package_data.rs` is v1's and is
//! never opened here. See `arch/comp/quilt-sync/node.md#v2-parallel-path` in the
//! spec corpus for why a v2 surface gets its own module rather than a wider v1
//! one.

use serde::Serialize;

use crate::commands::main_page::PackageStateDto;
use crate::commands::main_page::{misconfigured_remote, resolve_state};
use crate::error::Error;
use crate::model;
use crate::quilt;

/// Everything the v2 package page draws, for one package.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackagePageData {
    pub header: PackageHeaderData,
}

/// The header region: identity, one resolved condition, and what the overflow
/// menu may offer.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageHeaderData {
    pub namespace: quilt_uri::Namespace,
    /// Carried whole rather than as a catalog URL: the menu builds an
    /// entry-level link from the same value, and the frontend already owns that
    /// formatting.
    pub uri: Option<quilt_uri::S3PackageUri>,
    pub state: PackageStateDto,
    /// Whether the remote is pinned by a push — decides whether the menu offers
    /// to change the bucket or only to show it.
    pub remote_locked: bool,
    /// Whether there is a local commit to undo. Not derivable from `state`: a
    /// diverged package may hold one, and only the settled arm of the resolver
    /// consults it.
    pub has_local_commit: bool,
}

/// The state a failed remote read resolves to, when the failure is itself a
/// state this page can word.
///
/// `None` for anything else, which the caller propagates rather than dressing up
/// as a session failure.
fn blocked_state(err: &Error) -> Option<PackageStateDto> {
    // Narrowest first: `is_session_absent` is `is_invalid_credentials` OR
    // `LoginError::NoSession`, so a general arm above the specific one makes the
    // specific one unreachable.
    if err.is_invalid_credentials() {
        Some(PackageStateDto::SignInExpired {
            host: session_host(err),
        })
    } else if err.is_session_absent() {
        Some(PackageStateDto::NoSession {
            host: session_host(err),
        })
    } else if err.is_access_denied() {
        // Not a session failure — the credentials vended and the active role
        // cannot read the bucket — but a state all the same, and one the header
        // already has words for. Without this arm the read fails and the page
        // draws nothing at all for a package v1 says `No access` about.
        //
        // `role: None`: naming the role costs a `RoleCache` round trip per load,
        // which is the main page's bargain and not obviously this page's. The
        // kit words an unnamed denial `No access` and offers nothing, which is
        // what the deferred `denial-action` unit says the first draw should do.
        Some(PackageStateDto::RoleDenied { role: None })
    } else {
        None
    }
}

/// The deployment a session failure was for, by either route.
///
/// [`Error::s3_host`] answers for the S3 route only — a rejected credential
/// carries its host on the `S3Error`. The login route carries it on
/// [`quilt::LoginError::NoSession`] instead, and this surface names the
/// deployment whichever route the failure took.
fn session_host(err: &Error) -> Option<String> {
    match err {
        Error::Quilt(quilt::Error::Login(quilt::LoginError::NoSession(host))) => {
            host.as_ref().map(ToString::to_string)
        }
        _ => err.s3_host().map(ToString::to_string),
    }
}

/// The v2 package page's one read.
///
/// One command for the page rather than one per region: the header's state and
/// the file pane's entries fall out of a single status computation, so two
/// commands would ask that question twice and could be told two different
/// answers. One phase, not the main page's light-then-heavy pair — that split
/// exists because the main page walks every package, and this reads one.
#[tauri::command]
pub async fn get_package_page_data(
    m: tauri::State<'_, model::Model>,
    namespace: String,
) -> Result<PackagePageData, String> {
    let namespace: quilt_uri::Namespace = namespace
        .try_into()
        .map_err(|e: quilt_uri::UriError| e.to_string())?;

    get_package_page_data_from_model(&*m, &namespace)
        .await
        .map_err(|e| e.to_frontend_string())
}

async fn get_package_page_data_from_model(
    m: &impl model::QuiltModel,
    namespace: &quilt_uri::Namespace,
) -> Result<PackagePageData, Error> {
    let installed = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(
            namespace.to_owned(),
        ))
    })?;
    let lineage = m.get_installed_package_lineage(&installed).await?;

    let has_local_commit = lineage.commit.is_some();
    let has_remote = lineage.remote_uri.is_some();
    let remote_locked = lineage
        .remote_uri
        .as_ref()
        .is_some_and(|uri| !uri.hash.is_empty());
    let uri = lineage
        .remote_uri
        .as_ref()
        .map(quilt_uri::S3PackageUri::from);

    let state = if misconfigured_remote(&lineage) {
        // The same predicate both main-page phases apply before resolving, for
        // the same reason: without a catalog there is nowhere to vend
        // credentials from, so the status call cannot succeed. A third surface
        // answering this shape differently is the disagreement that predicate
        // exists to prevent.
        PackageStateDto::Unknown
    } else {
        // A blocked read is a state, not an error: the page still draws, and the
        // header says what is wrong and what to do about it.
        match m.get_installed_package_status(&installed, None).await {
            Ok(status) => resolve_state(
                status.upstream_state,
                has_local_commit,
                has_remote,
                // The count is measured, not guessed. This page reads one
                // package, so it has no light phase to be provisional for.
                Some(status.changes.len()),
            ),
            Err(err) => match blocked_state(&err) {
                Some(state) => state,
                None => return Err(err),
            },
        }
    };

    Ok(PackagePageData {
        header: PackageHeaderData {
            namespace: namespace.to_owned(),
            uri,
            state,
            remote_locked,
            has_local_commit,
        },
    })
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    fn host() -> quilt_uri::Host {
        quilt_uri::Host::from_str("demo.quiltdata.com").unwrap()
    }

    /// Narrowest arm first, or it is unreachable: `is_session_absent` is
    /// `is_invalid_credentials` OR `LoginError::NoSession`, so a general arm
    /// written above the specific one swallows it — silently, since both compile.
    #[test]
    fn a_rejected_credential_is_not_reported_as_a_missing_session() {
        let err = quilt::Error::S3(quilt::S3Error {
            host: Some(host()),
            kind: quilt::S3ErrorKind::InvalidCredentials("rejected".to_string()),
        });

        assert_eq!(
            blocked_state(&Error::Quilt(err)),
            Some(PackageStateDto::SignInExpired {
                host: Some("demo.quiltdata.com".to_string()),
            }),
        );
    }

    #[test]
    fn an_absent_session_names_the_deployment_it_is_absent_for() {
        let err = quilt::Error::Login(quilt::LoginError::NoSession(Some(host())));

        assert_eq!(
            blocked_state(&Error::Quilt(err)),
            Some(PackageStateDto::NoSession {
                host: Some("demo.quiltdata.com".to_string()),
            }),
        );
    }

    /// A denial is not a session failure, and must not be worded as one: the
    /// credentials vended, so signing in again re-vends the same denied role.
    /// It still resolves to a state rather than propagating, or the page draws
    /// nothing for a package v1 says `No access` about.
    #[test]
    fn a_refused_role_is_a_denial_and_not_a_session_failure() {
        let err = quilt::Error::S3(quilt::S3Error {
            host: Some(host()),
            kind: quilt::S3ErrorKind::AccessDenied("denied".to_string()),
        });

        assert_eq!(
            blocked_state(&Error::Quilt(err)),
            Some(PackageStateDto::RoleDenied { role: None }),
        );
    }

    /// Not every failure is a blocked one. An error this function does not
    /// recognise must fall through, or the header would claim a session problem
    /// for a failure that had nothing to do with sessions.
    #[test]
    fn an_unrelated_failure_is_not_a_session_state() {
        let err = quilt::Error::S3(quilt::S3Error {
            host: None,
            kind: quilt::S3ErrorKind::ListObjects("timed out".to_string()),
        });
        assert_eq!(blocked_state(&Error::Quilt(err)), None);
    }
}
