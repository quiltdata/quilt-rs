//! The v2 package page's data.
//!
//! Beside `main_page.rs` and borrowing from it; `package_data.rs` is v1's and is
//! never opened here. See `arch/comp/quilt-sync/node.md#v2-parallel-path` in the
//! spec corpus for why a v2 surface gets its own module rather than a wider v1
//! one.

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::autopull::PausedReason;
use crate::autopull::Watcher;
use crate::commands::main_page::PackageStateDto;
use crate::commands::main_page::{
    conflict_files, misconfigured_remote, resolve_state, unexplained_pause,
};
use crate::error::Error;
use crate::model;
use crate::quilt;

/// Everything the v2 package page draws, for one package.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackagePageData {
    pub header: PackageHeaderData,
    pub context: PackageContextData,
    /// Why autosync stopped for this package, when the reason is one no state
    /// covers — the **residue**, which is `PausedReason::Other`: a workflow
    /// rejection, a hash mismatch, remote config drift.
    ///
    /// `None` for every other pause, because those resolve into `header.state`
    /// and the header says them: a conflict names its files, a denial names the
    /// refusal, pending work names the work. The residue is the one fact the
    /// header has no word for, and it is carried beside the state rather than
    /// displacing it — the package still has a real upstream state while the
    /// worker is stopped, and a header that said `Sync paused` over a package
    /// with a newer revision upstream would hide what the package needs.
    ///
    /// Prose, and the one field here that is. The vocabulary stays UI-owned —
    /// the surface writes the sentence and renders this as its detail — but the
    /// detail is the engine's own refusal text and nothing else knows it.
    pub sync_paused: Option<String>,
}

/// The read-only facts shown beside the v2 package page.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageContextData {
    pub revision: CurrentRevisionData,
    /// Raw bucket name. Presentation (`s3://` or the absent-state copy) stays
    /// in the UI rather than crossing the command boundary.
    pub bucket: Option<String>,
}

/// The current revision's user-facing facts.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentRevisionData {
    pub message: Option<String>,
    pub obtained_at: f64,
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
    /// Whether there is a local commit. Not derivable from `state`: a diverged
    /// package may hold one, and only the settled arm of the resolver consults
    /// it.
    ///
    /// **Not "undo is available" on its own.** Undo is scoped by the pending
    /// commit chain, and this is one of the three facts that bound it — see
    /// [`Self::commit_has_parent`] for the other two and how they compose.
    pub has_local_commit: bool,
    /// Whether the pending commit has a revision behind it.
    ///
    /// Undo's **floor**: the initial commit from `create` has no parent, so
    /// there is nothing to step back to and the engine refuses. `uninstall` is
    /// the different act.
    ///
    /// # The three facts that bound undo
    ///
    /// Undo is scoped by the pending commit chain — the only on-disk record of
    /// a revision's parent, which a push consumes. A surface offering it must
    /// compose all three, and every one of them is already here:
    ///
    /// - [`Self::has_local_commit`] — there is a pending commit at all;
    /// - this — it is not the floor;
    /// - [`Self::uri`] is `None` — the shipped guard, and **narrower than the
    ///   scope rule**: a remote-backed package with unpushed commits still has
    ///   an intact chain, but the engine refuses on *any* remote, because
    ///   undoing there leaves a pending commit equal to the package's own base.
    ///
    /// Composed on the surface rather than answered here, because the reasons a
    /// command is unavailable are words and words live in the UI. A dirty tree
    /// is a fourth refusal, but it belongs to execution and not to this gate:
    /// it is transient, and the engine states it rather than hiding the command.
    pub commit_has_parent: bool,
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

/// `i64` milliseconds into `f64`, because JavaScript has no other number.
#[allow(
    clippy::cast_precision_loss,
    reason = "epoch millis fit f64 exactly for any date this program can see"
)]
fn epoch_millis(at: DateTime<Utc>) -> f64 {
    at.timestamp_millis() as f64
}

