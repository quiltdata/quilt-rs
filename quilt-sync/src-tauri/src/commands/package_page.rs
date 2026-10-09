//! The v2 package page's data.
//!
//! Beside `main_page.rs` and borrowing from it; `package_data.rs` is v1's and is
//! never opened here. See `arch/comp/quilt-sync/node.md#v2-parallel-path` in the
//! spec corpus for why a v2 surface gets its own module rather than a wider v1
//! one.

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::autopull::PausedReason;
use crate::autopull::Watcher;
use crate::commands::RoleCache;
use crate::commands::main_page::PackageStateDto;
use crate::commands::main_page::{conflict_files, session_state, unexplained_pause};
use crate::commands::package_entries::{EntryList, entry_list};
use crate::error::Error;
use crate::model;
use crate::notify::Notify;
use crate::quilt;
use crate::quilt::lineage::UpstreamState;
use crate::telemetry::MixpanelEvent;
use crate::telemetry::event::RemotePackageEvent;
use crate::telemetry::prelude::*;
use crate::toast::ToastCenter;
use crate::toast::ToastDraft;
use crate::toast::ToastKind;

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
    /// The file pane's list, classified by the same status the header's state
    /// comes from.
    pub files: FilesData,
}

/// The file pane's list, or why there is none.
#[derive(Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum FilesData {
    Listed(EntryList),
    /// Not even a local status could be computed, or the manifest's rows
    /// could not be read. No list is sent rather than one classified without
    /// a status: every row would read as pristine or not downloaded, whatever
    /// is on disk.
    Unlisted {
        reason: String,
    },
}

/// The read-only facts shown beside the v2 package page.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageContextData {
    pub revision: CurrentRevisionData,
    /// Raw bucket name. Presentation (`s3://` or the absent-state copy) stays
    /// in the UI rather than crossing the command boundary.
    pub bucket: Option<String>,
    /// How many revisions this copy holds — the trigger's N. A count, not the
    /// list: the list is `get_revision_history`, fetched on open.
    pub revision_count: usize,
    pub keeping: KeepingData,
    /// The current revision's bytes, as Keeping's caption says them. `None`
    /// when they cannot be read — the rows' sizes do not add up in a `u64` —
    /// and the caption then gives the count alone.
    pub size: Option<PackageSize>,
    /// `None` unless the package is diverged, as the status the header shows
    /// says; see `get_package_page_data_from_model`.
    pub resolve: Option<ResolveData>,
}

/// The current revision's size, summed from the sizes its manifest rows
/// record: logical bytes, not disk usage, and no file is stat'ed for it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageSize {
    /// Every listed file.
    pub total: u64,
    /// Every listed file not in the backlog — what `remote_only` leaves out,
    /// so a file deleted here counts: Download would not fetch it.
    pub downloaded: u64,
}

/// Which files this copy keeps, as the pane says it. Its own wire enum rather
/// than `SyncScope`, whose serde form is the lineage file's, not this page's.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KeepingScope {
    IndividualFiles,
    EntirePackage,
}

/// The Keeping section: the standing rule, and what it has not yet fetched.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KeepingData {
    /// The package's stored scope — the one both pull paths apply.
    pub scope: KeepingScope,
    /// Files in the current revision — the caption's M.
    pub total: usize,
    /// Listed by the current revision and not downloaded here, sorted: the
    /// backlog, and exactly what `Download N files` installs.
    pub remote_only: Vec<String>,
    /// Files deleted here: local changes waiting to commit, so not in the
    /// backlog. The caption names them, since they count as downloaded. The
    /// file pane's `counts.deleted`, carried here so the caption does not wait
    /// on the list; 0 when the status could not be read.
    pub deleted_here: usize,
}

/// The resolve mode's facts, or why they could not be read. Present only
/// for a diverged package; see `get_package_page_data_from_model`.
#[derive(Debug, PartialEq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum ResolveData {
    Compared {
        published_message: Option<String>,
        /// Sorted logical keys, the whole package: neither capped nor filtered,
        /// because the pane's count and the file pane's marks read this one value.
        differing: Vec<String>,
        unpublished: usize,
        /// Tracked paths with a local change: the edits a reset overwrites.
        uncommitted: usize,
    },
    /// The manifest fetch, the listing, the status or the session failed.
    Refused { reason: String },
}

/// The current revision's user-facing facts.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CurrentRevisionData {
    /// Top hash of the revision's manifest. Only a deep link that asked for a
    /// different revision reads it: the mismatch band names this side by its
    /// message and puts the hash on hover, falling back to its short form when
    /// the revision has no message.
    pub hash: String,
    pub message: Option<String>,
    pub obtained_at: f64,
}

/// `Revisions you have`: the rows, and what removing every removable one
/// frees, measured for the set.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionHistoryData {
    pub rows: Vec<RevisionHistoryRow>,
    /// The footer's figure. `None` when nothing is removable, or when what a
    /// removal frees could not be read — then no row is offered for removal.
    pub removable_frees: Option<u64>,
}

/// One row of `Revisions you have`, newest obtained first.
#[derive(Debug, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RevisionHistoryRow {
    /// The handle `remove_revisions` takes. Never drawn: a revision is named
    /// by its message and time.
    pub hash: String,
    pub message: Option<String>,
    pub obtained_at: f64,
    pub published: bool,
    /// The catalog page for exactly this revision, or `None`: an unpublished
    /// row has no page to link, and a remote with no catalog host has no
    /// catalog. Built here rather than from the header's `uri`, so the page
    /// never formats an address from the hash.
    pub catalog_url: Option<String>,
    /// Why it cannot be removed, in the order the row says them; empty when
    /// it can.
    pub kept: Vec<KeptReason>,
    /// What removing it alone frees. `None` when it is kept, or when the
    /// measure could not be read.
    pub frees: Option<u64>,
}

/// Why a revision is kept. Mirrors `quilt::flow::Kept`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum KeptReason {
    Current,
    Latest,
    Base,
    NotPushed,
    Unpublished,
}

impl From<quilt::flow::Kept> for KeptReason {
    fn from(kept: quilt::flow::Kept) -> Self {
        match kept {
            quilt::flow::Kept::Current => Self::Current,
            quilt::flow::Kept::Latest => Self::Latest,
            quilt::flow::Kept::Base => Self::Base,
            quilt::flow::Kept::NotPushed => Self::NotPushed,
            quilt::flow::Kept::Unpublished => Self::Unpublished,
        }
    }
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
    /// The remedy a denial offers, and `None` in every other state.
    ///
    /// A sibling of [`Self::state`] rather than a field inside `RoleDenied`: the
    /// state enum is the UI's vocabulary and four surfaces deserialise it, while
    /// a host and a role list are transport one page reads. `None` on a denial
    /// means there is nothing to switch to — a single-role reader, a denial with
    /// no host, or a roles lookup that failed, which are indistinguishable to the
    /// reader and should be: each one is "the app cannot offer you another role".
    pub role_switch: Option<RoleSwitch>,
    /// The remote's `latest` hash this read's status was computed against,
    /// when it read the remote just now; `None` when the read fell back to the
    /// last-known tip, did not ask the remote, or found no `latest`. The page
    /// keys its kept dry-run answer on it, so an answer about an earlier newer
    /// revision is not drawn as this one's.
    pub newer: Option<String>,
}

/// The roles a denied reader could switch to, on the host that denied them.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RoleSwitch {
    pub host: String,
    /// Every role the reader holds on `host` except the refused one. Never empty:
    /// the field is `None` when there is nothing to switch to.
    pub alternatives: Vec<String>,
}

/// The state a failed remote read resolves to, when the failure is itself a
/// state this page can word.
///
/// `None` for anything else, which the caller propagates rather than dressing up
/// as a session failure.
fn blocked_state(err: &Error) -> Option<PackageStateDto> {
    // The session half is the main page's too, so the two pages cannot word one
    // failure two ways.
    if let Some(state) = session_state(err) {
        Some(state)
    } else if err.is_access_denied() {
        // Not a session failure — the credentials vended and the active role
        // cannot read the bucket — but a state all the same, and one the header
        // already has words for. Without this arm the read fails and the page
        // draws nothing at all for a package v1 says `No access` about.
        //
        // `role: None` is the classification only. Naming the role is the
        // caller's second step, and a network one: `role_remedy` fills it.
        Some(PackageStateDto::RoleDenied { role: None })
    } else {
        None
    }
}

/// The remedy a denial offers: the role it was refused under, and the other
/// roles the reader holds on that host.
///
/// Called ONLY once the read has classified a denial — that is the one state
/// where the roles are the remedy, so every other read pays nothing for them
/// (`state.md#switch-role-honest`). Through `RoleCache`, which single-flights
/// per host, as the roster's own denial mark does.
///
/// A failure here is not the page's. The denial was classified before this
/// ran and stands whatever this answers: `(None, None)` leaves the state
/// saying `No access` with no role named and offers no switch, which is
/// exactly what a hostless denial already produced.
async fn role_remedy(
    m: &impl model::QuiltModel,
    roles: &RoleCache,
    err: &Error,
) -> (Option<String>, Option<RoleSwitch>) {
    // A denial always arrives by the S3 route, which carries its host parsed.
    let Some(host) = err.s3_host() else {
        return (None, None);
    };
    let Ok(info) = roles.get(m, host).await else {
        return (None, None);
    };
    let alternatives: Vec<String> = info
        .available
        .iter()
        .filter(|role| **role != info.current)
        .cloned()
        .collect();
    let switch = (!alternatives.is_empty()).then(|| RoleSwitch {
        host: host.to_string(),
        alternatives,
    });
    (Some(info.current), switch)
}

/// `i64` milliseconds into `f64`, because JavaScript has no other number.
#[allow(
    clippy::cast_precision_loss,
    reason = "epoch millis fit f64 exactly for any date this program can see"
)]
fn epoch_millis(at: DateTime<Utc>) -> f64 {
    at.timestamp_millis() as f64
}

/// A listed path is outstanding when this copy neither tracks it nor has a
/// local change at it — v1's `remote` rows (`package_data.rs:195-206`).
/// `changes` is `None` when the status could not be read: then only
/// tracking decides, and the engine still refuses to overwrite a local file.
///
/// The size follows the same rule, over the same rows: downloaded is every
/// listed file's bytes less the backlog's. A file deleted here is a local
/// change, so it counts as downloaded, and `deleted_here` says how many.
fn keeping_data(
    lineage: &quilt::lineage::PackageLineage,
    sizes: &BTreeMap<PathBuf, u64>,
    changes: Option<&quilt::lineage::ChangeSet>,
) -> (KeepingData, Option<PackageSize>) {
    let scope = match lineage.sync_scope {
        quilt::lineage::SyncScope::IndividualFiles => KeepingScope::IndividualFiles,
        quilt::lineage::SyncScope::EntirePackage => KeepingScope::EntirePackage,
    };
    let outstanding: Vec<(&PathBuf, u64)> = sizes
        .iter()
        .filter(|(key, _)| {
            !lineage.paths.contains_key(*key) && changes.is_none_or(|c| !c.contains_key(*key))
        })
        .map(|(key, size)| (key, *size))
        .collect();
    let total = sizes
        .values()
        .try_fold(0u64, |sum, size| sum.checked_add(*size));
    let missing = outstanding
        .iter()
        .try_fold(0u64, |sum, (_, size)| sum.checked_add(*size));
    // `missing <= total` whenever `total` fits, since it sums a subset.
    let size = total.zip(missing).map(|(total, missing)| PackageSize {
        total,
        downloaded: total - missing,
    });
    // Every deletion in the status, as the file pane's `EntryCounts.deleted`
    // counts it, so the two panes give one number.
    let deleted_here = changes.map_or(0, |c| {
        c.values()
            .filter(|change| matches!(change, quilt::lineage::Change::Removed(_)))
            .count()
    });
    let keeping = KeepingData {
        scope,
        total: sizes.len(),
        remote_only: outstanding
            .into_iter()
            .map(|(key, _)| key.display().to_string())
            .collect(),
        deleted_here,
    };
    (keeping, size)
}

/// The gate: the header's status decides, not the snapshot, whose `latest_hash` is stale here.
/// A blocked status on a snapshot that says diverged refuses without comparing.
/// A failed comparison refuses in the pane and never fails the page read.
async fn resolve_for_page(
    m: &impl model::QuiltModel,
    installed: &quilt::InstalledPackage,
    lineage: &quilt::lineage::PackageLineage,
    status_read: Option<&Result<quilt::lineage::InstalledPackageStatus, String>>,
) -> Option<ResolveData> {
    match status_read? {
        Ok(status) if status.upstream_state == UpstreamState::Diverged => Some(
            match m
                .get_installed_package_resolve_comparison(installed, lineage)
                .await
            {
                Ok(comparison) => resolve_data(lineage, comparison, &status.changes),
                Err(err) => ResolveData::Refused {
                    reason: err.to_frontend_string(),
                },
            },
        ),
        Err(reason) if UpstreamState::from(lineage.clone()) == UpstreamState::Diverged => {
            Some(ResolveData::Refused {
                reason: reason.clone(),
            })
        }
        Ok(_) | Err(_) => None,
    }
}

/// The file pane's list from the page's own status: no second walk.
///
/// Which files are here and whether they changed does not depend on the
/// remote, so a blocked or skipped remote status does not hide them. A blocked
/// status stopped before its walk — the refusal came from refreshing the
/// latest hash — and a misconfigured remote never asked, so either way the
/// tree is walked once, here, against the cached manifest (v1 does the same
/// after a denial). Only what needs the remote degrades, and the header says
/// so. The manifest's rows are read only once there is a status.
async fn files_data(
    m: &impl model::QuiltModel,
    installed: &quilt::InstalledPackage,
    namespace: &quilt_uri::Namespace,
    lineage: &quilt::lineage::PackageLineage,
    status_read: Option<&Result<quilt::lineage::InstalledPackageStatus, String>>,
) -> FilesData {
    let local;
    let status = match status_read {
        Some(Ok(status)) => status,
        Some(Err(_)) | None => match m.recompute_local_status(installed, None).await {
            Ok(status) => {
                local = status;
                &local
            }
            Err(err) => {
                return FilesData::Unlisted {
                    reason: err.to_frontend_string(),
                };
            }
        },
    };
    match m.get_installed_package_records(installed).await {
        Ok(records) => FilesData::Listed(entry_list(namespace, status, &lineage.paths, &records)),
        Err(err) => FilesData::Unlisted {
            reason: err.to_frontend_string(),
        },
    }
}

/// Untracked files are not counted: reset's touch set is `lineage.paths`
/// (`flow/reset_to_latest.rs:56`), and a file it does not track stays.
fn resolve_data(
    lineage: &quilt::lineage::PackageLineage,
    comparison: quilt::flow::ResolveComparison,
    changes: &quilt::lineage::ChangeSet,
) -> ResolveData {
    let mut differing: Vec<String> = comparison
        .differing
        .iter()
        .map(|key| key.display().to_string())
        .collect();
    // `PathBuf` orders by component, so `a-b` and `a/b` swap once they are strings.
    differing.sort();
    ResolveData::Compared {
        published_message: comparison.published_message,
        differing,
        unpublished: comparison.unpublished,
        uncommitted: changes
            .keys()
            .filter(|p| lineage.paths.contains_key(*p))
            .count(),
    }
}

fn package_context_data(
    namespace: &quilt_uri::Namespace,
    lineage: &quilt::lineage::PackageLineage,
    revision: Option<quilt::flow::Revision>,
    revision_count: usize,
    (keeping, size): (KeepingData, Option<PackageSize>),
    resolve: Option<ResolveData>,
) -> Result<PackageContextData, Error> {
    let revision = revision.ok_or_else(|| {
        Error::General(format!(
            "Installed package {namespace} has no current revision"
        ))
    })?;

    Ok(PackageContextData {
        revision: CurrentRevisionData {
            hash: revision.hash,
            message: revision.message,
            obtained_at: epoch_millis(revision.obtained),
        },
        bucket: lineage
            .remote_uri
            .as_ref()
            .map(|uri| uri.bucket.clone())
            .filter(|bucket| !bucket.is_empty()),
        revision_count,
        keeping,
        size,
        resolve,
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
    roles: tauri::State<'_, RoleCache>,
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

    get_package_page_data_from_model(&*m, &roles, &namespace, paused.as_ref())
        .await
        .map_err(|e| e.to_frontend_string())
}

async fn get_package_page_data_from_model(
    m: &impl model::QuiltModel,
    roles: &RoleCache,
    namespace: &quilt_uri::Namespace,
    paused: Option<&PausedReason>,
) -> Result<PackagePageData, Error> {
    let installed = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(
            namespace.to_owned(),
        ))
    })?;
    let lineage = m.get_installed_package_lineage(&installed).await?;
    let revision = m
        .get_installed_package_current_revision(&installed, &lineage)
        .await?;
    let revision_count = m.get_installed_package_revision_count(&installed).await?;

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

    let mut role_switch = None;
    // The status, or why a blocked read refused; `None` when it was not asked.
    let mut status_read: Option<Result<quilt::lineage::InstalledPackageStatus, String>> = None;
    let state = if lineage.misconfigured_remote() {
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
                let state = if let Some(files) = conflict_files(paused) {
                    PackageStateDto::PullConflict { files }
                } else {
                    quilt::lineage::PackageState::resolve(
                        status.upstream_state,
                        has_local_commit,
                        has_remote,
                        // The count is measured, not guessed. This page reads one
                        // package, so it has no light phase to be provisional for.
                        Some(status.changes.len()),
                    )
                    .into()
                };
                status_read = Some(Ok(status));
                state
            }
            Err(err) => {
                status_read = Some(Err(err.to_frontend_string()));
                match blocked_state(&err) {
                    // The one state whose remedy is worth a round trip, and the
                    // only place this function goes back to the network.
                    Some(PackageStateDto::RoleDenied { .. }) => {
                        let (role, switch) = role_remedy(m, roles, &err).await;
                        role_switch = switch;
                        PackageStateDto::RoleDenied { role }
                    }
                    Some(state) => state,
                    None => return Err(err),
                }
            }
        }
    };

    // A blocked read counts from tracking alone.
    let status = status_read.as_ref().and_then(|r| r.as_ref().ok());
    let newer = status
        .filter(|s| s.latest_refreshed && !s.latest_hash.is_empty())
        .map(|s| s.latest_hash.clone());
    let keeping = keeping_data(
        &lineage,
        &m.get_installed_package_sizes(&installed, &lineage).await?,
        status.map(|s| &s.changes),
    );
    let resolve = resolve_for_page(m, &installed, &lineage, status_read.as_ref()).await;
    let files = files_data(m, &installed, namespace, &lineage, status_read.as_ref()).await;
    let context = package_context_data(
        namespace,
        &lineage,
        revision,
        revision_count,
        keeping,
        resolve,
    )?;

    Ok(PackagePageData {
        context,
        files,
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
            role_switch,
            newer,
        },
    })
}

/// What removing a set of revisions frees, in bytes.
type Measure<'a> = &'a dyn Fn(&BTreeSet<String>) -> u64;

/// Project the engine's entries onto wire rows, in the engine's order, with
/// each row's protection and what removing it frees. `frees` measures a set;
/// `None` means it could not be read, and then nothing is offered.
fn revision_history_data(
    lineage: &quilt::lineage::PackageLineage,
    entries: Vec<quilt::flow::HistoryEntry>,
    frees: Option<Measure<'_>>,
) -> RevisionHistoryData {
    let kept = quilt::flow::protection(lineage, &entries);
    let removable: BTreeSet<String> = entries
        .iter()
        .map(|entry| entry.revision.hash.clone())
        .filter(|hash| !kept.contains_key(hash))
        .collect();
    let rows = revision_history_rows(lineage, entries)
        .into_iter()
        .map(|row| {
            let reasons = kept.get(&row.hash).cloned().unwrap_or_default();
            RevisionHistoryRow {
                frees: frees
                    .filter(|_| reasons.is_empty())
                    .map(|frees| frees(&BTreeSet::from([row.hash.clone()]))),
                kept: reasons.into_iter().map(KeptReason::from).collect(),
                ..row
            }
        })
        .collect();
    RevisionHistoryData {
        rows,
        removable_frees: frees
            .filter(|_| !removable.is_empty())
            .map(|frees| frees(&removable)),
    }
}

/// Project the engine's entries onto wire rows, in the engine's order, with
/// nothing yet said about removing them.
fn revision_history_rows(
    lineage: &quilt::lineage::PackageLineage,
    entries: Vec<quilt::flow::HistoryEntry>,
) -> Vec<RevisionHistoryRow> {
    entries
        .into_iter()
        .map(|entry| RevisionHistoryRow {
            hash: entry.revision.hash.clone(),
            kept: Vec::new(),
            frees: None,
            catalog_url: lineage
                .remote_uri
                .as_ref()
                .filter(|_| entry.published)
                .and_then(|uri| {
                    quilt_uri::S3PackageUri::from(&quilt_uri::ManifestUri {
                        hash: entry.revision.hash.clone(),
                        ..uri.clone()
                    })
                    .display_for_catalog()
                    .ok()
                })
                .map(|url| url.to_string()),
            message: entry.revision.message,
            obtained_at: epoch_millis(entry.revision.obtained),
            published: entry.published,
        })
        .collect()
}