fn package_context_data(
    namespace: &quilt_uri::Namespace,
    lineage: &quilt::lineage::PackageLineage,
    revision: Option<quilt::flow::Revision>,
) -> Result<PackageContextData, Error> {
    let revision = revision.ok_or_else(|| {
        Error::General(format!(
            "Installed package {namespace} has no current revision"
        ))
    })?;

    Ok(PackageContextData {
        revision: CurrentRevisionData {
            message: revision.message,
            obtained_at: epoch_millis(revision.obtained),
        },
        bucket: lineage
            .remote_uri
            .as_ref()
            .map(|uri| uri.bucket.clone())
            .filter(|bucket| !bucket.is_empty()),
    })
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
    watcher: tauri::State<'_, Watcher>,
    namespace: String,
) -> Result<PackagePageData, String> {
    let namespace: quilt_uri::Namespace = namespace
        .try_into()
        .map_err(|e: quilt_uri::UriError| e.to_string())?;

    // One lookup, at the moment of the read. The watcher's map is the only
    // source for a pause: nothing about the working tree or the upstream hash
    // says a package stopped syncing.
    let paused = watcher.paused_reason(&namespace).await;

    get_package_page_data_from_model(&*m, &namespace, paused.as_ref())
        .await
        .map_err(|e| e.to_frontend_string())
}