/// The revisions this copy holds, for the context pane's popover. Lazy: the
/// page read carries only the count (`revision-list-lazy`). Not under the
/// page's mutation lock — it is a read (`page-owns-pane-actions`).
#[tauri::command]
pub async fn get_revision_history(
    m: tauri::State<'_, model::Model>,
    namespace: String,
) -> Result<RevisionHistoryData, String> {
    let namespace: quilt_uri::Namespace = namespace
        .try_into()
        .map_err(|e: quilt_uri::UriError| e.to_string())?;

    get_revision_history_from_model(&*m, &namespace)
        .await
        .map_err(|e| e.to_frontend_string())
}

async fn get_revision_history_from_model(
    m: &impl model::QuiltModel,
    namespace: &quilt_uri::Namespace,
) -> Result<RevisionHistoryData, Error> {
    let installed = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(
            namespace.to_owned(),
        ))
    })?;
    // One snapshot: the rows' publication and their links read the same remote.
    let lineage = m.get_installed_package_lineage(&installed).await?;
    let entries = m
        .get_installed_package_revision_history(&installed, &lineage)
        .await?;
    // A manifest anywhere that cannot be read stops the measure, as it would
    // stop the removal: the list still draws, offering nothing to remove.
    let usage = match m.get_installed_package_revision_usage(&installed).await {
        Ok(usage) => Some(usage),
        Err(err) => {
            warn!("Could not measure what removing revisions of {namespace} frees: {err}");
            None
        }
    };
    let frees = usage
        .as_ref()
        .map(|usage| move |set: &BTreeSet<String>| usage.frees(set));
    Ok(revision_history_data(
        &lineage,
        entries,
        frees
            .as_ref()
            .map(|f| f as &dyn Fn(&BTreeSet<String>) -> u64),
    ))
}

/// Remove old revisions of `namespace` and the objects only they used, then
/// post what it freed to the notification stack. A refusal (busy, protected,
/// gone) is the error, for the page's band; nothing was removed.
///
/// Not under the page's command lock: the package's own lock guards it, and
/// the rest of the page stays usable meanwhile (`page-lock`).
#[tauri::command]
pub async fn remove_revisions(
    m: tauri::State<'_, model::Model>,
    toasts: tauri::State<'_, ToastCenter>,
    namespace: String,
    hashes: Vec<String>,
) -> Result<String, String> {
    remove_revisions_command(&*m, &toasts, namespace, hashes).await
}

async fn remove_revisions_command(
    m: &impl model::QuiltModel,
    toasts: &ToastCenter,
    namespace: String,
    hashes: Vec<String>,
) -> Result<String, String> {
    let notify = Notify::new(format!("Removing old revisions of {namespace}"));
    let result = remove_revisions_from_model(m, &namespace, hashes).await;
    if let Ok(report) = &result {
        toasts
            .post(ToastDraft {
                kind: ToastKind::Success,
                body: report.to_string(),
                ..ToastDraft::default()
            })
            .await;
    }
    let msg_err = |err: &Error| format!("Could not remove old revisions: {}", err.user_facing());
    // The toast has said it; the band says nothing more.
    notify.map(result.map(|_| ()), String::new(), msg_err)
}

async fn remove_revisions_from_model(
    m: &impl model::QuiltModel,
    namespace: &str,
    hashes: Vec<String>,
) -> Result<quilt::flow::RemovalReport, Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    let installed = m
        .get_installed_package(&namespace)
        .await?
        .ok_or_else(|| Error::from(quilt::InstallPackageError::NotInstalled(namespace.clone())))?;
    let hashes: BTreeSet<String> = hashes.into_iter().collect();
    m.package_remove_revisions(&installed, &hashes).await
}

/// Install the backlog the page read listed — Keeping's `Download N files`.
///
/// Namespace-keyed like the rest of this page, and not v1's
/// `package_install_paths`: that opens the file browser after every install
/// (the file, or the package folder), which a backlog catch-up must not do.
///
/// Returns the paths it skipped because the remote no longer holds their
/// bytes (an unversioned bucket, overwritten since). Empty when all installed.
#[tauri::command]
pub async fn package_download_backlog(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    watcher: tauri::State<'_, Watcher>,
    namespace: String,
    paths: Vec<String>,
) -> Result<Vec<String>, String> {
    let namespace: quilt_uri::Namespace = namespace
        .try_into()
        .map_err(|e: quilt_uri::UriError| e.to_string())?;

    let result = download_backlog_from_model(&*m, &watcher, &namespace, &paths).await;
    let skipped: Vec<String> = match &result {
        Ok(skipped) => skipped.iter().map(|p| p.display().to_string()).collect(),
        Err(_) => Vec::new(),
    };
    let msg_ok = if skipped.is_empty() {
        format!("Downloaded {} files", paths.len())
    } else {
        format!(
            "Downloaded {} of {} files; skipped {} no longer on the remote",
            paths.len() - skipped.len(),
            paths.len(),
            skipped.len()
        )
    };
    Notify::new(format!("Downloading the backlog of {namespace}"))
        .on_success(
            &tracing,
            MixpanelEvent::PackageInstalled(RemotePackageEvent::for_uri(None)),
        )
        .map(result, msg_ok, |err| {
            format!("Failed to download files: {}", err.user_facing())
        })?;
    Ok(skipped)
}

async fn download_backlog_from_model(
    m: &impl model::QuiltModel,
    watcher: &Watcher,
    namespace: &quilt_uri::Namespace,
    paths: &[String],
) -> Result<Vec<PathBuf>, Error> {
    let installed = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(
            namespace.to_owned(),
        ))
    })?;
    let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    // Lock before raising the flag: while another writer holds the package,
    // the download has written nothing, so quitting then loses nothing.
    let locked = m.lock_package(&installed).await?;
    // A whole-package catch-up, so it raises the in-flight flag a pull does:
    // quitting mid-download would leave files in place that the lineage never records.
    let _applying = watcher.apply_guard(namespace);
    Ok(m.locked_package_install_paths(&locked, &paths)
        .await?
        .skipped)
}

#[cfg(test)]
mod tests {
    use quilt_uri::fixtures;
    use std::collections::BTreeMap;

    use super::*;
    use crate::commands::RoleCache;
    use crate::commands::package_entries::EntryCounts;
    use crate::commands::test_support::{
        access_denied_error, access_denied_error_on, make_installed_package, make_manifest_uri,
        make_manifest_uri_no_origin,
    };
    use quilt_rs::RoleInfo;

    const NS: &str = "team/dataset";

    /// One installed package whose status call answers `status`.
    fn mock_one_package(
        status: Result<quilt::lineage::InstalledPackageStatus, Error>,
    ) -> crate::model::MockQuiltModel {
        mock_one_package_reading_records(status, None)
    }

    /// [`mock_one_package`], with the package on `origin` instead of the
    /// default host.
    fn mock_one_package_on(
        origin: quilt_uri::Host,
        status: Result<quilt::lineage::InstalledPackageStatus, Error>,
    ) -> crate::model::MockQuiltModel {
        mock_one_package_with(Some(origin), status, None)
    }

    /// The manifest's rows at `paths`, each 7 bytes.
    fn records(paths: &[&str]) -> BTreeMap<PathBuf, quilt::manifest::ManifestRow> {
        paths
            .iter()
            .map(|path| {
                (
                    PathBuf::from(path),
                    quilt::manifest::ManifestRow {
                        logical_key: PathBuf::from(path),
                        size: 7,
                        ..Default::default()
                    },
                )
            })
            .collect()
    }

    /// [`mock_one_package`], asserting how many times the manifest's rows are
    /// read when `reads` is given.
    fn mock_one_package_reading_records(
        status: Result<quilt::lineage::InstalledPackageStatus, Error>,
        reads: Option<usize>,
    ) -> crate::model::MockQuiltModel {
        mock_one_package_with(None, status, reads)
    }