async fn get_package_page_data_from_model(
    m: &impl model::QuiltModel,
    namespace: &quilt_uri::Namespace,
    paused: Option<&PausedReason>,
) -> Result<PackagePageData, Error> {
    let installed = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(
            namespace.to_owned(),
        ))
    })?;
    let lineage = m.get_installed_package_lineage(&installed).await?;
    let context = package_context_data(
        namespace,
        &lineage,
        m.get_installed_package_current_revision(&installed, &lineage)
            .await?,
    )?;

    let has_local_commit = lineage.commit.is_some();
    let commit_has_parent = lineage
        .commit
        .as_ref()
        .is_some_and(|commit| !commit.prev_hashes.is_empty());
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
            // One pause arm, not the main page's two. A conflict names a
            // condition of the package's FILES and carries the action that
            // clears it, so it belongs in the header's precedence — and the
            // watcher's map is its only source, so without this arm the header
            // could never show it. The residue names a condition of the
            // WORKER, and displacing the package's real state with it would
            // hide what the package needs; it travels as `sync_paused` and the
            // page states it beside the header instead.
            //
            // Below the denial the `Err` side catches: a denial is rank 1.
            Ok(status) => {
                if let Some(files) = conflict_files(paused) {
                    PackageStateDto::PullConflict { files }
                } else {
                    resolve_state(
                        status.upstream_state,
                        has_local_commit,
                        has_remote,
                        // The count is measured, not guessed. This page reads one
                        // package, so it has no light phase to be provisional for.
                        Some(status.changes.len()),
                    )
                }
            }
            Err(err) => match blocked_state(&err) {
                Some(state) => state,
                None => return Err(err),
            },
        }
    };

    Ok(PackagePageData {
        context,
        // The residue only. `unexplained_pause` is `Other` and nothing else, so
        // a pause with a state of its own is reported once, by the header.
        sync_paused: unexplained_pause(paused)
            .then(|| match paused {
                Some(PausedReason::Other(message)) => Some(message.clone()),
                _ => None,
            })
            .flatten(),
        header: PackageHeaderData {
            namespace: namespace.to_owned(),
            uri,
            state,
            remote_locked,
            has_local_commit,
            commit_has_parent,
        },
    })
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;
    use crate::commands::test_support::{
        access_denied_error, make_installed_package, make_manifest_uri,
    };
    use crate::quilt::lineage::UpstreamState;

    const NS: &str = "team/dataset";

    /// One installed package whose status call answers `status`.
    fn mock_one_package(
        status: Result<quilt::lineage::InstalledPackageStatus, Error>,
    ) -> crate::model::MockQuiltModel {
        let mut model = crate::model::mocks::create();
        model
            .expect_get_installed_package()
            .returning(|ns| Ok(Some(make_installed_package(ns.clone()))));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    make_manifest_uri(&pkg.namespace.to_string()),
                    "abcdef".to_string(),
                ))
            });
        model
            .expect_get_installed_package_current_revision()
            .times(1)
            .returning(|_, lineage| {
                assert_eq!(lineage.current_hash(), Some("abcdef"));
                assert_eq!(
                    lineage.remote_uri.as_ref().map(|uri| uri.bucket.as_str()),
                    Some("test")
                );
                Ok(Some(quilt::flow::Revision {
                    hash: "abcdef".to_string(),
                    obtained: DateTime::from_timestamp_millis(1_758_500_000_000).unwrap(),
                    message: Some("Initial upload".to_string()),
                }))
            });
        // `return_once`, not `returning`: `Error` is not `Clone`. `.times(1)`
        // makes "exactly one status call" an assertion — without it a caller
        // that skipped the call entirely would pass silently.
        model
            .expect_get_installed_package_status()
            .times(1)
            .return_once(move |_, _| status);
        model
    }

    async fn page(
        status: Result<quilt::lineage::InstalledPackageStatus, Error>,
        paused: Option<&PausedReason>,
    ) -> PackagePageData {
        let m = mock_one_package(status);
        let ns: quilt_uri::Namespace = NS.try_into().unwrap();
        get_package_page_data_from_model(&m, &ns, paused)
            .await
            .expect("a blocked read is a state, not an error")
    }

    async fn header_state(
        status: Result<quilt::lineage::InstalledPackageStatus, Error>,
        paused: Option<&PausedReason>,
    ) -> PackageStateDto {
        page(status, paused).await.header.state
    }

    fn settled() -> quilt::lineage::InstalledPackageStatus {
        quilt::lineage::InstalledPackageStatus::new(
            UpstreamState::UpToDate,
            quilt::lineage::ChangeSet::new(),
        )
    }

    fn revision(message: &str) -> quilt::flow::Revision {
        quilt::flow::Revision {
            hash: "pending-hash".to_string(),
            obtained: DateTime::from_timestamp_millis(1_758_500_000_000).unwrap(),
            message: Some(message.to_string()),
        }
    }

    #[test]
    fn current_revision_context_wire_form_is_verbatim() {
        let context = PackageContextData {
            revision: CurrentRevisionData {
                message: Some("Initial upload".to_string()),
                obtained_at: 1_758_500_000_000.0,
            },
            bucket: Some("quilt-lab-plates".to_string()),
        };

        assert_eq!(
            serde_json::to_string(&context).unwrap(),
            r#"{"revision":{"message":"Initial upload","obtainedAt":1758500000000.0},"bucket":"quilt-lab-plates"}"#,
        );
    }

    #[test]
    fn the_engine_selected_revision_crosses_with_raw_remote_bucket() {
        let namespace: quilt_uri::Namespace = NS.try_into().unwrap();
        let lineage = quilt::lineage::PackageLineage::from_remote(
            make_manifest_uri(NS),
            "remote-hash".to_string(),
        );

        let context =
            package_context_data(&namespace, &lineage, Some(revision("Pending commit"))).unwrap();

        assert_eq!(
            context,
            PackageContextData {
                revision: CurrentRevisionData {
                    message: Some("Pending commit".to_string()),
                    obtained_at: 1_758_500_000_000.0,
                },
                bucket: Some("test".to_string()),
            }
        );
    }

    #[test]
    fn a_local_package_has_no_bucket() {
        let namespace: quilt_uri::Namespace = NS.try_into().unwrap();
        let context = package_context_data(
            &namespace,
            &quilt::lineage::PackageLineage::default(),
            Some(revision("Local commit")),
        )
        .unwrap();

        assert_eq!(context.bucket, None);
    }

    #[test]
    fn an_empty_configured_bucket_is_absent() {
        let namespace: quilt_uri::Namespace = NS.try_into().unwrap();
        let mut uri = make_manifest_uri(NS);
        uri.bucket.clear();
        let lineage = quilt::lineage::PackageLineage::from_remote(uri, "remote-hash".to_string());

        let context =
            package_context_data(&namespace, &lineage, Some(revision("Initial upload"))).unwrap();

        assert_eq!(context.bucket, None);
    }

    #[test]
    fn an_installed_package_without_a_current_revision_is_a_read_failure() {
        let namespace: quilt_uri::Namespace = NS.try_into().unwrap();
        let err =
            package_context_data(&namespace, &quilt::lineage::PackageLineage::default(), None)
                .unwrap_err();

        assert_eq!(
            err.to_string(),
            "General error: Installed package team/dataset has no current revision"
        );
    }

    /// A pause outranks what the tree says, because it is WHY the tree is not
    /// being acted on. Without the arm this page measures straight past the
    /// pause and reports `Latest` over a package that stopped syncing — and
    /// `PullConflict` has no other source, so the header could never show it.
    #[tokio::test]
    async fn a_pull_conflict_pause_outranks_the_settled_tree() {
        let paused = PausedReason::PullConflict(vec!["a.csv".to_string(), "b.csv".to_string()]);
        assert_eq!(
            header_state(Ok(settled()), Some(&paused)).await,
            PackageStateDto::PullConflict {
                files: vec!["a.csv".to_string(), "b.csv".to_string()],
            },
        );
    }

    /// The residue does NOT displace the package's own state, and that is the
    /// ruling rather than an omission: a package `Latest` by hash and stopped by
    /// a workflow rejection needs both facts, and a header reading `Sync paused`
    /// would hide the one the package is actually in. It travels beside the
    /// state and the page states it in a banner.
    #[tokio::test]
    async fn the_residue_travels_beside_the_state_rather_than_over_it() {
        let paused = PausedReason::Other("workflow rejected the revision".to_string());
        let page = page(Ok(settled()), Some(&paused)).await;

        assert_eq!(page.header.state, PackageStateDto::Latest);
        assert_eq!(
            page.sync_paused.as_deref(),
            Some("workflow rejected the revision"),
        );
    }

    /// A pause with a state of its own is reported once, by the header. Both
    /// halves: the conflict wins the state, and it does not also fill the
    /// banner, which would say the same thing twice on one screen.
    #[tokio::test]
    async fn a_pause_the_header_can_word_does_not_also_fill_the_banner() {
        let paused = PausedReason::PullConflict(vec!["a.csv".to_string()]);
        let page = page(Ok(settled()), Some(&paused)).await;

        assert_eq!(
            page.header.state,
            PackageStateDto::PullConflict {
                files: vec!["a.csv".to_string()],
            },
        );
        assert_eq!(page.sync_paused, None);
    }

    #[tokio::test]
    async fn an_unpaused_package_has_no_banner() {
        assert_eq!(page(Ok(settled()), None).await.sync_paused, None);
    }

    /// Rank 1 beats rank 2. The denial is caught on the `Err` side, so it wins
    /// without the pause arms ever running — the same ordering the main page's
    /// heavy phase has.
    #[tokio::test]
    async fn a_denial_outranks_a_pause() {
        let paused = PausedReason::PullConflict(vec!["a.csv".to_string()]);
        assert_eq!(
            header_state(Err(access_denied_error()), Some(&paused)).await,
            PackageStateDto::RoleDenied { role: None },
        );
    }

    /// The three facts that bound undo, and the two the payload was missing.
    ///
    /// `mock_one_package`'s lineage is `from_remote` with no commit, which is
    /// the shape that exposed the bug: `has_local_commit` alone said undo was
    /// available for a remote-backed package, and the engine refuses on any
    /// remote. Asserted together because composing them is the point — each
    /// alone is true of packages undo would reject.
    #[tokio::test]
    async fn the_payload_carries_every_fact_that_bounds_undo() {
        let header = page(Ok(settled()), None).await.header;

        assert!(
            !header.has_local_commit,
            "this fixture has no pending commit"
        );
        assert!(
            !header.commit_has_parent,
            "and so nothing behind one either"
        );
        assert!(
            header.uri.is_some(),
            "but it DOES have a remote, which is the guard `has_local_commit` \
             could not express — a surface reading that field alone would offer \
             undo where the engine refuses"
        );
    }

    /// No pause, so the tree answers for itself.
    #[tokio::test]
    async fn an_unpaused_package_resolves_from_its_status() {
        assert_eq!(
            header_state(Ok(settled()), None).await,
            PackageStateDto::Latest,
        );
    }

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