    /// The shared body: `origin` overrides the default host when given.
    fn mock_one_package_with(
        origin: Option<quilt_uri::Host>,
        status: Result<quilt::lineage::InstalledPackageStatus, Error>,
        reads: Option<usize>,
    ) -> crate::model::MockQuiltModel {
        let mut model = crate::model::mocks::create();
        let rows = model
            .expect_get_installed_package_records()
            .returning(|_| Ok(records(&["a.csv", "b.csv", "c.csv"])));
        if let Some(n) = reads {
            rows.times(n);
        }
        // Only a blocked status asks for the local one, and never twice.
        model
            .expect_recompute_local_status()
            .times(..=1)
            .returning(|_, _| Ok(settled()));
        model
            .expect_get_installed_package()
            .returning(|ns| Ok(Some(make_installed_package(ns.clone()))));
        model
            .expect_get_installed_package_lineage()
            .returning(move |pkg| {
                let mut uri = make_manifest_uri(&pkg.namespace.to_string());
                if let Some(origin) = &origin {
                    uri.origin = Some(origin.clone());
                }
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
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
        // `.times(1)`: one count per page read, never a history parse.
        model
            .expect_get_installed_package_revision_count()
            .times(1)
            .returning(|_| Ok(4));
        // `.times(1)`: one manifest read for Keeping per page read, and from
        // the page's own lineage snapshot.
        model
            .expect_get_installed_package_sizes()
            .times(1)
            .returning(|_, lineage| {
                assert_eq!(lineage.current_hash(), Some("abcdef"));
                Ok(sizes(&["a.csv", "b.csv", "c.csv"]))
            });
        // `return_once`, not `returning`: `Error` is not `Clone`. `.times(1)`
        // makes "exactly one status call" an assertion — without it a caller
        // that skipped the call entirely would pass silently.
        model
            .expect_get_installed_package_status()
            .times(1)
            .return_once(move |_, _| status);
        // `.times(0)`: a read that is not diverged fetches nothing new.
        model
            .expect_get_installed_package_resolve_comparison()
            .times(0);
        model
    }

    /// One installed package on `lineage`, whose status call answers `status`.
    /// The comparison is left to each test.
    fn mock_package(
        lineage: &quilt::lineage::PackageLineage,
        status: Result<quilt::lineage::InstalledPackageStatus, Error>,
    ) -> crate::model::MockQuiltModel {
        let mut model = crate::model::mocks::create();
        model
            .expect_get_installed_package()
            .returning(|ns| Ok(Some(make_installed_package(ns.clone()))));
        let snapshot = lineage.clone();
        model
            .expect_get_installed_package_lineage()
            .returning(move |_| Ok(snapshot.clone()));
        model
            .expect_get_installed_package_current_revision()
            .returning(|_, _| Ok(Some(revision("Local commit"))));
        model
            .expect_get_installed_package_revision_count()
            .returning(|_| Ok(1));
        model
            .expect_get_installed_package_sizes()
            .returning(|_, _| Ok(sizes(&["a.csv"])));
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(records(&["a.csv"])));
        model
            .expect_recompute_local_status()
            .times(..=1)
            .returning(|_, _| Ok(settled()));
        model
            .expect_get_installed_package_status()
            .times(1)
            .return_once(move |_, _| status);
        model
    }

    async fn read(model: &crate::model::MockQuiltModel) -> Result<PackagePageData, Error> {
        let ns: quilt_uri::Namespace = NS.try_into().unwrap();
        get_package_page_data_from_model(model, &RoleCache::default(), &ns, None).await
    }

    /// Diverged on disk: the remote at `b`, the registry's latest `l`, and a
    /// commit `c` over `b`.
    fn diverged_snapshot() -> quilt::lineage::PackageLineage {
        let mut uri = make_manifest_uri(NS);
        uri.hash = "b".to_string();
        let mut lineage = quilt::lineage::PackageLineage::from_remote(uri, "l".to_string());
        lineage.base_hash = "b".to_string();
        lineage.commit = Some(quilt::lineage::CommitState {
            timestamp: DateTime::from_timestamp_millis(1_758_500_000_000).unwrap(),
            hash: "c".to_string(),
            prev_hashes: Vec::new(),
        });
        lineage
            .paths
            .insert(PathBuf::from("a.csv"), quilt::lineage::PathState::default());
        lineage
    }

    fn diverged(changes: quilt::lineage::ChangeSet) -> quilt::lineage::InstalledPackageStatus {
        quilt::lineage::InstalledPackageStatus::new(UpstreamState::Diverged, changes)
    }

    #[tokio::test]
    async fn an_ordinary_read_fetches_nothing_new() {
        let page = page(&RoleCache::default(), Ok(settled()), None).await;

        assert_eq!(page.context.resolve, None);
    }

    #[tokio::test]
    async fn a_diverged_read_carries_the_comparison() {
        let lineage = diverged_snapshot();
        let mut model = mock_package(&lineage, Ok(diverged(modified("a.csv"))));
        model
            .expect_get_installed_package_resolve_comparison()
            .times(1)
            .returning(|_, lineage| {
                assert_eq!(lineage.current_hash(), Some("c"));
                Ok(quilt::flow::ResolveComparison {
                    published_message: Some("Sent".to_string()),
                    differing: vec![PathBuf::from("plate/a.csv")],
                    unpublished: 2,
                })
            });

        let page = read(&model).await.unwrap();

        assert_eq!(
            page.context.resolve,
            Some(ResolveData::Compared {
                published_message: Some("Sent".to_string()),
                differing: vec!["plate/a.csv".to_string()],
                unpublished: 2,
                uncommitted: 1,
            })
        );
        assert_eq!(page.header.state, PackageStateDto::Diverged);
    }

    #[tokio::test]
    async fn a_failed_comparison_refuses_and_the_page_still_draws() {
        let lineage = diverged_snapshot();
        let mut model = mock_package(&lineage, Ok(diverged(modified("a.csv"))));
        model
            .expect_get_installed_package_resolve_comparison()
            .times(1)
            .return_once(|_, _| Err(access_denied_error()));

        let page = read(&model)
            .await
            .expect("a comparison never fails the page");

        assert_eq!(
            page.context.resolve,
            Some(ResolveData::Refused {
                reason: access_denied_error().to_frontend_string(),
            })
        );
        assert_eq!(page.header.state, PackageStateDto::Diverged);
    }

    /// Uncommitted is unknowable without the status, so no comparison is asked.
    #[tokio::test]
    async fn a_blocked_status_on_a_diverged_snapshot_refuses_without_comparing() {
        let lineage = diverged_snapshot();
        let mut model = mock_package(&lineage, Err(access_denied_error()));
        model
            .expect_get_installed_package_resolve_comparison()
            .times(0);

        let page = read(&model).await.unwrap();

        assert_eq!(
            page.context.resolve,
            Some(ResolveData::Refused {
                reason: access_denied_error().to_frontend_string(),
            })
        );
        assert_eq!(
            page.header.state,
            PackageStateDto::RoleDenied { role: None }
        );
    }

    #[tokio::test]
    async fn a_blocked_status_on_a_settled_snapshot_carries_nothing() {
        let page = page(&RoleCache::default(), Err(access_denied_error()), None).await;

        assert_eq!(page.context.resolve, None);
    }

    async fn page(
        roles: &RoleCache,
        status: Result<quilt::lineage::InstalledPackageStatus, Error>,
        paused: Option<&PausedReason>,
    ) -> PackagePageData {
        let m = mock_one_package(status);
        let ns: quilt_uri::Namespace = NS.try_into().unwrap();
        get_package_page_data_from_model(&m, roles, &ns, paused)
            .await
            .expect("a blocked read is a state, not an error")
    }

    async fn header_state(
        roles: &RoleCache,
        status: Result<quilt::lineage::InstalledPackageStatus, Error>,
        paused: Option<&PausedReason>,
    ) -> PackageStateDto {
        page(roles, status, paused).await.header.state
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
                hash: "abc123".to_string(),
                message: Some("Initial upload".to_string()),
                obtained_at: 1_758_500_000_000.0,
            },
            bucket: Some("quilt-lab-plates".to_string()),
            revision_count: 4,
            keeping: keeping_fixture(),
            size: Some(size_fixture()),
            resolve: None,
        };

        assert_eq!(
            serde_json::to_string(&context).unwrap(),
            r#"{"revision":{"hash":"abc123","message":"Initial upload","obtainedAt":1758500000000.0},"bucket":"quilt-lab-plates","revisionCount":4,"keeping":{"scope":"entirePackage","total":56,"remoteOnly":["plate/b.csv","plate/c.csv"],"deletedHere":1},"size":{"total":3400000,"downloaded":1900000},"resolve":null}"#,
        );
    }

    /// The listed paths, each 7 bytes, as `records` gives them.
    fn sizes(paths: &[&str]) -> BTreeMap<PathBuf, u64> {
        paths.iter().map(|path| (PathBuf::from(path), 7)).collect()
    }

    fn lineage_tracking(paths: &[&str]) -> quilt::lineage::PackageLineage {
        let mut lineage = quilt::lineage::PackageLineage::from_remote(
            make_manifest_uri(NS),
            "abcdef".to_string(),
        );
        for path in paths {
            lineage
                .paths
                .insert(PathBuf::from(path), quilt::lineage::PathState::default());
        }
        lineage
    }

    fn modified(path: &str) -> quilt::lineage::ChangeSet {
        quilt::lineage::ChangeSet::from([(
            PathBuf::from(path),
            quilt::lineage::Change::Modified(quilt::manifest::ManifestRow::default()),
        )])
    }

    fn comparison(differing: &[&str], unpublished: usize) -> quilt::flow::ResolveComparison {
        quilt::flow::ResolveComparison {
            published_message: Some("Add intake folder-upload note".to_string()),
            differing: differing.iter().map(PathBuf::from).collect(),
            unpublished,
        }
    }

    #[test]
    fn resolve_compared_wire_form_is_verbatim() {
        let data = resolve_data(
            &lineage_tracking(&["a.csv"]),
            comparison(&["plate/a.csv", "plate/b.csv"], 2),
            &modified("a.csv"),
        );

        assert_eq!(
            serde_json::to_string(&data).unwrap(),
            r#"{"kind":"compared","publishedMessage":"Add intake folder-upload note","differing":["plate/a.csv","plate/b.csv"],"unpublished":2,"uncommitted":1}"#,
        );
    }

    #[test]
    fn resolve_refused_wire_form_is_verbatim() {
        let data = ResolveData::Refused {
            reason: "AccessDenied".to_string(),
        };

        assert_eq!(
            serde_json::to_string(&data).unwrap(),
            r#"{"kind":"refused","reason":"AccessDenied"}"#,
        );
    }

    /// A deletion reset restores is an edit it overwrites; a new file it does
    /// not track stays.
    #[test]
    fn only_tracked_paths_with_a_change_are_uncommitted() {
        let row = quilt::manifest::ManifestRow::default;
        let changes = quilt::lineage::ChangeSet::from([
            (
                PathBuf::from("a.csv"),
                quilt::lineage::Change::Modified(row()),
            ),
            (
                PathBuf::from("b.csv"),
                quilt::lineage::Change::Removed(row()),
            ),
            (
                PathBuf::from("new.csv"),
                quilt::lineage::Change::Added(row()),
            ),
        ]);

        let data = resolve_data(
            &lineage_tracking(&["a.csv", "b.csv"]),
            comparison(&[], 0),
            &changes,
        );

        let ResolveData::Compared { uncommitted, .. } = data else {
            panic!("a comparison projects to Compared, got {data:?}");
        };
        assert_eq!(uncommitted, 2);
    }

    /// No cap, per `#resolve-seams`: the pane's count and the file pane's
    /// marks read this one value.
    #[test]
    fn the_differing_set_crosses_whole_and_sorted() {
        let keys: Vec<String> = (0..1_500)
            .rev()
            .map(|i| format!("plate/{i:04}.csv"))
            .collect();
        let refs: Vec<&str> = keys.iter().map(String::as_str).collect();

        let data = resolve_data(
            &lineage_tracking(&[]),
            comparison(&refs, 0),
            &quilt::lineage::ChangeSet::new(),
        );

        let ResolveData::Compared { differing, .. } = data else {
            panic!("a comparison projects to Compared, got {data:?}");
        };
        let mut expected = keys.clone();
        expected.sort();
        assert_eq!(differing.len(), 1_500);
        assert_eq!(differing, expected);
    }

    fn keeping_fixture() -> KeepingData {
        KeepingData {
            scope: KeepingScope::EntirePackage,
            total: 56,
            remote_only: vec!["plate/b.csv".to_string(), "plate/c.csv".to_string()],
            deleted_here: 1,
        }
    }

    fn size_fixture() -> PackageSize {
        PackageSize {
            total: 3_400_000,
            downloaded: 1_900_000,
        }
    }

    #[test]
    fn keeping_wire_form_is_verbatim() {
        assert_eq!(
            serde_json::to_string(&keeping_fixture()).unwrap(),
            r#"{"scope":"entirePackage","total":56,"remoteOnly":["plate/b.csv","plate/c.csv"],"deletedHere":1}"#,
        );
    }

    #[test]
    fn the_backlog_is_what_is_listed_but_neither_tracked_nor_changed_here() {
        let lineage = lineage_tracking(&["a.csv"]);
        let changes = modified("b.csv");

        let (keeping, _) = keeping_data(
            &lineage,
            &sizes(&["a.csv", "b.csv", "c.csv", "d.csv"]),
            Some(&changes),
        );

        assert_eq!(keeping.total, 4);
        assert_eq!(keeping.remote_only, vec!["c.csv", "d.csv"]);
    }

    #[test]
    fn without_a_status_only_tracking_decides() {
        let lineage = lineage_tracking(&["a.csv"]);

        let (keeping, _) = keeping_data(
            &lineage,
            &sizes(&["a.csv", "b.csv", "c.csv", "d.csv"]),
            None,
        );

        assert_eq!(keeping.remote_only, vec!["b.csv", "c.csv", "d.csv"]);
    }

    #[test]
    fn a_tracked_path_the_revision_no_longer_lists_is_not_counted() {
        let lineage = lineage_tracking(&["a.csv", "z.csv"]);
        let listed = sizes(&["a.csv", "b.csv"]);

        let (keeping, _) = keeping_data(&lineage, &listed, None);

        assert_eq!(keeping.total, listed.len());
        assert!(!keeping.remote_only.contains(&"z.csv".to_string()));
    }

    #[test]
    fn the_scope_is_the_one_the_package_stores() {
        let listed = sizes(&["a.csv"]);
        let mut lineage = lineage_tracking(&[]);
        assert_eq!(
            keeping_data(&lineage, &listed, None).0.scope,
            KeepingScope::IndividualFiles
        );

        lineage.sync_scope = quilt::lineage::SyncScope::EntirePackage;
        assert_eq!(
            keeping_data(&lineage, &listed, None).0.scope,
            KeepingScope::EntirePackage
        );
    }

    fn removed(path: &str) -> quilt::lineage::ChangeSet {
        quilt::lineage::ChangeSet::from([(
            PathBuf::from(path),
            quilt::lineage::Change::Removed(quilt::manifest::ManifestRow::default()),
        )])
    }

    fn sized(rows: &[(&str, u64)]) -> BTreeMap<PathBuf, u64> {
        rows.iter()
            .map(|(path, size)| (PathBuf::from(path), *size))
            .collect()
    }

    /// The total is every listed row's size; downloaded leaves out the backlog's.
    #[test]
    fn the_size_is_the_rows_and_the_backlog_is_left_out_of_downloaded() {
        let lineage = lineage_tracking(&["a.csv"]);
        let changes = modified("b.csv");

        let (keeping, size) = keeping_data(
            &lineage,
            &sized(&[
                ("a.csv", 1),
                ("b.csv", 20),
                ("c.csv", 300),
                ("d.csv", 4_000),
            ]),
            Some(&changes),
        );

        assert_eq!(keeping.remote_only, vec!["c.csv", "d.csv"]);
        assert_eq!(
            size,
            Some(PackageSize {
                total: 4_321,
                downloaded: 21,
            })
        );
    }

    /// A file deleted here is a local change waiting to commit, not a file
    /// Download fetches: it counts as downloaded, and Keeping names it.
    #[test]
    fn a_file_deleted_here_counts_as_downloaded_and_is_named() {
        let lineage = lineage_tracking(&["a.csv", "b.csv"]);
        let changes = removed("b.csv");

        let (keeping, size) = keeping_data(
            &lineage,
            &sized(&[("a.csv", 1), ("b.csv", 20), ("c.csv", 300)]),
            Some(&changes),
        );

        assert_eq!(keeping.remote_only, vec!["c.csv"]);
        assert_eq!(keeping.deleted_here, 1);
        assert_eq!(
            size,
            Some(PackageSize {
                total: 321,
                downloaded: 21,
            })
        );
    }

    /// Deleted is the status's own count, as the file pane counts it: a
    /// modification is not one, and without a status none is known.
    #[test]
    fn deleted_here_is_every_removal_the_status_reports() {
        let lineage = lineage_tracking(&["a.csv", "b.csv"]);
        let listed = sized(&[("a.csv", 1), ("b.csv", 1)]);
        let mut changes = removed("a.csv");
        changes.extend(removed("b.csv"));
        changes.extend(modified("c.csv"));

        let (keeping, _) = keeping_data(&lineage, &listed, Some(&changes));
        assert_eq!(keeping.deleted_here, 2);
        let (keeping, _) = keeping_data(&lineage, &listed, Some(&modified("a.csv")));
        assert_eq!(keeping.deleted_here, 0);
        let (keeping, _) = keeping_data(&lineage, &listed, None);
        assert_eq!(keeping.deleted_here, 0);
    }

    #[test]
    fn an_empty_revision_weighs_nothing() {
        let (keeping, size) = keeping_data(&lineage_tracking(&[]), &BTreeMap::new(), None);

        assert_eq!(keeping.total, 0);
        assert_eq!(
            size,
            Some(PackageSize {
                total: 0,
                downloaded: 0,
            })
        );
    }

    /// Sizes that do not add up in a `u64` are not read, rather than wrapped
    /// or saturated into a figure nobody has: the count still crosses.
    #[test]
    fn a_size_past_u64_is_not_read_and_the_count_still_is() {
        let (keeping, size) = keeping_data(
            &lineage_tracking(&[]),
            &sized(&[("a.csv", u64::MAX), ("b.csv", 1)]),
            None,
        );

        assert_eq!(size, None);
        assert_eq!(keeping.total, 2);
        assert_eq!(keeping.remote_only, vec!["a.csv", "b.csv"]);
    }

    #[test]
    fn package_size_wire_form_is_verbatim() {
        assert_eq!(
            serde_json::to_string(&size_fixture()).unwrap(),
            r#"{"total":3400000,"downloaded":1900000}"#,
        );
    }

    #[test]
    fn the_engine_selected_revision_crosses_with_raw_remote_bucket() {
        let namespace: quilt_uri::Namespace = NS.try_into().unwrap();
        let lineage = quilt::lineage::PackageLineage::from_remote(
            make_manifest_uri(NS),
            "remote-hash".to_string(),
        );

        let context = package_context_data(
            &namespace,
            &lineage,
            Some(revision("Pending commit")),
            1,
            (keeping_fixture(), Some(size_fixture())),
            None,
        )
        .unwrap();

        assert_eq!(
            context,
            PackageContextData {
                revision: CurrentRevisionData {
                    hash: "pending-hash".to_string(),
                    message: Some("Pending commit".to_string()),
                    obtained_at: 1_758_500_000_000.0,
                },
                bucket: Some("test".to_string()),
                revision_count: 1,
                keeping: keeping_fixture(),
                size: Some(size_fixture()),
                resolve: None,
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
            1,
            (keeping_fixture(), Some(size_fixture())),
            None,
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

        let context = package_context_data(
            &namespace,
            &lineage,
            Some(revision("Initial upload")),
            1,
            (keeping_fixture(), Some(size_fixture())),
            None,
        )
        .unwrap();

        assert_eq!(context.bucket, None);
    }

    #[test]
    fn an_installed_package_without_a_current_revision_is_a_read_failure() {
        let namespace: quilt_uri::Namespace = NS.try_into().unwrap();
        let err = package_context_data(
            &namespace,
            &quilt::lineage::PackageLineage::default(),
            None,
            1,
            (keeping_fixture(), Some(size_fixture())),
            None,
        )
        .unwrap_err();

        assert_eq!(
            err.to_string(),
            "General error: Installed package team/dataset has no current revision"
        );
    }

    /// The trigger's N arrives with the rest of the page, not with the list.
    #[tokio::test]
    async fn the_page_read_carries_the_revision_count() {
        let data = page(&RoleCache::default(), Ok(settled()), None).await;
        assert_eq!(data.context.revision_count, 4);
    }

    #[tokio::test]
    async fn the_page_read_carries_what_this_copy_keeps() {
        let keeping = page(&RoleCache::default(), Ok(settled()), None)
            .await
            .context
            .keeping;

        assert_eq!(keeping.scope, KeepingScope::IndividualFiles);
        assert_eq!(keeping.total, 3);
        assert_eq!(keeping.remote_only, vec!["a.csv", "b.csv", "c.csv"]);
    }

    #[tokio::test]
    async fn a_path_changed_here_is_not_outstanding() {
        let status =
            quilt::lineage::InstalledPackageStatus::new(UpstreamState::UpToDate, modified("b.csv"));

        let keeping = page(&RoleCache::default(), Ok(status), None)
            .await
            .context
            .keeping;

        assert_eq!(keeping.remote_only, vec!["a.csv", "c.csv"]);
    }

    /// From the rows Keeping already read: three of 7 bytes, none here.
    #[tokio::test]
    async fn the_page_read_carries_the_package_size() {
        let status =
            quilt::lineage::InstalledPackageStatus::new(UpstreamState::UpToDate, removed("b.csv"));

        let context = page(&RoleCache::default(), Ok(status), None).await.context;

        assert_eq!(context.keeping.remote_only, vec!["a.csv", "c.csv"]);
        assert_eq!(context.keeping.deleted_here, 1);
        assert_eq!(
            context.size,
            Some(PackageSize {
                total: 21,
                downloaded: 7,
            })
        );
    }

    /// No role expectation: without a host, `RoleCache::get` is never reached.
    #[tokio::test]
    async fn a_blocked_read_still_counts_from_tracking_alone() {
        let page = page(&RoleCache::default(), Err(access_denied_error()), None).await;

        assert_eq!(
            page.header.state,
            PackageStateDto::RoleDenied { role: None }
        );
        assert_eq!(
            page.context.keeping.remote_only,
            vec!["a.csv", "b.csv", "c.csv"]
        );
    }

    fn listed(files: FilesData) -> EntryList {
        match files {
            FilesData::Listed(list) => list,
            FilesData::Unlisted { reason } => panic!("no list: {reason}"),
        }
    }

    /// The file pane's rows come with the page, classified by the header's own
    /// status: `mock_one_package` expects exactly one status call.
    #[tokio::test]
    async fn the_page_read_carries_the_file_list_from_its_one_status() {
        let status =
            quilt::lineage::InstalledPackageStatus::new(UpstreamState::UpToDate, modified("b.csv"));
        let m = mock_one_package_reading_records(Ok(status), Some(1));
        let list = listed(read(&m).await.expect("a page").files);

        let rows: Vec<(&str, &str)> = list
            .entries
            .iter()
            .map(|e| (e.filename.as_str(), e.status.as_str()))
            .collect();
        assert_eq!(
            rows,
            vec![
                ("a.csv", "remote"),
                ("b.csv", "modified"),
                ("c.csv", "remote")
            ]
        );
        assert_eq!(
            list.counts,
            EntryCounts {
                all: 3,
                changed: 1,
                not_downloaded: 2,
                ignored: 0,
                deleted: 0,
            }
        );
        assert_eq!(list.total, 3);
        assert!(!list.truncated);
    }

    /// One package on `uri` whose remote-aware status answers `status`, whose
    /// local status answers `local`, and whose manifest rows answer `rows`.
    /// `None` asserts the call is never made, and `Some` that it is made once.
    fn mock_rows(
        uri: quilt_uri::ManifestUri,
        status: Option<Result<quilt::lineage::InstalledPackageStatus, Error>>,
        local: Option<Result<quilt::lineage::InstalledPackageStatus, Error>>,
        rows: Option<Result<BTreeMap<PathBuf, quilt::manifest::ManifestRow>, Error>>,
    ) -> crate::model::MockQuiltModel {
        let mut m = crate::model::mocks::create();
        m.expect_get_installed_package()
            .returning(|ns| Ok(Some(make_installed_package(ns.clone()))));
        m.expect_get_installed_package_lineage()
            .returning(move |_| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri.clone(),
                    "abcdef".to_string(),
                ))
            });
        m.expect_get_installed_package_current_revision()
            .returning(|_, _| Ok(Some(revision("Initial upload"))));
        m.expect_get_installed_package_revision_count()
            .returning(|_| Ok(1));
        m.expect_get_installed_package_sizes()
            .returning(|_, _| Ok(sizes(&["a.csv"])));
        match status {
            Some(status) => {
                m.expect_get_installed_package_status()
                    .times(1)
                    .return_once(move |_, _| status);
            }
            None => {
                m.expect_get_installed_package_status().times(0);
            }
        }
        match local {
            Some(local) => {
                m.expect_recompute_local_status()
                    .times(1)
                    .return_once(move |_, _| local);
            }
            None => {
                m.expect_recompute_local_status().times(0);
            }
        }
        match rows {
            Some(rows) => {
                m.expect_get_installed_package_records()
                    .times(1)
                    .return_once(move |_| rows);
            }
            None => {
                m.expect_get_installed_package_records().times(0);
            }
        }
        m
    }

    #[tokio::test]
    async fn the_page_read_caps_the_file_list_and_says_so() {
        let paths: Vec<String> = (0..1001).map(|i| format!("f{i:04}")).collect();
        let many: Vec<&str> = paths.iter().map(String::as_str).collect();
        let m = mock_rows(
            make_manifest_uri(NS),
            Some(Ok(settled())),
            None,
            Some(Ok(records(&many))),
        );

        let list = listed(read(&m).await.expect("a page").files);

        assert_eq!(list.entries.len(), 1000);
        assert_eq!(
            list.entries.last().map(|e| e.filename.as_str()),
            Some("f0999")
        );
        assert_eq!(list.total, 1001);
        assert!(list.truncated);
        assert_eq!(list.counts.not_downloaded, 1001);
    }

    /// A local change, as the local walk sees it.
    fn changed_here(path: &str) -> quilt::lineage::InstalledPackageStatus {
        quilt::lineage::InstalledPackageStatus::new(UpstreamState::UpToDate, modified(path))
    }

    fn rows_of(list: &EntryList) -> Vec<(&str, &str)> {
        list.entries
            .iter()
            .map(|e| (e.filename.as_str(), e.status.as_str()))
            .collect()
    }

    /// A denial refuses the remote, not the tree: the files are still listed,
    /// from one local walk. The remote status stopped before walking.
    #[tokio::test]
    async fn a_denied_package_still_lists_its_local_files() {
        let m = mock_rows(
            make_manifest_uri(NS),
            Some(Err(access_denied_error())),
            Some(Ok(changed_here("b.csv"))),
            Some(Ok(records(&["a.csv", "b.csv"]))),
        );
        let page = read(&m).await.expect("a blocked read is a state");

        assert_eq!(
            page.header.state,
            PackageStateDto::RoleDenied { role: None }
        );
        let list = listed(page.files);
        assert_eq!(
            rows_of(&list),
            vec![("a.csv", "remote"), ("b.csv", "modified")]
        );
    }

    #[tokio::test]
    async fn a_package_with_no_session_still_lists_its_local_files() {
        let no_session = Error::from(quilt::Error::Login(quilt::LoginError::NoSession(None)));
        let m = mock_rows(
            make_manifest_uri(NS),
            Some(Err(no_session)),
            Some(Ok(changed_here("a.csv"))),
            Some(Ok(records(&["a.csv"]))),
        );
        let page = read(&m).await.expect("a blocked read is a state");

        assert!(
            matches!(page.header.state, PackageStateDto::NoSession { .. }),
            "state was {:?}",
            page.header.state
        );
        let list = listed(page.files);
        assert_eq!(rows_of(&list), vec![("a.csv", "modified")]);
    }

    /// A misconfigured remote never asks for the remote status; the local
    /// walk still lists the files.
    #[tokio::test]
    async fn a_misconfigured_remote_still_lists_its_local_files() {
        let m = mock_rows(
            make_manifest_uri_no_origin(NS),
            None,
            Some(Ok(changed_here("a.csv"))),
            Some(Ok(records(&["a.csv"]))),
        );
        let page = read(&m).await.expect("a page");

        assert_eq!(page.header.state, PackageStateDto::Unknown);
        let list = listed(page.files);
        assert_eq!(rows_of(&list), vec![("a.csv", "modified")]);
    }

    /// With no status at all, no list: never one classified without a status,
    /// and the manifest's rows are not read.
    #[tokio::test]
    async fn no_local_status_sends_no_list_and_says_why() {
        let m = mock_rows(
            make_manifest_uri(NS),
            Some(Err(access_denied_error())),
            Some(Err(Error::General("tree unreadable".to_string()))),
            None,
        );
        let page = read(&m).await.expect("a blocked read is a state");

        match page.files {
            FilesData::Unlisted { reason } => {
                assert!(reason.contains("tree unreadable"), "reason was {reason}");
            }
            FilesData::Listed(_) => panic!("a list without a status"),
        }
    }

    /// A manifest that cannot be read leaves the header standing.
    #[tokio::test]
    async fn unreadable_rows_send_no_list_and_the_page_still_draws() {
        let m = mock_rows(
            make_manifest_uri(NS),
            Some(Ok(settled())),
            None,
            Some(Err(Error::General("manifest gone".to_string()))),
        );

        let page = read(&m).await.expect("a page");

        match page.files {
            FilesData::Unlisted { reason } => {
                assert!(reason.contains("manifest gone"), "reason was {reason}");
            }
            FilesData::Listed(_) => panic!("a list from rows that were not read"),
        }
    }

    /// Anchored identically in the UI's `files_data_wire_form_is_verbatim`.
    #[test]
    fn files_data_wire_form_is_verbatim() {
        let unlisted = FilesData::Unlisted {
            reason: "denied".to_string(),
        };
        assert_eq!(
            serde_json::to_string(&unlisted).unwrap(),
            r#"{"kind":"unlisted","reason":"denied"}"#,
        );
        let listed = FilesData::Listed(EntryList {
            entries: Vec::new(),
            counts: EntryCounts::default(),
            total: 0,
            truncated: false,
        });
        assert_eq!(
            serde_json::to_string(&listed).unwrap(),
            r#"{"kind":"listed","entries":[],"counts":{"all":0,"changed":0,"notDownloaded":0,"ignored":0,"deleted":0},"total":0,"truncated":false}"#,
        );
    }

    fn history_entry(
        hash: &str,
        at_millis: i64,
        message: Option<&str>,
        published: bool,
    ) -> quilt::flow::HistoryEntry {
        quilt::flow::HistoryEntry {
            revision: quilt::flow::Revision {
                hash: hash.to_string(),
                obtained: DateTime::from_timestamp_millis(at_millis).unwrap(),
                message: message.map(ToString::to_string),
            },
            published,
        }
    }

    fn remote_lineage(uri: quilt_uri::ManifestUri) -> quilt::lineage::PackageLineage {
        quilt::lineage::PackageLineage::from_remote(uri, "abcdef".to_string())
    }

    const PUBLISHED_URL: &str =
        "https://quilt.test/b/test/packages/team/dataset/tree/published-hash";

    #[test]
    fn revision_history_wire_form_is_verbatim() {
        let data = RevisionHistoryData {
            rows: vec![
                RevisionHistoryRow {
                    hash: "published-hash".to_string(),
                    message: Some("Sent".to_string()),
                    obtained_at: 1_758_500_000_000.0,
                    published: true,
                    catalog_url: Some(PUBLISHED_URL.to_string()),
                    kept: Vec::new(),
                    frees: Some(1_200_000),
                },
                RevisionHistoryRow {
                    hash: "local-hash".to_string(),
                    message: None,
                    obtained_at: 1_758_400_000_000.0,
                    published: false,
                    catalog_url: None,
                    kept: vec![KeptReason::Current, KeptReason::NotPushed],
                    frees: None,
                },
            ],
            removable_frees: Some(1_200_000),
        };

        assert_eq!(
            serde_json::to_string(&data).unwrap(),
            r#"{"rows":[{"hash":"published-hash","message":"Sent","obtainedAt":1758500000000.0,"published":true,"catalogUrl":"https://quilt.test/b/test/packages/team/dataset/tree/published-hash","kept":[],"frees":1200000},{"hash":"local-hash","message":null,"obtainedAt":1758400000000.0,"published":false,"catalogUrl":null,"kept":["current","notPushed"],"frees":null}],"removableFrees":1200000}"#,
        );
    }

    /// The link addresses the row's own revision, not the lineage's current one.
    #[test]
    fn a_published_row_links_to_its_exact_revision() {
        let rows = revision_history_rows(
            &remote_lineage(make_manifest_uri(NS)),
            vec![history_entry(
                "published-hash",
                1_758_500_000_000,
                Some("Sent"),
                true,
            )],
        );

        assert_eq!(
            rows,
            vec![RevisionHistoryRow {
                hash: "published-hash".to_string(),
                message: Some("Sent".to_string()),
                obtained_at: 1_758_500_000_000.0,
                published: true,
                catalog_url: Some(PUBLISHED_URL.to_string()),
                kept: Vec::new(),
                frees: None,
            }]
        );
        assert!(
            !PUBLISHED_URL.contains("abcdef"),
            "the lineage's hash must not address another row's page"
        );
    }

    #[test]
    fn an_unpublished_row_has_no_link() {
        let rows = revision_history_rows(
            &remote_lineage(make_manifest_uri(NS)),
            vec![history_entry(
                "local-hash",
                1_758_400_000_000,
                Some("Draft"),
                false,
            )],
        );

        assert!(!rows[0].published);
        assert_eq!(rows[0].catalog_url, None);
    }

    #[test]
    fn a_remote_without_a_catalog_host_links_nothing() {
        let rows = revision_history_rows(
            &remote_lineage(make_manifest_uri_no_origin(NS)),
            vec![history_entry(
                "published-hash",
                1_758_500_000_000,
                Some("Sent"),
                true,
            )],
        );

        assert!(rows[0].published);
        assert_eq!(rows[0].catalog_url, None);
    }

    /// The engine owns newest-first; the projection must not re-sort.
    #[test]
    fn the_rows_keep_the_engine_order() {
        let rows = revision_history_rows(
            &remote_lineage(make_manifest_uri(NS)),
            vec![
                history_entry("c", 1_758_300_000_000, Some("Third"), false),
                history_entry("a", 1_758_500_000_000, Some("First"), true),
                history_entry("b", 1_758_400_000_000, Some("Second"), false),
            ],
        );

        assert_eq!(
            rows.iter()
                .map(|row| row.message.as_deref())
                .collect::<Vec<_>>(),
            vec![Some("Third"), Some("First"), Some("Second")]
        );
    }

    /// Installed from `mine`, which is current and base; `abcdef` is latest.
    fn removal_lineage() -> quilt::lineage::PackageLineage {
        remote_lineage(quilt_uri::ManifestUri {
            hash: "mine".to_string(),
            ..make_manifest_uri(NS)
        })
    }

    fn removal_entries() -> Vec<quilt::flow::HistoryEntry> {
        vec![
            history_entry("abcdef", 1_758_500_000_000, Some("Latest"), true),
            history_entry("mine", 1_758_400_000_000, Some("Mine"), true),
            history_entry("old-1", 1_758_300_000_000, Some("Old"), true),
            history_entry("old-2", 1_758_200_000_000, Some("Older"), true),
            history_entry("draft", 1_758_100_000_000, Some("Draft"), false),
        ]
    }

    /// A stand-in measure: each old revision frees 10 alone, and the two
    /// together free 25, since they share objects nothing else uses.
    fn measure(set: &BTreeSet<String>) -> u64 {
        match set.len() {
            1 => 10,
            2 => 25,
            _ => 0,
        }
    }

    #[test]
    fn a_kept_row_says_why_and_frees_nothing_and_the_footer_measures_the_set() {
        let data = revision_history_data(&removal_lineage(), removal_entries(), Some(&measure));

        let said: Vec<(&str, &[KeptReason], Option<u64>)> = data
            .rows
            .iter()
            .map(|row| (row.message.as_deref().unwrap(), &row.kept[..], row.frees))
            .collect();
        assert_eq!(
            said,
            vec![
                ("Latest", &[KeptReason::Latest][..], None),
                ("Mine", &[KeptReason::Current, KeptReason::Base][..], None),
                ("Old", &[][..], Some(10)),
                ("Older", &[][..], Some(10)),
                ("Draft", &[KeptReason::Unpublished][..], None),
            ]
        );
        assert_eq!(
            data.removable_frees,
            Some(25),
            "measured as a set, not summed"
        );
    }

    /// Without a measure the list still draws, and offers nothing.
    #[test]
    fn without_a_measure_nothing_is_offered() {
        let data = revision_history_data(&removal_lineage(), removal_entries(), None);

        assert!(data.rows.iter().all(|row| row.frees.is_none()));
        assert_eq!(data.removable_frees, None);
        assert_eq!(
            data.rows[2].kept,
            Vec::new(),
            "still removable, just unmeasured"
        );
    }

    #[test]
    fn nothing_removable_has_no_footer_figure() {
        let data = revision_history_data(
            &removal_lineage(),
            removal_entries().into_iter().take(2).collect(),
            Some(&measure),
        );

        assert_eq!(data.removable_frees, None);
    }

    struct Collector(std::sync::Arc<std::sync::Mutex<Vec<crate::toast::Toast>>>);

    impl crate::toast::ToastEmitter for Collector {
        fn emit(&self, toast: &crate::toast::Toast) {
            self.0.lock().unwrap().push(toast.clone());
        }
    }

    fn report() -> quilt::flow::RemovalReport {
        quilt::flow::RemovalReport {
            namespace: NS.try_into().unwrap(),
            revisions: 4,
            objects: 6,
            bytes: 6_900_000,
            kept_for: None,
            kept_bytes: 0,
        }
    }

    /// The result is the stack's to say: a Success toast with the report's
    /// own sentence, and nothing for the band.
    #[tokio::test]
    async fn a_removal_posts_its_report_as_a_success_toast() {
        let mut m = crate::model::mocks::create();
        m.expect_get_installed_package()
            .returning(|ns| Ok(Some(make_installed_package(ns.clone()))));
        m.expect_package_remove_revisions()
            .withf(|_, hashes| hashes == &BTreeSet::from(["old-1".to_string()]))
            .returning(|_, _| Ok(report()));
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let toasts = ToastCenter::new(Box::new(Collector(std::sync::Arc::clone(&seen))));

        let said =
            remove_revisions_command(&m, &toasts, NS.to_string(), vec!["old-1".to_string()]).await;

        assert_eq!(said, Ok(String::new()));
        let seen = seen.lock().unwrap();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].kind, ToastKind::Success);
        assert_eq!(
            seen[0].body,
            "Removed 4 old revisions of team/dataset \u{b7} freed 6.9 MB"
        );
    }

    /// A refusal goes to the band, and the stack says nothing.
    #[tokio::test]
    async fn a_busy_package_is_the_bands_and_posts_nothing() {
        let mut m = crate::model::mocks::create();
        m.expect_get_installed_package()
            .returning(|ns| Ok(Some(make_installed_package(ns.clone()))));
        m.expect_package_remove_revisions()
            .returning(|installed, _| {
                Err(quilt::Error::PackageBusy(installed.namespace.clone()).into())
            });
        let seen = std::sync::Arc::new(std::sync::Mutex::new(Vec::new()));
        let toasts = ToastCenter::new(Box::new(Collector(std::sync::Arc::clone(&seen))));

        let said =
            remove_revisions_command(&m, &toasts, NS.to_string(), vec!["old-1".to_string()]).await;

        let err = said.expect_err("refused");
        assert!(err.contains("team/dataset is busy"), "{err}");
        assert!(seen.lock().unwrap().is_empty());
    }

    #[tokio::test]
    async fn a_missing_package_is_an_error_not_an_empty_history() {
        let mut m = crate::model::mocks::create();
        m.expect_get_installed_package().returning(|_| Ok(None));
        let ns: quilt_uri::Namespace = NS.try_into().unwrap();

        let err = get_revision_history_from_model(&m, &ns)
            .await
            .expect_err("an absent package has no history to be empty");

        assert!(err.to_string().contains("team/dataset"), "{err}");
    }

    fn test_watcher() -> Watcher {
        Watcher::new_for_test(std::sync::Arc::new(crate::autopull::reporter::LogReporter))
    }

    /// The download writes working files like a pull, so it holds the flag
    /// that makes quitting ask first, and only while it writes: not while it
    /// takes the package's lock.
    #[tokio::test]
    async fn the_download_holds_the_apply_flag_while_it_writes() {
        let watcher = test_watcher();
        let aggregator = watcher.inner_for_test().aggregator.clone();
        let mut m = crate::model::mocks::create();
        m.expect_get_installed_package()
            .returning(|ns| Ok(Some(make_installed_package(ns.clone()))));
        let locking = aggregator.clone();
        m.expect_lock_package().times(1).returning(move |p| {
            assert!(!locking.apply_in_progress(), "down while taking the lock");
            Ok(p.namespace.clone())
        });
        m.expect_locked_package_install_paths()
            .times(1)
            .returning(move |_, _| {
                assert!(aggregator.apply_in_progress(), "held while installing");
                Ok(quilt::flow::InstallPathsReport::default())
            });
        let ns: quilt_uri::Namespace = NS.try_into().unwrap();

        download_backlog_from_model(&m, &watcher, &ns, &["plate/b.csv".into()])
            .await
            .expect("the download succeeds");

        assert!(
            !watcher.inner_for_test().aggregator.apply_in_progress(),
            "and dropped once it returns"
        );
    }

    /// While another writer holds the package, the download waits having
    /// written nothing, so it must not report an apply: quitting then loses
    /// nothing and must not ask.
    #[tokio::test]
    async fn a_download_waiting_for_the_package_lock_reports_nothing_applying() {
        use crate::model::QuiltModel;
        let dir = tempfile::TempDir::new().expect("temp dir");
        let m = crate::model::Model::create(dir.path());
        m.set_home(dir.path().join("home")).await.expect("home");
        let ns: quilt_uri::Namespace = NS.try_into().unwrap();
        let package = m
            .package_create(ns.clone(), None, None)
            .await
            .expect("create");
        let watcher = test_watcher();

        // Another writer of the package, the `quilt` CLI say, holds its lock.
        let other_writer = package.lock().await.expect("lock");
        let paths = ["plate/b.csv".to_string()];
        let mut download = std::pin::pin!(download_backlog_from_model(&m, &watcher, &ns, &paths));
        let early =
            tokio::time::timeout(std::time::Duration::from_millis(200), &mut download).await;
        assert!(early.is_err(), "the download waits for the other writer");
        assert!(
            !watcher.inner_for_test().aggregator.apply_in_progress(),
            "nothing is applying while the download waits for the lock"
        );

        drop(other_writer);
        let _ = tokio::time::timeout(std::time::Duration::from_secs(5), download)
            .await
            .expect("the download runs once the other writer lets go");
    }

    /// A download mock that asserts it installs `expected` and never opens
    /// the file browser, as v1's install path does after every install.
    fn mock_download(expected: &'static [&'static str]) -> crate::model::MockQuiltModel {
        let mut m = crate::model::mocks::create();
        m.expect_get_installed_package()
            .returning(|ns| Ok(Some(make_installed_package(ns.clone()))));
        m.expect_lock_package()
            .returning(|p| Ok(p.namespace.clone()));
        m.expect_locked_package_install_paths()
            .times(1)
            .returning(move |_, paths| {
                assert_eq!(
                    paths,
                    expected.iter().map(PathBuf::from).collect::<Vec<_>>()
                );
                Ok(quilt::flow::InstallPathsReport::default())
            });
        m.expect_reveal_in_file_browser().times(0);
        m.expect_open_in_file_browser().times(0);
        m
    }

    #[tokio::test]
    async fn the_download_installs_exactly_the_paths_it_was_given() {
        let m = mock_download(&["plate/b.csv", "plate/c.csv"]);
        let ns: quilt_uri::Namespace = NS.try_into().unwrap();

        download_backlog_from_model(
            &m,
            &test_watcher(),
            &ns,
            &["plate/b.csv".into(), "plate/c.csv".into()],
        )
        .await
        .expect("the listed paths install");
    }

    #[tokio::test]
    async fn a_single_file_download_opens_nothing() {
        let m = mock_download(&["plate/b.csv"]);
        let ns: quilt_uri::Namespace = NS.try_into().unwrap();

        download_backlog_from_model(&m, &test_watcher(), &ns, &["plate/b.csv".into()])
            .await
            .expect("the one path installs");
    }

    /// On an unversioned bucket a file whose object a later revision replaced
    /// is skipped, not an error: the download returns it for the page to show.
    #[tokio::test]
    async fn a_download_returns_the_files_it_skipped() {
        let mut m = crate::model::mocks::create();
        m.expect_get_installed_package()
            .returning(|ns| Ok(Some(make_installed_package(ns.clone()))));
        m.expect_lock_package()
            .returning(|p| Ok(p.namespace.clone()));
        m.expect_locked_package_install_paths().returning(|_, _| {
            Ok(quilt::flow::InstallPathsReport {
                skipped: vec![PathBuf::from("plate/c.csv")],
                ..Default::default()
            })
        });
        let ns: quilt_uri::Namespace = NS.try_into().unwrap();

        let skipped = download_backlog_from_model(
            &m,
            &test_watcher(),
            &ns,
            &["plate/b.csv".into(), "plate/c.csv".into()],
        )
        .await
        .expect("a partial download is not an error");

        assert_eq!(skipped, vec![PathBuf::from("plate/c.csv")]);
    }

    #[tokio::test]
    async fn a_refused_download_is_an_error() {
        let mut m = crate::model::mocks::create();
        m.expect_get_installed_package()
            .returning(|ns| Ok(Some(make_installed_package(ns.clone()))));
        m.expect_lock_package()
            .returning(|p| Ok(p.namespace.clone()));
        m.expect_locked_package_install_paths()
            .returning(|_, _| Err(access_denied_error()));
        let ns: quilt_uri::Namespace = NS.try_into().unwrap();

        let err = download_backlog_from_model(&m, &test_watcher(), &ns, &["plate/b.csv".into()])
            .await
            .expect_err("a refusal reaches the caller");

        assert!(err.is_access_denied(), "{err}");
    }

    #[tokio::test]
    async fn a_missing_package_cannot_download() {
        let mut m = crate::model::mocks::create();
        m.expect_get_installed_package().returning(|_| Ok(None));
        let ns: quilt_uri::Namespace = NS.try_into().unwrap();

        let err = download_backlog_from_model(&m, &test_watcher(), &ns, &["plate/b.csv".into()])
            .await
            .expect_err("an absent package has nothing to download into");

        assert!(err.to_string().contains("team/dataset"), "{err}");
    }

    /// A pause outranks what the tree says, because it is WHY the tree is not
    /// being acted on. Without the arm this page measures straight past the
    /// pause and reports `Latest` over a package that stopped syncing — and
    /// `PullConflict` has no other source, so the header could never show it.
    #[tokio::test]
    async fn a_pull_conflict_pause_outranks_the_settled_tree() {
        let paused = PausedReason::PullConflict(vec!["a.csv".to_string(), "b.csv".to_string()]);
        assert_eq!(
            header_state(&RoleCache::default(), Ok(settled()), Some(&paused)).await,
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
        let page = page(&RoleCache::default(), Ok(settled()), Some(&paused)).await;

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
        let page = page(&RoleCache::default(), Ok(settled()), Some(&paused)).await;

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
        assert_eq!(
            page(&RoleCache::default(), Ok(settled()), None)
                .await
                .sync_paused,
            None
        );
    }

    /// Rank 1 beats rank 2. The denial is caught on the `Err` side, so it wins
    /// without the pause arms ever running — the same ordering the main page's
    /// heavy phase has.
    #[tokio::test]
    async fn a_denial_outranks_a_pause() {
        let paused = PausedReason::PullConflict(vec!["a.csv".to_string()]);
        assert_eq!(
            header_state(
                &RoleCache::default(),
                Err(access_denied_error()),
                Some(&paused)
            )
            .await,
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
        let header = page(&RoleCache::default(), Ok(settled()), None)
            .await
            .header;

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

    /// A behind status whose newer revision is `latest`, read against the
    /// remote just now or not.
    fn behind_on(latest: &str, refreshed: bool) -> quilt::lineage::InstalledPackageStatus {
        quilt::lineage::InstalledPackageStatus {
            latest_hash: latest.to_string(),
            latest_refreshed: refreshed,
            ..quilt::lineage::InstalledPackageStatus::new(
                UpstreamState::Behind,
                quilt::lineage::ChangeSet::new(),
            )
        }
    }

    /// The read names the newer revision its status was computed against, so
    /// the page can tell an answer about an earlier newer revision from one
    /// about this one. Only a tip read just now counts: a fallen-back one is
    /// what the last read already said, and names nothing new.
    #[tokio::test]
    async fn the_read_names_the_newer_revision_only_when_it_reached_the_remote() {
        let roles = RoleCache::default();
        let fresh = page(&roles, Ok(behind_on("bbbb", true)), None).await;
        assert_eq!(fresh.header.newer.as_deref(), Some("bbbb"));

        let stale = page(&roles, Ok(behind_on("bbbb", false)), None).await;
        assert_eq!(stale.header.newer, None, "the remote was not reached");

        let none = page(&roles, Ok(behind_on("", true)), None).await;
        assert_eq!(
            none.header.newer, None,
            "no `latest` tag, no newer revision"
        );
    }

    /// No pause, so the tree answers for itself.
    #[tokio::test]
    async fn an_unpaused_package_resolves_from_its_status() {
        assert_eq!(
            header_state(&RoleCache::default(), Ok(settled()), None).await,
            PackageStateDto::Latest,
        );
    }

    /// A model whose role query answers `info`. `RoleCache::get` routes through
    /// `observe_role`, which calls `refresh_roles` then
    /// `clear_remote_client_cache` — both need an expectation or mockall panics
    /// on the unmatched one. Same shape as `main_page.rs::mock_with_role`.
    fn with_role(model: &mut crate::model::MockQuiltModel, info: RoleInfo) {
        model
            .expect_refresh_roles()
            .times(1)
            .returning(move |_| Ok(info.clone()));
        model.expect_clear_remote_client_cache().returning(|_| ());
    }

    /// The whole bargain of `switch-role-honest`: the roles are fetched because
    /// the read has already classified a denial, so the state names the refused
    /// role and the payload carries what to switch to.
    #[tokio::test]
    async fn a_denial_names_its_role_and_carries_the_alternatives() {
        let mut m = mock_one_package_on(
            fixtures::one_host(),
            Err(access_denied_error_on(fixtures::another_host())),
        );
        with_role(
            &mut m,
            RoleInfo {
                current: "analyst".to_string(),
                available: vec!["analyst".to_string(), "admin".to_string()],
            },
        );
        let roles = RoleCache::default();
        let ns: quilt_uri::Namespace = NS.try_into().unwrap();
        let header = get_package_page_data_from_model(&m, &roles, &ns, None)
            .await
            .expect("a denial is a state, not an error")
            .header;

        assert_eq!(
            header.state,
            PackageStateDto::RoleDenied {
                role: Some("analyst".to_string()),
            },
        );
        let switch = header.role_switch.expect("another role is held");
        assert_eq!(
            switch.host, "another.quilt.test",
            "the switch is offered on the host that refused, not the package's origin"
        );
        assert_eq!(
            switch.alternatives,
            vec!["admin".to_string()],
            "the refused role is not something to switch to"
        );
    }

    /// A single-role reader sees the reason and no button —
    /// `access-marking`'s rule for the roster, applied here.
    #[tokio::test]
    async fn a_single_role_reader_is_offered_no_switch() {
        let mut m = mock_one_package_on(
            fixtures::one_host(),
            Err(access_denied_error_on(fixtures::another_host())),
        );
        with_role(
            &mut m,
            RoleInfo {
                current: "analyst".to_string(),
                available: vec!["analyst".to_string()],
            },
        );
        let roles = RoleCache::default();
        let ns: quilt_uri::Namespace = NS.try_into().unwrap();
        let header = get_package_page_data_from_model(&m, &roles, &ns, None)
            .await
            .expect("a denial is a state")
            .header;

        assert_eq!(
            header.state,
            PackageStateDto::RoleDenied {
                role: Some("analyst".to_string()),
            },
            "the role is still named — the lookup succeeded"
        );
        assert!(header.role_switch.is_none());
    }

    /// The rider. The lookup is a REMEDY read over a denial already classified,
    /// so its failure is not the page's: the denial stands, naming what it
    /// already knows, and offers no switch.
    #[tokio::test]
    async fn a_failed_roles_lookup_leaves_the_denial_standing() {
        let mut m = mock_one_package_on(
            fixtures::one_host(),
            Err(access_denied_error_on(fixtures::another_host())),
        );
        m.expect_refresh_roles()
            .times(1)
            .returning(|_| Err(Error::General("role query unavailable".to_string())));
        let roles = RoleCache::default();
        let ns: quilt_uri::Namespace = NS.try_into().unwrap();
        let header = get_package_page_data_from_model(&m, &roles, &ns, None)
            .await
            .expect("a failed remedy read is never a page error")
            .header;

        assert_eq!(
            header.state,
            PackageStateDto::RoleDenied { role: None },
            "the denial stands; it simply cannot be named"
        );
        assert!(header.role_switch.is_none());
    }

    /// Every other state pays nothing. `.times(0)` is the assertion — mockall's
    /// default range is satisfied at zero calls, so the expectation's presence
    /// proves nothing on its own.
    #[tokio::test]
    async fn a_read_that_did_not_deny_never_asks_for_roles() {
        let mut m = mock_one_package(Ok(settled()));
        m.expect_refresh_roles().times(0);
        let roles = RoleCache::default();
        let ns: quilt_uri::Namespace = NS.try_into().unwrap();
        let page = get_package_page_data_from_model(&m, &roles, &ns, None)
            .await
            .expect("a settled read");

        assert_eq!(page.header.state, PackageStateDto::Latest);
        assert!(page.header.role_switch.is_none());
    }

    /// Narrowest arm first, or it is unreachable: `is_session_absent` is
    /// `is_invalid_credentials` OR `LoginError::NoSession`, so a general arm
    /// written above the specific one swallows it — silently, since both compile.
    #[test]
    fn a_rejected_credential_is_not_reported_as_a_missing_session() {
        let err = quilt::Error::S3(quilt::S3Error {
            host: Some(fixtures::host()),
            kind: quilt::S3ErrorKind::InvalidCredentials("rejected".to_string()),
        });

        assert_eq!(
            blocked_state(&Error::Quilt(err)),
            Some(PackageStateDto::SignInExpired {
                host: Some("quilt.test".to_string()),
            }),
        );
    }

    #[test]
    fn an_absent_session_names_the_deployment_it_is_absent_for() {
        let err = quilt::Error::Login(quilt::LoginError::NoSession(Some(fixtures::host())));

        assert_eq!(
            blocked_state(&Error::Quilt(err)),
            Some(PackageStateDto::NoSession {
                host: Some("quilt.test".to_string()),
            }),
        );
    }

    /// The header and the main page's rows share one session classifier, so a
    /// failure the header calls signed out is signed out on the main page too.
    #[test]
    fn the_header_words_a_session_failure_as_the_main_page_does() {
        let failures = [
            quilt::Error::Login(quilt::LoginError::NoSession(Some(fixtures::host()))),
            quilt::Error::Login(quilt::LoginError::NoSession(None)),
            quilt::Error::S3(quilt::S3Error {
                host: Some(fixtures::host()),
                kind: quilt::S3ErrorKind::InvalidCredentials("rejected".to_string()),
            }),
        ];
        for err in failures {
            let err = Error::Quilt(err);
            assert!(blocked_state(&err).is_some());
            assert_eq!(blocked_state(&err), session_state(&err));
        }
    }

    /// A denial is not a session failure, and must not be worded as one: the
    /// credentials vended, so signing in again re-vends the same denied role.
    /// It still resolves to a state rather than propagating, or the page draws
    /// nothing for a package v1 says `No access` about.
    #[test]
    fn a_refused_role_is_a_denial_and_not_a_session_failure() {
        let err = quilt::Error::S3(quilt::S3Error {
            host: Some(fixtures::host()),
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
