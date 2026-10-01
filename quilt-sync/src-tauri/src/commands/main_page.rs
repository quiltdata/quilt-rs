//! v2's main-page payloads. Nothing here is shared with `package_list.rs`, which
//! is v1's, frozen, and deleted once v2 ships.

use std::collections::HashMap;
use std::collections::HashSet;
use std::str::FromStr;
use std::time::Duration;

use chrono::DateTime;
use chrono::Utc;
use serde::Serialize;
use tauri::Manager;
use tokio::sync;
use tokio::time::Instant;
use tokio::time::timeout_at;

use quilt_rs::RoleInfo;
use quilt_uri::Host;

use crate::autopull::PausedReason;
use crate::autopull::Watcher;
use crate::autopull::WatcherFacts;
use crate::commands::RoleCache;
use crate::error::Error;
use crate::model;
use crate::quilt;
use crate::quilt::lineage::PackageState;

/// A package's resolved state, as the UI's `kit::PackageState` expects it.
///
/// §2: a discriminator, never prose. The UI owns the words; a rewording must not
/// need a backend release.
///
/// Two variants come from outside [`PackageState::resolve`]: `PullConflict` from the
/// watcher's paused map (see [`conflict_files`]), `RoleDenied` from the access
/// pass below (`AccessMark::state`).
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PackageStateDto {
    Latest,
    Behind,
    PendingChanges {
        files: usize,
    },
    PendingCommit,
    Diverged,
    PullConflict {
        files: Vec<String>,
    },
    /// `None` when the denial is certain but the role query behind the wording
    /// failed. The denial still stands — the bucket refused — so suppressing the
    /// state would lose a real fact; it simply cannot be named.
    RoleDenied {
        role: Option<String>,
    },
    NoRemote,
    Unpublished,
    /// There is no session for this package's deployment — never signed in, or
    /// signed out.
    ///
    /// `host` is optional because a bare bucket reached on ambient AWS
    /// credentials has no deployment to sign in to; the same reason
    /// [`quilt::LoginError::NoSession`] carries an optional one. A `None` host
    /// means the remedy is the credentials file rather than a sign-in, and the
    /// surface has to be able to say so.
    NoSession {
        host: Option<String>,
    },
    /// A session that existed and was refused.
    ///
    /// Separate from [`Self::NoSession`] although `quilt::Error::is_session_absent`
    /// merges them: that predicate answers "can this caller proceed", which is one
    /// bit, and a surface may say more than the bit. Being told you were signed out
    /// when you never signed in is a different sentence from being told your
    /// sign-in lapsed underneath you.
    SignInExpired {
        host: Option<String>,
    },
    /// Autosync stopped for this package for a reason no other state covers.
    /// The pause reasons that have a state resolve into it instead:
    /// `PendingChanges`, `PendingCommit` and `Diverged` into the state the working
    /// tree reports, and `PullConflict` and `RoleDenied` into their own states.
    /// Only `PausedReason::Other` resolves here (see `unexplained_pause`).
    ///
    /// Kept separate from `Unknown`, which makes a different and stronger claim:
    /// that the upstream state could not be read at all.
    Paused,
    /// `UpstreamState::Error`. The UI's `PackageState` catches this with
    /// `#[serde(other)]`, the same arm that catches a kind added after this build.
    Unknown,
}

/// Why the watcher stopped syncing one package, as sent to the UI.
///
/// v2's own type. v1's `reporter::PausedEvent` is frozen: its `reason` is a plain
/// string, and its single `message` means something different for each reason
/// (the raw refusal for `other`, comma-joined file names for `pullConflict`, the
/// role name for `roleDenied`). A comma-joined list cannot be split back apart
/// when a file name contains a comma. Here each meaning has its own field.
///
/// Kinds are `snake_case`, matching [`PackageStateDto`] rather than v1's
/// camelCase reason strings. The queue reads a pause and a state side by side,
/// so those two vocabularies are the pair that must agree.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PausedDto {
    PendingChanges,
    PendingCommit,
    Diverged,
    /// The conflicting paths, as a list. `kit::render` counts them for
    /// `conflicts in n files`, which is §1's rule: the count derives from the
    /// collection it counts.
    PullConflict {
        files: Vec<String>,
    },
    /// `None` when the role query behind the wording failed. The pause still
    /// stands — the bucket refused — it simply cannot name the role.
    RoleDenied {
        role: Option<String>,
    },
    /// `PausedReason`'s fallback arm: a refusal with no more specific reason,
    /// such as a workflow rejection. `message` is the refusal text. It needs no
    /// state of its own; the package state for it is [`PackageStateDto::Paused`].
    Other {
        message: String,
    },
}

impl From<&PausedReason> for PausedDto {
    fn from(reason: &PausedReason) -> Self {
        match reason {
            PausedReason::PendingChanges => Self::PendingChanges,
            PausedReason::PendingCommit => Self::PendingCommit,
            PausedReason::Diverged => Self::Diverged,
            PausedReason::PullConflict(files) => Self::PullConflict {
                files: files.clone(),
            },
            PausedReason::RoleDenied { role } => Self::RoleDenied {
                role: (!role.is_empty()).then(|| role.clone()),
            },
            PausedReason::Other(message) => Self::Other {
                message: message.clone(),
            },
        }
    }
}

/// One paused package. The namespace is the join key: the queue matches it
/// against the package list's rows to answer §4.3's question — *is this pause
/// explained by a row the user can already see?*
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PausedPackage {
    pub namespace: quilt_uri::Namespace,
    pub reason: PausedDto,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MainPagePackage {
    pub namespace: quilt_uri::Namespace,
    pub state: PackageStateDto,
    /// Epoch milliseconds. The backend being generous with a format the UI can use
    /// directly, rather than the UI carrying date arithmetic.
    ///
    /// Epoch milliseconds: the most recent thing that happened to this copy.
    ///
    /// `None` only when nothing has, which is genuine rather than a gap — see
    /// [`last_changed`].
    pub changed_at: Option<f64>,
    pub bucket: Option<String>,
    /// The catalog this package points at, as the queue's join key.
    ///
    /// `None` for a package with no remote. Not `role_switch_host`, which is
    /// `Some` only where the user holds more than one role — absent exactly in
    /// the signed-out case the queue groups on.
    pub host: Option<String>,
    /// True while the state came from cached lineage alone. The heavy phase clears
    /// it.
    pub provisional: bool,
    /// The host whose role selector this row's switch affordance opens. `Some`
    /// only when the user holds more than one role there, so the affordance is
    /// never a dead end: a single-role user gets the state and no button.
    ///
    /// The denial itself is not a second field — it is `state ==
    /// RoleDenied`. Two fields carrying one fact is §1's rule at payload scale.
    pub role_switch_host: Option<String>,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MainPagePackages {
    pub packages: Vec<MainPagePackage>,
}

/// One host in the Accounts card.
///
/// `signed_in` is whether the host has a directory under the auth dir, which
/// `erase_auth` removes on logout — so it tracks the session, not a guess about
/// one. The words are `HostRow`'s: this carries `bool` and `Option<String>`, and
/// "Signed out" / "Role unavailable" / "Role: analyst" are the component's.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountHost {
    pub host: String,
    pub signed_in: bool,
    /// `None` when the role is not yet resolved, and also when the query failed:
    /// the session stands either way, so this never implies signed out.
    pub current_role: Option<String>,
    /// Every role held here. `HostRow` renders a switcher only above one, so a
    /// short list is not a bug.
    pub roles: Vec<String>,
    /// Still waiting on the role query. Always `false` for a signed-out host —
    /// there is no session to ask.
    pub provisional: bool,
}

impl AccountHost {
    /// The light phase's answer: everything readable from disk, and nothing else.
    fn light(host: String, signed_in: bool) -> Self {
        Self {
            host,
            signed_in,
            current_role: None,
            roles: Vec::new(),
            // A signed-out host has no role to fetch, so it is already final.
            provisional: signed_in,
        }
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MainPageAccounts {
    pub hosts: Vec<AccountHost>,
}

/// Every host worth a row: the catalogs the roster points at, plus the ones with
/// a session on disk.
///
/// Two sources because each is incomplete. The roster's hosts are what the
/// queue's shared causes name; the auth dir's are what makes a session with no
/// installed packages manageable at all.
fn account_hosts(rows: &[Row], auth_hosts: &[String]) -> Vec<String> {
    let mut hosts: Vec<String> = rows
        .iter()
        .filter_map(row_host)
        .map(ToString::to_string)
        .chain(auth_hosts.iter().cloned())
        .collect();
    hosts.sort();
    hosts.dedup();
    hosts
}

/// The tree-derived states, as resolved by `quilt-rs`.
///
/// Resolution happens exactly once, upstream of every payload (§1): both
/// phases, the package page and `quilt list --fetch` hand `quilt-rs`'s
/// `UpstreamState` to the one [`PackageState::resolve`], and a remote with no
/// catalog host is caught by the one
/// [`PackageLineage::misconfigured_remote`](quilt::lineage::PackageLineage::misconfigured_remote)
/// before it. Two places deciding what is true is the 2026-07-11 bug. This
/// mapping only renames.
///
/// `PendingChanges` can only reach rank 7 of §5's precedence lattice; the
/// app-only states above it are added by the callers.
impl From<PackageState> for PackageStateDto {
    fn from(state: PackageState) -> Self {
        match state {
            PackageState::Latest => Self::Latest,
            PackageState::PendingCommit => Self::PendingCommit,
            PackageState::PendingChanges { files } => Self::PendingChanges { files },
            PackageState::Behind => Self::Behind,
            PackageState::Diverged => Self::Diverged,
            PackageState::Unpublished => Self::Unpublished,
            PackageState::NoRemote => Self::NoRemote,
            PackageState::Unknown => Self::Unknown,
        }
    }
}

/// Precedence rank 2: a pull-conflict pause outranks everything the tree's own
/// state can say (ranks 5-7). It is outranked only by a denial (rank 1), which
/// is why the heavy phase applies this **after** its access-denied arm and the
/// light phase applies it before `mark_unreadable_buckets`, which overwrites.
/// The invariant is `a_denial_outranks_a_pause_even_a_pull_conflict`.
///
/// `Other` is deliberately absent: it has no state in the vocabulary, and
/// folding it would destroy the case the queue exists to surface — a package
/// `latest` by hash and paused by a workflow rejection. It travels as a pause on
/// the watcher payload instead. The other three reasons are absent because the
/// tree's own state already says what they say.
/// Whether this pause has no state of its own to resolve into.
///
/// Only `Other`. Every other reason either resolves into the state the working
/// tree already reports (`PendingChanges`, `PendingCommit`, `Diverged`) or has a
/// state of its own (`PullConflict`, `RoleDenied`). `Other` is the catch-all —
/// a workflow rejection, a hash mismatch — and it is non-transient, so a package
/// left in it stays stopped until something outside this app changes.
pub(super) fn unexplained_pause(paused: Option<&PausedReason>) -> bool {
    matches!(paused, Some(PausedReason::Other(_)))
}

pub(super) fn conflict_files(paused: Option<&PausedReason>) -> Option<Vec<String>> {
    match paused {
        Some(PausedReason::PullConflict(files)) => Some(files.clone()),
        _ => None,
    }
}

/// How long the roster waits on one host before giving up on it. The budget
/// covers the readable-bucket query and the role query behind a denial
/// together.
///
/// A copy of `BUCKET_LIST_BUDGET` in `package_list.rs`, not shared with it,
/// because v1 is frozen and will be deleted as a whole. Keep the two in sync.
///
/// Apart from these queries the roster is local data and renders without the
/// network. The bucket query adds two round trips per host (`config.json`, then
/// a GraphQL POST), serialised under the host's credential lock behind a retry
/// middleware with a 10s connect timeout. Without a budget, an unreachable host
/// keeps the main screen blank for tens of seconds. The bucket pre-filter is only
/// a hint: without it, the per-row status call still marks denied rows a moment
/// later. So the budget is short. On a slow link the roster loses only the hint;
/// a longer wait would delay the first render.
///
/// One deadline covers both queries. A separate budget for each, run in series,
/// would double the wait.
const BUCKET_LIST_BUDGET: Duration = Duration::from_secs(2);

/// Whether the active role can reach a row's bucket, and what to say about it.
///
/// COPIED from `package_list.rs:81-87`. v2 carries the ROLE NAME rather than
/// v1's rendered `reason` string: §2 keeps prose off the wire, and
/// `kit::render` already words `RoleDenied` for both sites.
struct AccessMark {
    role: Option<String>,
    role_switch_host: Option<String>,
}

impl AccessMark {
    /// A denial. `roles` is `None` when the role query itself failed: the mark
    /// still stands (the bucket refused either way), it just cannot name the
    /// role, and it offers no switch because we cannot tell whether another
    /// role is held.
    fn denied(host: Option<&Host>, roles: Option<&RoleInfo>) -> Self {
        let holds_another_role = roles.is_some_and(|roles| roles.available.len() > 1);
        Self {
            role: roles.map(|roles| roles.current.clone()),
            role_switch_host: host
                .filter(|_| holds_another_role)
                .map(std::string::ToString::to_string),
        }
    }

    /// The state a denied row resolves to. Ruling R3: denial is precedence rank
    /// 1, so it replaces the mapped state rather than riding beside it.
    fn state(&self) -> PackageStateDto {
        PackageStateDto::RoleDenied {
            role: self.role.clone(),
        }
    }
}

/// Ask who the active role on `host` is, so a denial can name it.
///
/// COPIED from `package_list.rs:133-146`. Goes through the [`RoleCache`], so the
/// answer is fetched once per host per load however many rows deny — the heavy
/// phase runs one command invocation per row, concurrently, and they all land
/// here. A failed query is not fatal: the mark degrades to unnamed.
async fn denied_mark(
    m: &impl model::QuiltModel,
    roles: &RoleCache,
    host: Option<&Host>,
) -> AccessMark {
    let info = match host {
        Some(host) => roles.get(m, host).await.ok(),
        None => None,
    };
    AccessMark::denied(host, info.as_ref())
}

/// A row plus the URI the access pass needs and the wire does not carry.
///
/// §4.4 lists `bucket` and no URI. R1 later put the catalog host on the DTO
/// too (`MainPagePackage.host`), for the queue's join key — but the parsed
/// URI itself has no reason to follow it there: only the two scalars derived
/// from it do, and the access pass below still needs the URI's own `bucket`
/// for its readable-bucket comparison mid-walk.
struct Row {
    package: MainPagePackage,
    uri: Option<quilt_uri::S3PackageUri>,
}

fn row_host(row: &Row) -> Option<&Host> {
    row.uri.as_ref().and_then(|uri| uri.catalog.as_ref())
}

/// Resolve the rows whose bucket is outside what the active role can read.
///
/// COPIED from `package_list.rs:232-283`. One query per host, not per package.
/// The answer is an optimistic hint — it over-reports for unmanaged roles and
/// anonymous-access stacks, and says nothing about writes — so a miss only greys
/// the row; the authoritative answer still comes from the per-row status call,
/// which clears the mark in both directions.
///
/// A failed query degrades to reactive-only marking. It must NEVER be read as
/// "nothing is readable": an empty set would grey the entire roster. A query
/// that does not answer inside [`BUCKET_LIST_BUDGET`] counts as failed, which is
/// what keeps the roster local-only in effect.
async fn mark_unreadable_buckets(m: &impl model::QuiltModel, roles: &RoleCache, rows: &mut [Row]) {
    let mut hosts: Vec<Host> = Vec::new();
    for host in rows.iter().filter_map(row_host) {
        if !hosts.contains(host) {
            hosts.push(host.clone());
        }
    }

    for host in hosts {
        // One deadline for the host, shared by both calls below.
        let deadline = Instant::now() + BUCKET_LIST_BUDGET;
        let query = timeout_at(deadline, m.readable_buckets(&host));
        let readable: HashSet<String> = match query.await {
            Ok(Ok(buckets)) => buckets.into_iter().collect(),
            Ok(Err(err)) => {
                tracing::debug!("No readable-bucket list for {host}: {err}");
                continue;
            }
            Err(_) => {
                tracing::debug!("Readable-bucket list for {host} timed out");
                continue;
            }
        };

        let unreadable: Vec<usize> = rows
            .iter()
            .enumerate()
            .filter(|(_, row)| row_host(row) == Some(&host))
            .filter(|(_, row)| {
                row.uri
                    .as_ref()
                    .is_some_and(|uri| !readable.contains(&uri.bucket))
            })
            .map(|(index, _)| index)
            .collect();
        if unreadable.is_empty() {
            // A host the role can fully read costs one query, not two.
            continue;
        }

        // Whatever is left of the host's budget. The name is the cosmetic
        // half — the bucket already said no — so a denial with no role is the
        // right answer here, and the vocabulary has words for exactly that
        // (`PackageState::RoleDenied { role: None }`). Dropping the denial to
        // keep the name would trade a certain fact for a label.
        let mark = timeout_at(deadline, denied_mark(m, roles, Some(&host)))
            .await
            .unwrap_or_else(|_| {
                tracing::debug!("Role query for {host} timed out");
                AccessMark::denied(Some(&host), None)
            });
        for index in unreadable {
            let row = &mut rows[index];
            row.package.state = mark.state();
            row.package
                .role_switch_host
                .clone_from(&mark.role_switch_host);
        }
    }
}

/// Walk the installed packages and load each one's row. One roster walk for
/// both the Autosync card and the Accounts card — see `load_main_page_package`
/// for what a row carries and how a failure is handled.
async fn load_rows(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
    paused_reasons: &HashMap<quilt_uri::Namespace, PausedReason>,
) -> Result<Vec<Row>, Error> {
    let list = m.get_installed_packages_list().await?;
    let mut rows = Vec::new();
    for installed_package in list {
        match load_main_page_package(m, tracing, &installed_package, paused_reasons).await {
            Ok(row) => rows.push(row),
            Err(err) => {
                tracing::warn!(
                    "Failed to load package {}: {err}",
                    installed_package.namespace,
                );
            }
        }
    }
    Ok(rows)
}

async fn get_main_page_packages_from_model(
    m: &impl model::QuiltModel,
    roles: &RoleCache,
    tracing: &crate::telemetry::Telemetry,
    paused_reasons: &HashMap<quilt_uri::Namespace, PausedReason>,
) -> Result<MainPagePackages, Error> {
    // Clear the role cache on every load, as v1's
    // `get_installed_packages_list_data_from_model` does (a copy, not shared).
    // A role switch is server-side and global, so it can happen in the web
    // catalog without the app knowing. If the cached name lived for the whole
    // session, a row could name a role that in fact has access, and
    // `observe_role` would never run again, so the S3 clients would keep signing
    // as the old role until Settings is opened or the ~1h credential TTL
    // expires. All hosts are cleared because the roster's hosts are known only
    // after `load_rows` below, and a cache entry is only a name.
    roles.invalidate(None).await;

    let mut rows = load_rows(m, tracing, paused_reasons).await?;
    mark_unreadable_buckets(m, roles, &mut rows).await;
    Ok(MainPagePackages {
        packages: rows.into_iter().map(|row| row.package).collect(),
    })
}

/// When this copy last changed: the last local commit, or the last file we installed
/// or committed, whichever is later. Epoch milliseconds.
///
/// Not a filesystem mtime, and no directory walk happens here. Both values come
/// from `data.json`, which the light phase has already read, so this does no I/O.
///
/// `PathState`'s doc explains the difference: *"We don't track files
/// modifications in real time. We calculate hash when we commit or install file."*
/// So this is the last time `QuiltSync` wrote to the copy, not the last time
/// anything on disk changed. A later edit in the working directory does not move
/// it; the heavy phase's hashing finds those edits.
///
/// `None` means we have never written to this package: no commit, and no installed
/// paths. That is a real answer, not a missing one, which is why the row says
/// `not recorded` rather than leaving the cell blank.
fn last_changed(lineage: &quilt::lineage::PackageLineage) -> Option<f64> {
    let newest = lineage
        .commit
        .as_ref()
        .map(|commit| commit.timestamp)
        .into_iter()
        .chain(lineage.paths.values().map(|path| path.timestamp).max())
        .max()?;
    Some(epoch_millis(newest))
}

async fn load_main_page_package(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
    installed_package: &quilt::InstalledPackage,
    paused_reasons: &HashMap<quilt_uri::Namespace, PausedReason>,
) -> Result<Row, Error> {
    let namespace = installed_package.namespace.clone();

    let lineage = m.get_installed_package_lineage(installed_package).await?;
    // Computed before `lineage` is moved by the `into()` below.
    let has_local_commit = lineage.commit.is_some();
    let has_remote = lineage.remote_uri.is_some();
    let bucket = lineage.remote_uri.as_ref().map(|uri| uri.bucket.clone());
    let changed_at = last_changed(&lineage);
    let typed_uri = lineage
        .remote_uri
        .as_ref()
        .map(quilt_uri::S3PackageUri::from);
    // Crash reports are attributed to the deployment of the most recent action.
    // v1 does this in its own light phase (`package_list.rs:334`); without it here,
    // the attribution dies with v1.
    if let Some(host) = typed_uri.as_ref().and_then(|uri| uri.catalog.as_ref()) {
        tracing.add_host(host);
    }
    let host = typed_uri
        .as_ref()
        .and_then(|uri| uri.catalog.as_ref())
        .map(ToString::to_string);
    let state = if lineage.misconfigured_remote() {
        PackageStateDto::Unknown
    } else if let Some(files) = conflict_files(paused_reasons.get(&namespace)) {
        // Rank 2, below the arm above: a remote with no catalog host is a package
        // the tick never touched, so a conflict pause cannot be describing it.
        PackageStateDto::PullConflict { files }
    } else if unexplained_pause(paused_reasons.get(&namespace)) {
        // §5's row 3, below the conflict above, which names its files and is the
        // more specific fact about the same disk.
        PackageStateDto::Paused
    } else {
        // `None`, not `Some(0)`: this phase has not looked at the working tree.
        // The heavy phase (`refresh_main_page_package`) measures it.
        PackageState::resolve(lineage.into(), has_local_commit, has_remote, None).into()
    };

    // `provisional` means the state came from cached lineage and the remote has
    // not confirmed it. A pull conflict and an unexplained pause are not that:
    // both come from the watcher's paused map above, so they are facts about
    // this disk that no network call can confirm. The queue drops provisional
    // rows to keep the access pre-filter's guesses out of it. If these two were
    // provisional, they would leave the queue whenever the remote is
    // unreachable, which is when syncing cannot fix them.
    let provisional = !matches!(
        state,
        PackageStateDto::PullConflict { .. } | PackageStateDto::Paused
    );

    Ok(Row {
        package: MainPagePackage {
            namespace,
            state,
            changed_at,
            bucket,
            host,
            provisional,
            role_switch_host: None,
        },
        uri: typed_uri,
    })
}

/// Walk the installed packages, read each one's cached lineage, resolve, and
/// attach the paused reason from the watcher's authoritative map. A failed
/// package is warned-and-skipped, per `package_list.rs:198-206` — one bad
/// lineage must not blank the whole list.
#[tauri::command]
pub async fn get_main_page_packages(
    m: tauri::State<'_, model::Model>,
    roles: tauri::State<'_, RoleCache>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    watcher: tauri::State<'_, Watcher>,
) -> Result<MainPagePackages, String> {
    // From the same one read the watcher payload uses. The old route went
    // through `PausedEvent` and `filter_map`ped away every pause with no
    // message — `PendingChanges`, `PendingCommit` and `Diverged` — so three of
    // the six reasons never reached this map at all.
    // Keyed by the type the watcher already hands over: this map used to flatten
    // each key to a string, and every lookup below then had one too.
    let paused_reasons: HashMap<quilt_uri::Namespace, PausedReason> =
        watcher.main_page_facts().await.paused.into_iter().collect();

    let started = std::time::Instant::now();
    let result = get_main_page_packages_from_model(&*m, &roles, &tracing, &paused_reasons).await;
    tracing::debug!(
        elapsed_ms = started.elapsed().as_millis(),
        packages = result.as_ref().map_or(0, |r| r.packages.len()),
        "main page light phase"
    );
    result.map_err(|e| e.to_frontend_string())
}

/// The Accounts card, light phase: one row per host, resolved from disk alone.
///
/// No network. The role costs a `/me` per host and arrives via
/// `refresh_main_page_account`, one invocation per row. The roster walk is
/// [`load_rows`] — the same one `get_main_page_packages` uses — with an empty
/// paused-reasons map: the Accounts card has no use for package state.
#[tauri::command]
pub async fn get_main_page_accounts(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
) -> Result<MainPageAccounts, String> {
    let data_dir = {
        let app_handle = app_handle.lock().await;
        app_handle
            .path()
            .app_local_data_dir()
            .map_err(|e| Error::from(e).to_string())?
    };
    let auth_hosts = quilt::paths::list_auth_hosts(&data_dir);
    let rows = load_rows(&*m, &tracing, &HashMap::new())
        .await
        .map_err(|e| e.to_string())?;

    let hosts = account_hosts(&rows, &auth_hosts)
        .into_iter()
        .map(|host| {
            let signed_in = auth_hosts.contains(&host);
            AccountHost::light(host, signed_in)
        })
        .collect();

    Ok(MainPageAccounts { hosts })
}

/// Fill in one host's role. Extracted from the command so it is testable without
/// a Tauri `AppHandle`, the same split `refresh_main_page_package` uses.
async fn refresh_account_for(
    m: &impl model::QuiltModel,
    roles: &RoleCache,
    host: &str,
    signed_in: bool,
) -> AccountHost {
    // R4: no session, nothing to ask, and the light phase already said so.
    if !signed_in {
        return AccountHost::light(host.to_string(), false);
    }

    // `Host::from_str` is fallible (there is no infallible `Host::from(String)`).
    // A host string this layer cannot parse gets the same settlement R5 gives a
    // failed query below — we asked and could not tell — rather than failing the
    // whole command over a name the light phase already accepted.
    let parsed = match Host::from_str(host) {
        Ok(parsed) => parsed,
        Err(err) => {
            tracing::debug!("Could not parse host {host}: {err}");
            return AccountHost {
                host: host.to_string(),
                signed_in: true,
                current_role: None,
                roles: Vec::new(),
                provisional: false,
            };
        }
    };

    match roles.get(m, &parsed).await {
        Ok(info) => AccountHost {
            host: host.to_string(),
            signed_in: true,
            current_role: Some(info.current),
            roles: info.available,
            provisional: false,
        },
        // R5: asked, could not tell. The session stands; only its name is missing.
        Err(err) => {
            tracing::debug!("No role for {host}: {err}");
            AccountHost {
                host: host.to_string(),
                signed_in: true,
                current_role: None,
                roles: Vec::new(),
                provisional: false,
            }
        }
    }
}

/// The Accounts card, heavy phase: one host's role, one invocation per row.
#[tauri::command]
pub async fn refresh_main_page_account(
    m: tauri::State<'_, model::Model>,
    roles: tauri::State<'_, RoleCache>,
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    host: String,
) -> Result<AccountHost, String> {
    let data_dir = {
        let app_handle = app_handle.lock().await;
        app_handle
            .path()
            .app_local_data_dir()
            .map_err(|e| Error::from(e).to_string())?
    };
    // Re-read rather than trusting the caller: a logout between the two phases
    // must not produce a role query against a host with no session.
    let signed_in = quilt::paths::list_auth_hosts(&data_dir).contains(&host);

    Ok(refresh_account_for(&*m, &roles, &host, signed_in).await)
}

// ── The heavy phase ──

/// What the per-package refresh corrects on a row.
///
/// Deliberately not the whole `MainPagePackage`: `namespace`, `bucket`, `host`
/// and `changed_at` are facts the light phase read from `data.json` and the
/// network has nothing to say about them. Sending them again would be a
/// second source for a value that already arrived.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MainPagePackageRefresh {
    pub state: PackageStateDto,
    /// See [`MainPagePackage::role_switch_host`]. Carried here too, and cleared
    /// here too: the refresh is the real call, so it has the last word in BOTH
    /// directions. Only ever *adding* the mark made a false positive permanent
    /// for the life of the page.
    pub role_switch_host: Option<String>,
}

/// The heavy phase: one real status call, resolved through the SAME
/// [`PackageState::resolve`] the light phase uses.
///
/// §1 — resolution happens exactly once, upstream of every payload. The light
/// and heavy phases differ in what they *measure*, never in how they decide:
/// both hand `quilt-rs`'s `UpstreamState` to one function. That is what stops
/// the two phases contradicting each other, which is the 2026-07-11 bug at
/// row scale.
pub(super) async fn refresh_main_page_package_from_model(
    m: &impl model::QuiltModel,
    roles: &RoleCache,
    tracing: &crate::telemetry::Telemetry,
    namespace: &quilt_uri::Namespace,
    paused: Option<&PausedReason>,
) -> Result<MainPagePackageRefresh, Error> {
    let installed_package = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(
            namespace.to_owned(),
        ))
    })?;
    let lineage = m.get_installed_package_lineage(&installed_package).await?;
    // Read now; `upstream_state` comes from the status call below, so a commit
    // landing in between leaves these stale until the next refresh (e.g. `Latest`
    // where `PendingCommit` was due). `InstalledPackageStatus` carries no commit
    // field, so there is no better source — seen and accepted, not fixable here.
    let has_local_commit = lineage.commit.is_some();
    let has_remote = lineage.remote_uri.is_some();
    let host = lineage
        .remote_uri
        .as_ref()
        .and_then(|uri| uri.origin.clone());

    // A remote with a bucket but no catalog host: nowhere to vend credentials
    // from, so the status call cannot succeed and its answer would not be
    // actionable if it did. The SAME predicate the light phase uses, so the two
    // phases cannot disagree about this shape.
    if lineage.misconfigured_remote() {
        return Ok(MainPagePackageRefresh {
            state: PackageStateDto::Unknown,
            role_switch_host: None,
        });
    }

    // No remote at all: `PackageState::resolve` ignores the file count for `Local`, so the
    // status call would be a local hash walk whose answer nothing reads. A remote
    // that exists but has never been pushed to still goes through the call below —
    // it is reachable, and `Unpublished` is what comes back.
    if !has_remote {
        return Ok(MainPagePackageRefresh {
            state: PackageState::resolve(lineage.into(), has_local_commit, false, None).into(),
            role_switch_host: None,
        });
    }

    if let Some(host) = host.as_ref() {
        tracing.add_host(host);
    }

    match m
        .get_installed_package_status(&installed_package, None)
        .await
    {
        Ok(status) => Ok(MainPagePackageRefresh {
            // The pause checks go here rather than at the top of the function,
            // because the access-denied arm below has higher precedence and must
            // still be able to win.
            //
            // Both pause checks, in the same order as the light phase. A pause
            // takes precedence over the working tree's state because it is the
            // reason the tree is not being synced. Without the unexplained-pause
            // check, this phase would replace the light phase's `Paused` with the
            // measured tree state.
            state: if let Some(files) = conflict_files(paused) {
                PackageStateDto::PullConflict { files }
            } else if unexplained_pause(paused) {
                PackageStateDto::Paused
            } else {
                PackageState::resolve(
                    status.upstream_state,
                    has_local_commit,
                    has_remote,
                    Some(status.changes.len()),
                )
                .into()
            },
            // The refresh did not deny, so any pre-filter mark is cleared.
            role_switch_host: None,
        }),
        Err(err) if err.is_access_denied() => {
            // Ruling R3: denial is precedence rank 1, so it REPLACES the state
            // rather than riding beside it, and `render` gives it no action —
            // which is the invariant
            // `denied_row_hides_publish_that_a_readable_row_with_the_same_changes_offers`.
            let mark = denied_mark(m, roles, host.as_ref()).await;
            Ok(MainPagePackageRefresh {
                state: mark.state(),
                role_switch_host: mark.role_switch_host,
            })
        }
        // A failure to reach the remote is not evidence about the package — it is
        // not "Sync stopped" (Unknown), which asserts a state the call never
        // earned. Propagate the error instead: the command surfaces it, and the
        // UI's `PackageListRow` `Err` arm already takes the honest path, keeping
        // the light phase's guess and leaving the row provisional (dashed) rather
        // than replacing it with a false answer.
        Err(err) => {
            tracing::warn!(
                "Failed to get status for {}: {err}",
                installed_package.namespace,
            );
            Err(err)
        }
    }
}

/// One package's real state. Called once per row, concurrently — see Ruling R0
/// in the plan and `RoleCache::get`'s doc comment, which is written for exactly
/// this caller.
#[tauri::command]
pub async fn refresh_main_page_package(
    m: tauri::State<'_, model::Model>,
    roles: tauri::State<'_, RoleCache>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    watcher: tauri::State<'_, Watcher>,
    namespace: String,
) -> Result<MainPagePackageRefresh, String> {
    let namespace: quilt_uri::Namespace = namespace
        .try_into()
        .map_err(|e: quilt_uri::UriError| e.to_string())?;

    // One lookup, at the moment of the refresh. It can differ from what the
    // light phase saw — the tick runs in between — which is the same
    // between-phases staleness `provisional` already exists to express.
    let paused = watcher.paused_reason(&namespace).await;

    refresh_main_page_package_from_model(&*m, &roles, &tracing, &namespace, paused.as_ref())
        .await
        .map_err(|e| e.to_frontend_string())
}

// ── The watcher's payload ──

/// Whether a direction's machinery is counting down, waiting for something to
/// do, or stopped. §4.2's three states, and the whole vocabulary of the toggle's
/// trailing slot — a closed set, because a fourth would be a design change and
/// not a wire addition.
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToggleActivity {
    Armed,
    Idle,
    Paused,
}

/// One direction of autosync, as the Autosync card needs it.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct ToggleState {
    /// The **setting**, which stays `true` while paused — what stopped is the
    /// machinery.
    pub enabled: bool,
    pub activity: ToggleActivity,
    /// Epoch milliseconds. `Some` exactly when `activity` is `Armed`.
    pub deadline: Option<f64>,
    /// The whole wait, in milliseconds. A determinate ring cannot be drawn from
    /// a remaining time alone.
    pub interval_ms: f64,
}

/// Payload 3 of the three (§8 decision 3). Carries no counts: the queue derives
/// its own from the collections it renders.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MainPageWatcher {
    pub pull: ToggleState,
    pub publish: ToggleState,
    /// The paused set, typed. It is meant for the queue, and nothing reads it
    /// yet: the Autosync card needs only `activity`, which is derived from this
    /// list.
    pub paused: Vec<PausedPackage>,
}

/// `i64` milliseconds into `f64`, because JavaScript has no other number. The
/// one conversion, shared by [`last_changed`], [`toggle_state`], and
/// [`recent_files`] — every epoch timestamp on this page's wire goes through
/// this function, not a second `as` chain with its own rounding.
#[allow(
    clippy::cast_precision_loss,
    reason = "epoch millis fit f64 exactly for any date this program can see"
)]
fn epoch_millis(at: DateTime<Utc>) -> f64 {
    at.timestamp_millis() as f64
}

/// One direction's state, derived once and called twice.
///
/// The ordering is the ruling: `enabled` is checked before `paused`, because a
/// setting that is off has no machinery to have stopped. And `deadline` is
/// returned by the same expression that decides `activity`, so §4.2's "`armed`
/// only" holds by construction rather than by two callers agreeing.
fn toggle_state(
    enabled: bool,
    any_paused: bool,
    deadline: Option<DateTime<Utc>>,
    interval: Duration,
) -> ToggleState {
    let (activity, deadline) = if !enabled {
        (ToggleActivity::Idle, None)
    } else if any_paused {
        (ToggleActivity::Paused, None)
    } else {
        match deadline {
            Some(at) => (ToggleActivity::Armed, Some(epoch_millis(at))),
            None => (ToggleActivity::Idle, None),
        }
    };
    ToggleState {
        enabled,
        activity,
        deadline,
        // `as_secs_f64` rather than `as_millis()`: no cast, no lint, no
        // truncation to argue about.
        interval_ms: interval.as_secs_f64() * 1000.0,
    }
}

impl From<WatcherFacts> for MainPageWatcher {
    fn from(facts: WatcherFacts) -> Self {
        // R2: a pause of any reason stops both directions, because `run_once`
        // skips a paused namespace above both branches (`tick.rs`). There is no
        // per-direction pause state in the watcher to read, and inventing one
        // would be a second resolution of what this map already decides.
        let any_paused = !facts.paused.is_empty();
        Self {
            pull: toggle_state(
                facts.pull_enabled,
                any_paused,
                facts.next_pull_at,
                facts.pull_interval,
            ),
            publish: toggle_state(
                facts.publish_enabled,
                any_paused,
                facts.publish_arm_at,
                facts.publish_interval,
            ),
            paused: facts
                .paused
                .iter()
                .map(|(namespace, reason)| PausedPackage {
                    namespace: namespace.clone(),
                    reason: PausedDto::from(reason),
                })
                .collect(),
        }
    }
}

/// The watcher's own payload. A memory read — the settings, the paused map and
/// the two clocks are all `RwLock`s the watcher already holds — so it has no
/// skeleton and needs none (§6: chrome is never skeletonised).
#[tauri::command]
pub async fn get_main_page_watcher(
    watcher: tauri::State<'_, Watcher>,
) -> Result<MainPageWatcher, String> {
    Ok(MainPageWatcher::from(watcher.main_page_facts().await))
}

// ── The recent files feed ──

/// One installed or published file, flat across every package. §3.2: the
/// owning package travels with the row rather than grouping it, so the list
/// component never has to flatten what the backend already flattened.
#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MainPageFile {
    pub path: String,
    pub namespace: quilt_uri::Namespace,
    /// Epoch milliseconds — see [`MainPagePackage::changed_at`]. Never `None`
    /// here: a path only exists in `lineage.paths` because it was installed or
    /// committed, and both of those write a timestamp.
    pub changed_at: f64,
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct MainPageRecentFiles {
    pub files: Vec<MainPageFile>,
}

/// §4.5: "bounded and pre-selected by the backend — newest first, ~50."
/// Applied after the global sort, so the cap keeps the newest rather than an
/// arbitrary fifty.
const RECENT_FILES_LIMIT: usize = 50;

/// Flatten every installed package's `paths` into one list, newest first.
///
/// No network and no hashing: `PathState.timestamp` is already in `data.json`
/// and this is the same lineage read the light phase does. A package whose
/// lineage cannot be read is warned-and-skipped, matching [`load_rows`] — one
/// bad lineage must not blank the whole feed.
async fn recent_files(m: &impl model::QuiltModel) -> Result<Vec<MainPageFile>, Error> {
    let list = m.get_installed_packages_list().await?;
    let mut files = Vec::new();
    for installed_package in list {
        let lineage = match m.get_installed_package_lineage(&installed_package).await {
            Ok(lineage) => lineage,
            Err(err) => {
                tracing::warn!(
                    "Failed to load lineage for {}: {err}",
                    installed_package.namespace,
                );
                continue;
            }
        };
        let namespace = installed_package.namespace.clone();
        for (path, state) in &lineage.paths {
            files.push(MainPageFile {
                path: path.to_string_lossy().into_owned(),
                namespace: namespace.clone(),
                changed_at: epoch_millis(state.timestamp),
            });
        }
    }
    // Newest first, then bound. `total_cmp` because `f64` has no `Ord` — the
    // same comparison `last_changed`'s callers would need if they ever sorted.
    files.sort_by(|a, b| b.changed_at.total_cmp(&a.changed_at));
    files.truncate(RECENT_FILES_LIMIT);
    Ok(files)
}

/// The recent files feed. Single-phase and final on arrival: a path and an
/// mtime are facts on disk, so there is no heavy phase and no `provisional`.
#[tauri::command]
pub async fn get_main_page_recent_files(
    m: tauri::State<'_, model::Model>,
) -> Result<MainPageRecentFiles, String> {
    recent_files(&*m)
        .await
        .map(|files| MainPageRecentFiles { files })
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::commands::test_support::*;
    use crate::quilt::lineage::UpstreamState;

    #[test]
    fn a_conflict_crosses_the_wire_as_a_list_not_a_joined_string() {
        // v1's `reporter` sends `files.join(", ")`. Counting files for
        // "conflicts in N files" from that string means splitting on ", ", which
        // gives the wrong count for this file name.
        let reason =
            PausedReason::PullConflict(vec!["plate, run 3.csv".to_string(), "b.csv".to_string()]);
        let json = serde_json::to_string(&PausedDto::from(&reason)).unwrap();
        assert_eq!(
            json,
            r#"{"kind":"pull_conflict","files":["plate, run 3.csv","b.csv"]}"#
        );
        assert!(
            !json.contains(r#""plate, run 3.csv, b.csv""#),
            "a joined list cannot be counted back apart"
        );
    }

    #[test]
    fn the_three_meanings_of_one_field_become_three_named_fields() {
        // `PausedEvent.message` is one `Option<String>` carrying a raw error for
        // `other`, comma-joined filenames for `pullConflict`, and a role name for
        // `roleDenied`. Each gets its own field and its own name here.
        assert_eq!(
            serde_json::to_string(&PausedDto::from(&PausedReason::Other(
                "workflow rejected metadata".to_string()
            )))
            .unwrap(),
            r#"{"kind":"other","message":"workflow rejected metadata"}"#
        );
        assert_eq!(
            serde_json::to_string(&PausedDto::from(&PausedReason::RoleDenied {
                role: "analyst".to_string()
            }))
            .unwrap(),
            r#"{"kind":"role_denied","role":"analyst"}"#
        );
        assert_eq!(
            serde_json::to_string(&PausedDto::from(&PausedReason::PullConflict(vec![
                "a.csv".to_string()
            ])))
            .unwrap(),
            r#"{"kind":"pull_conflict","files":["a.csv"]}"#
        );
    }

    #[test]
    fn an_unresolved_role_crosses_as_null_not_as_an_empty_name() {
        // `PausedReason::RoleDenied { role: String }` uses `""` for "the role query
        // itself failed" — `reporter.rs:170-172` already drops the message in that
        // case. The denial stands; it just cannot be named. Same shape plan 2's R2
        // gave `PackageStateDto::RoleDenied`.
        assert_eq!(
            serde_json::to_string(&PausedDto::from(&PausedReason::RoleDenied {
                role: String::new()
            }))
            .unwrap(),
            r#"{"kind":"role_denied","role":null}"#
        );
    }

    #[test]
    fn the_reasons_that_carry_nothing_carry_nothing() {
        // Three of the six are fully described by their kind. A `message: null` on
        // these is the overloaded field surviving in a new shape.
        for (reason, expected) in [
            (
                PausedReason::PendingChanges,
                r#"{"kind":"pending_changes"}"#,
            ),
            (PausedReason::PendingCommit, r#"{"kind":"pending_commit"}"#),
            (PausedReason::Diverged, r#"{"kind":"diverged"}"#),
        ] {
            assert_eq!(
                serde_json::to_string(&PausedDto::from(&reason)).unwrap(),
                expected
            );
        }
    }

    #[test]
    fn a_paused_package_names_itself_and_its_reason() {
        let row = PausedPackage {
            namespace: quilt_uri::Namespace::try_from("team/plate-07")
                .expect("a valid fixture namespace"),
            reason: PausedDto::from(&PausedReason::Diverged),
        };
        assert_eq!(
            serde_json::to_string(&row).unwrap(),
            r#"{"namespace":"team/plate-07","reason":{"kind":"diverged"}}"#
        );
    }

    /// Rows whose only interesting property is their catalog host. `None`
    /// builds a row with no `uri` at all — the shape a local-only package
    /// takes in `load_main_page_package`, where `typed_uri` stays `None`
    /// when there is no remote to derive it from.
    fn rows_for_hosts(hosts: &[Option<&str>]) -> Vec<Row> {
        hosts
            .iter()
            .enumerate()
            .map(|(i, host)| Row {
                package: MainPagePackage {
                    namespace: quilt_uri::Namespace::try_from(format!("team/pkg{i}"))
                        .expect("a valid fixture namespace"),
                    state: PackageStateDto::Latest,
                    changed_at: None,
                    bucket: None,
                    host: None,
                    provisional: false,
                    role_switch_host: None,
                },
                uri: host.map(|host| quilt_uri::S3PackageUri {
                    catalog: Some(host.parse().unwrap()),
                    bucket: "bucket".to_string(),
                    namespace: "team/pkg".try_into().unwrap(),
                    revision: quilt_uri::RevisionPointer::Hash("abcdef".to_string()),
                    path: None,
                }),
            })
            .collect()
    }

    /// One `Row` built through `load_main_page_package`'s real path — not a
    /// hand-built `Row` like [`rows_for_hosts`], which only ever proves what
    /// its own fixture already asserts. `Some(host)` drives a lineage with a
    /// remote whose origin is that host; `None` drives a lineage with no
    /// remote at all, the shape a local-only package takes.
    fn row_for_catalog(host: Option<&str>) -> Row {
        let installed = make_installed_package(("team", "one"));
        let lineage = match host {
            Some(host) => quilt::lineage::PackageLineage::from_remote(
                quilt_uri::ManifestUri {
                    origin: Some(host.parse().unwrap()),
                    bucket: "test".to_string(),
                    namespace: "team/one".try_into().unwrap(),
                    hash: "abcdef".to_string(),
                },
                "abcdef".to_string(),
            ),
            None => quilt::lineage::PackageLineage::default(),
        };

        let mut model = crate::model::mocks::create();
        model
            .expect_get_installed_package_lineage()
            .return_once(move |_| Ok(lineage));

        tokio::runtime::Runtime::new()
            .unwrap()
            .block_on(load_main_page_package(
                &model,
                &crate::telemetry::Telemetry::default(),
                &installed,
                &HashMap::new(),
            ))
            .expect("load_main_page_package should succeed")
    }

    #[test]
    fn a_package_carries_the_catalog_it_points_at() {
        // R1. The queue groups signed-out packages by host, and `role_switch_host`
        // cannot serve: it is `Some` only when the user holds more than one role
        // there, so it is absent exactly when the user is signed out.
        let row = row_for_catalog(Some("open.quiltdata.com"));
        assert_eq!(row.package.host.as_deref(), Some("open.quiltdata.com"));
    }

    #[test]
    fn a_local_only_package_has_no_host() {
        // `row_host` is `row.uri.and_then(|uri| uri.catalog)`, so a package with no
        // remote has no catalog. It must be `None`, never an empty string — the
        // queue would otherwise group every local package under a host named "".
        let row = row_for_catalog(None);
        assert_eq!(row.package.host, None);
    }

    #[test]
    fn the_host_set_is_the_union_of_the_roster_and_the_auth_dir() {
        // R2. Roster-only would hide a session the user holds but has no packages
        // from, making it impossible to sign out of from this page. Auth-only would
        // hide the host every "signed out from X — 11 packages" cause names.
        let rows = rows_for_hosts(&[Some("open.quiltdata.com"), Some("team.registry.io")]);
        let auth = vec![
            "open.quiltdata.com".to_string(),
            "solo.registry.io".to_string(),
        ];
        assert_eq!(
            account_hosts(&rows, &auth),
            vec![
                "open.quiltdata.com".to_string(),
                "solo.registry.io".to_string(),
                "team.registry.io".to_string(),
            ],
            "sorted, deduplicated, and a host in both appears once"
        );
    }

    #[test]
    fn a_row_without_a_catalog_contributes_no_host() {
        // `row_host` is `row.uri.and_then(|uri| uri.catalog)`, so a local-only
        // package (no `uri` at all) has no host. It must not become an
        // empty-string row, and must not suppress the host of a row that
        // legitimately has one — the roster's own hosts are not just this one.
        let rows = rows_for_hosts(&[None, Some("team.registry.io")]);
        assert_eq!(
            account_hosts(&rows, &[]),
            vec!["team.registry.io".to_string()]
        );
    }

    #[test]
    fn a_signed_out_host_is_settled_not_provisional() {
        // R4. There is no session to ask about, so the row is final on arrival and
        // the heavy phase must never be asked to fill it in.
        let host = AccountHost::light("solo.registry.io".to_string(), false);
        assert!(!host.signed_in);
        assert!(!host.provisional, "nothing to wait for");
        assert_eq!(host.current_role, None);
        assert_eq!(host.roles, [] as [String; 0]);
    }

    #[test]
    fn a_signed_in_host_arrives_provisional() {
        // Its role costs a round trip, so the light phase paints it unresolved and
        // the heavy phase settles it — the same split `provisional` already carries
        // on the package payload.
        let host = AccountHost::light("open.quiltdata.com".to_string(), true);
        assert!(host.signed_in);
        assert!(host.provisional);
        assert_eq!(host.current_role, None);
    }

    #[test]
    fn the_accounts_payload_serializes_the_wire_shape() {
        // Character-for-character what `MainPageAccountsData` must deserialize.
        // Compared against real serializer output, never a literal against itself.
        let payload = MainPageAccounts {
            hosts: vec![
                AccountHost::light("open.quiltdata.com".to_string(), true),
                AccountHost::light("solo.registry.io".to_string(), false),
            ],
        };
        assert_eq!(
            serde_json::to_value(&payload).unwrap(),
            serde_json::json!({
                "hosts": [
                    {"host": "open.quiltdata.com", "signedIn": true, "currentRole": null,
                     "roles": [], "provisional": true},
                    {"host": "solo.registry.io", "signedIn": false, "currentRole": null,
                     "roles": [], "provisional": false}
                ]
            })
        );
    }

    /// A model whose `observe_role` call succeeds with `info`. `RoleCache::get`
    /// routes through `observe_role`, which calls `refresh_roles` then
    /// `clear_remote_client_cache` on the success path — both need an
    /// expectation, or `mockall` panics on the unmatched one.
    fn mock_with_role(info: RoleInfo) -> crate::model::MockQuiltModel {
        let mut model = crate::model::mocks::create();
        model
            .expect_refresh_roles()
            .times(1)
            .returning(move |_| Ok(info.clone()));
        model.expect_clear_remote_client_cache().returning(|_| ());
        model
    }

    /// A model whose role query fails. `observe_role`'s `?` on `refresh_roles`
    /// short-circuits before `clear_remote_client_cache`, so only the first
    /// needs an expectation.
    fn mock_whose_role_query_fails() -> crate::model::MockQuiltModel {
        let mut model = crate::model::mocks::create();
        model
            .expect_refresh_roles()
            .times(1)
            .returning(|_| Err(Error::General("role query unavailable".to_string())));
        model
    }

    /// A model that must never be asked for a role. `mockall`'s default
    /// `TimesRange` is satisfied at zero calls, so `.times(0)` is the
    /// assertion, not the presence of an `.expect_...`.
    fn mock_that_must_not_be_asked() -> crate::model::MockQuiltModel {
        let mut model = crate::model::mocks::create();
        model.expect_refresh_roles().times(0);
        model
    }

    #[tokio::test]
    async fn the_heavy_phase_names_the_role_and_its_alternatives() {
        // The whole point of the second phase. `RoleInfo.available` is what decides
        // whether `HostRow` draws a switcher, so both fields cross together.
        let m = mock_with_role(RoleInfo {
            current: "analyst".to_string(),
            available: vec!["analyst".to_string(), "admin".to_string()],
        });
        let roles = RoleCache::default();
        let host = refresh_account_for(&m, &roles, "open.quiltdata.com", true).await;

        assert_eq!(host.current_role.as_deref(), Some("analyst"));
        assert_eq!(host.roles, vec!["analyst".to_string(), "admin".to_string()]);
        assert!(!host.provisional, "the row has settled");
        assert!(host.signed_in);
    }

    #[tokio::test]
    async fn a_failed_role_query_settles_the_row_without_signing_it_out() {
        // R5. `HostRow`'s own doc: empty role means the query failed, "the session is
        // fine, it simply cannot be named". Conflating this with a logout would offer
        // a [Sign in] button to someone already signed in.
        let m = mock_whose_role_query_fails();
        let roles = RoleCache::default();
        let host = refresh_account_for(&m, &roles, "open.quiltdata.com", true).await;

        assert!(host.signed_in, "a network failure is not a logout");
        assert_eq!(host.current_role, None);
        assert_eq!(host.roles, [] as [String; 0]);
        assert!(
            !host.provisional,
            "we asked and could not tell — that is settled"
        );
    }

    #[tokio::test]
    async fn a_signed_out_host_is_never_asked_for_a_role() {
        // R4. The mock asserts zero calls: `mockall`'s default range is satisfied at
        // zero, so `.times(0)` is the assertion, not the absence of one.
        let m = mock_that_must_not_be_asked();
        let roles = RoleCache::default();
        let host = refresh_account_for(&m, &roles, "solo.registry.io", false).await;

        assert!(!host.signed_in);
        assert!(!host.provisional);
        assert_eq!(host.current_role, None);
    }

    #[tokio::test]
    async fn an_unparseable_host_settles_the_row_without_a_query() {
        // Ruling P2: `Host::from_str` is fallible. A host string this layer cannot
        // parse gets the same settlement R5 gives a failed query — "we asked and
        // could not tell" — rather than failing the whole command. The mock proves
        // no query was even attempted: parsing fails before `roles.get` runs.
        let m = mock_that_must_not_be_asked();
        let roles = RoleCache::default();
        let host = refresh_account_for(&m, &roles, "[::1", true).await;

        assert!(host.signed_in, "the session on disk still stands");
        assert_eq!(host.current_role, None);
        assert_eq!(host.roles, [] as [String; 0]);
        assert!(
            !host.provisional,
            "we asked and could not tell — that is settled"
        );
    }

    #[test]
    fn a_settled_account_serializes_the_wire_shape() {
        let host = AccountHost {
            host: "open.quiltdata.com".to_string(),
            signed_in: true,
            current_role: Some("analyst".to_string()),
            roles: vec!["analyst".to_string(), "admin".to_string()],
            provisional: false,
        };
        assert_eq!(
            serde_json::to_value(&host).unwrap(),
            serde_json::json!({
                "host": "open.quiltdata.com",
                "signedIn": true,
                "currentRole": "analyst",
                "roles": ["analyst", "admin"],
                "provisional": false
            })
        );
    }

    /// A lineage carrying only the timestamps `last_changed` reads.
    fn lineage_with(commit_ms: Option<i64>, path_ms: &[i64]) -> quilt::lineage::PackageLineage {
        let mut lineage = quilt::lineage::PackageLineage::default();
        lineage.commit = commit_ms.map(|ms| quilt::lineage::CommitState {
            timestamp: chrono::DateTime::from_timestamp_millis(ms).unwrap(),
            hash: String::new(),
            prev_hashes: Vec::new(),
        });
        for (i, ms) in path_ms.iter().enumerate() {
            lineage.paths.insert(
                std::path::PathBuf::from(format!("f{i}.csv")),
                quilt::lineage::PathState {
                    timestamp: chrono::DateTime::from_timestamp_millis(*ms).unwrap(),
                    // `Multihash` by name would mean taking the `multihash` crate as a
                    // direct dependency of this one — it is not, and `quilt-rs` does not
                    // re-export it — purely to spell a test fixture's zero value.
                    #[allow(
                        clippy::default_trait_access,
                        reason = "the type is not nameable here without a new dependency"
                    )]
                    hash: Default::default(),
                },
            );
        }
        lineage
    }

    #[test]
    fn last_changed_takes_the_newest_path_when_it_beats_the_commit() {
        let l = lineage_with(Some(1_000), &[5_000, 3_000]);
        assert_eq!(last_changed(&l), Some(5_000.0));
    }

    #[test]
    fn last_changed_takes_the_commit_when_it_beats_every_path() {
        let l = lineage_with(Some(9_000), &[5_000, 3_000]);
        assert_eq!(last_changed(&l), Some(9_000.0));
    }

    #[test]
    fn last_changed_is_none_only_when_nothing_has_ever_been_written() {
        // No commit and no installed paths — a real answer, not a gap.
        assert_eq!(last_changed(&lineage_with(None, &[])), None);
        // Paths but no commit still has an answer.
        assert_eq!(last_changed(&lineage_with(None, &[7_000])), Some(7_000.0));
    }

    #[test]
    fn the_wire_shape_is_a_discriminator_and_carries_no_words() {
        let json = serde_json::to_string(&PackageStateDto::Diverged).unwrap();
        assert_eq!(json, r#"{"kind":"diverged"}"#);
        assert!(
            !json.contains("Changed in both places"),
            "§2: the wire carries a discriminator, never prose"
        );
    }

    #[test]
    fn pending_changes_carries_the_count_because_the_ui_cannot_measure_it() {
        let json = serde_json::to_string(&PackageStateDto::PendingChanges { files: 2 }).unwrap();
        assert_eq!(json, r#"{"kind":"pending_changes","files":2}"#);
    }

    #[test]
    fn an_unnameable_role_crosses_the_wire_as_null_not_as_an_empty_name() {
        assert_eq!(
            serde_json::to_string(&PackageStateDto::RoleDenied { role: None }).unwrap(),
            r#"{"kind":"role_denied","role":null}"#
        );
        assert_eq!(
            serde_json::to_string(&PackageStateDto::RoleDenied {
                role: Some("ReadOnly".to_string())
            })
            .unwrap(),
            r#"{"kind":"role_denied","role":"ReadOnly"}"#
        );
    }

    /// Drives `get_main_page_packages_from_model`'s real loop over a mock model —
    /// the paused map, the per-package lineage read, and the warn-and-skip on a
    /// failed lineage — and pins the result's serialized JSON. That JSON is the
    /// contract the UI's `MainPagePackageData` must deserialize (`kit/package_state.rs`),
    /// so this one test buys both the loop's integration coverage and the wire-shape
    /// pin the final review asked for, rather than two separate tests.
    ///
    /// The access pass runs here too — `readable_buckets` fails, which degrades
    /// to reactive-only marking and marks nothing, so the unchanged `"latest"`
    /// state and null `roleSwitchHost` below are what that degrade asserts.
    #[tokio::test]
    async fn get_main_page_packages_from_model_serializes_the_wire_shape() {
        let mut model = crate::model::mocks::create();

        let pkgs = vec![
            make_installed_package(("team", "latest")),
            make_installed_package(("team", "broken")),
        ];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let ns = pkg.namespace.to_string();
                if ns == "team/broken" {
                    // A package whose lineage load fails must be skipped, not
                    // fail the whole list — asserted below via `packages.len()`.
                    return Err(access_denied_error());
                }
                Ok(quilt::lineage::PackageLineage::from_remote(
                    make_manifest_uri(&ns),
                    "abcdef".to_string(),
                ))
            });
        model
            .expect_readable_buckets()
            .returning(|_| Err(Error::General("no bucket list in this test".to_string())));

        // No pause, so the row keeps its own `latest` and stays provisional. That
        // is the shape nearly every row has. The rule that some pauses leave the
        // state alone is tested by
        // `the_three_duplicate_pauses_do_not_fold_into_a_state`.
        let paused_reasons = HashMap::new();

        let result = get_main_page_packages_from_model(
            &model,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &paused_reasons,
        )
        .await
        .expect("one bad lineage must not fail the whole list");

        assert_eq!(
            result.packages.len(),
            1,
            "team/broken's lineage failure must be skipped, not surfaced"
        );

        let json = serde_json::to_value(&result).unwrap();
        assert_eq!(
            json,
            serde_json::json!({
                "packages": [{
                    "namespace": "team/latest",
                    "state": {"kind": "latest"},
                    "changedAt": null,
                    "bucket": "test",
                    "host": "test.quilt.dev",
                    "provisional": true,
                    "roleSwitchHost": null,
                }]
            }),
            "this shape is what the UI's MainPagePackageData must deserialize"
        );
    }

    /// A one-package roster whose lineage is a clean remote on a bucket the role
    /// can read: `PackageState::resolve` alone answers `Latest` (asserted by
    /// `other_and_the_three_duplicates_do_not_fold_into_a_state`, which drives
    /// this same fixture), and the access pass marks nothing. So any other state
    /// a test below sees came from the fold and nowhere else.
    fn mock_clean_roster() -> crate::model::MockQuiltModel {
        let mut model = crate::model::mocks::create();
        model
            .expect_get_installed_packages_list()
            .return_once(|| Ok(vec![make_installed_package(("team", "one"))]));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    make_manifest_uri(&pkg.namespace.to_string()),
                    "abcdef".to_string(),
                ))
            });
        // `make_manifest_uri`'s bucket, so the intersection is full and the access
        // pass has nothing to overwrite the fold with. `.times(1)`: without it the
        // expectation is satisfied at zero calls and would prove nothing about the
        // pass having run at all.
        model
            .expect_readable_buckets()
            .times(1)
            .returning(|_| Ok(vec!["test".to_string()]));
        model
    }

    /// The light phase over [`mock_clean_roster`], with its one package paused.
    async fn light_phase_state(
        m: &impl model::QuiltModel,
        paused: PausedReason,
    ) -> PackageStateDto {
        let paused_reasons = HashMap::from([(
            quilt_uri::Namespace::try_from("team/one").expect("a valid fixture namespace"),
            paused,
        )]);
        let packages = get_main_page_packages_from_model(
            m,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &paused_reasons,
        )
        .await
        .expect("list")
        .packages;
        row(&packages, "team/one").state.clone()
    }

    /// The light phase over [`mock_clean_roster`], returning the whole row.
    async fn light_phase_row(
        m: &impl model::QuiltModel,
        paused: Option<PausedReason>,
    ) -> MainPagePackage {
        let paused_reasons = paused
            .map(|paused| {
                HashMap::from([(
                    quilt_uri::Namespace::try_from("team/one").expect("a valid fixture namespace"),
                    paused,
                )])
            })
            .unwrap_or_default();
        let packages = get_main_page_packages_from_model(
            m,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &paused_reasons,
        )
        .await
        .expect("list")
        .packages;
        row(&packages, "team/one").clone()
    }

    #[tokio::test]
    async fn a_pull_conflict_arrives_settled_and_a_cached_state_provisional() {
        // `provisional` means "from cached lineage, unconfirmed by the remote". A
        // pull conflict is not that: it comes from the watcher's paused map, so it
        // is a fact about this disk that no network call can confirm or deny.
        //
        // The queue drops provisional rows, to keep the access pre-filter's
        // guesses out of it. A conflict marked provisional would leave the queue
        // whenever the remote is unreachable, which is when syncing cannot fix it.
        //
        // The second half checks that a cached state is still provisional, so the
        // test cannot pass by never marking anything provisional.
        let conflicted = light_phase_row(
            &mock_clean_roster(),
            Some(PausedReason::PullConflict(vec!["a.csv".to_string()])),
        )
        .await;
        assert!(
            !conflicted.provisional,
            "a conflict on disk owes the network nothing"
        );

        let clean = light_phase_row(&mock_clean_roster(), None).await;
        assert!(
            clean.provisional,
            "a cached state is still a guess until the heavy phase answers"
        );
    }

    #[tokio::test]
    async fn a_pull_conflict_pause_resolves_the_row_to_pull_conflict() {
        // Lattice rank 2. `PackageState::resolve` maps an `UpstreamState` and a pull
        // conflict is not one, which is why `PackageStateDto::PullConflict` has
        // been declared in both crates and constructed nowhere since Plan 1.
        let m = mock_clean_roster();
        let files = vec!["a.csv".to_string(), "b.csv".to_string()];

        assert_eq!(
            light_phase_state(&m, PausedReason::PullConflict(files.clone())).await,
            PackageStateDto::PullConflict { files },
            "both paths, in the order the pause carried them"
        );
    }

    #[tokio::test]
    async fn the_three_duplicate_pauses_do_not_fold_into_a_state() {
        // The tree's own state already says what these say, and two fields carrying
        // one fact is §1's rule at payload scale. `Other` is the exception and now
        // folds — see the test below.
        for reason in [
            PausedReason::PendingChanges,
            PausedReason::PendingCommit,
            PausedReason::Diverged,
        ] {
            let m = mock_clean_roster();
            assert_eq!(
                light_phase_state(&m, reason.clone()).await,
                PackageStateDto::Latest,
                "{reason:?} must leave the row's own state alone"
            );
        }
    }

    #[tokio::test]
    async fn the_heavy_phase_keeps_an_unexplained_pause_rather_than_measuring_past_it() {
        // The light phase resolves an `Other` pause to `Paused`, and the heavy
        // phase must do the same. Without a pause check in the heavy phase,
        // `PackageState::resolve` measures the working tree, and the row shows
        // `Sync paused` only until the first refresh answers, then `1 file
        // changed`.
        //
        // The fixture's tree has a change on purpose, matching the case seen on
        // the running app, where `PackageState::resolve` reports `1 file
        // changed`. `refresh_with_pause`'s doc states the rule: the two phases
        // must reach the same answer.
        let m = mock_one_package(Ok(status_with(UpstreamState::UpToDate, 1)), None);
        let refreshed = refresh_with_pause(
            &m,
            &RoleCache::default(),
            Some(&PausedReason::Other(
                "workflow rejected metadata".to_string(),
            )),
        )
        .await;

        assert_eq!(
            refreshed.state,
            PackageStateDto::Paused,
            "the heavy phase must not measure past a pause"
        );
    }

    #[tokio::test]
    async fn an_unexplained_pause_folds_into_a_state_of_its_own() {
        // A package that is latest by hash and paused by a workflow rejection.
        // If it resolved to `Latest`, `derive_queue` would leave it out, and since
        // `Other` does not clear by itself, the queue would never show it.
        //
        // It must not be `Unknown` either: `Unknown` claims the upstream state
        // could not be read, but here it was read and the sync stopped.
        let m = mock_clean_roster();
        let state = light_phase_state(
            &m,
            PausedReason::Other("workflow rejected metadata".to_string()),
        )
        .await;

        assert_eq!(state, PackageStateDto::Paused);
        assert_ne!(
            state,
            PackageStateDto::Unknown,
            "an unread state and a stopped sync are different claims"
        );
    }

    #[tokio::test]
    async fn an_unexplained_pause_arrives_settled_like_a_conflict() {
        // Both come from the watcher's paused map rather than from cached lineage,
        // so neither owes the network anything. It matters because the queue drops
        // provisional rows (R2): marked a guess, a pause would be dropped again and
        // stay as invisible as it was before it had a state at all.
        let row = light_phase_row(
            &mock_clean_roster(),
            Some(PausedReason::Other(
                "workflow rejected metadata".to_string(),
            )),
        )
        .await;

        assert_eq!(row.state, PackageStateDto::Paused);
        assert!(!row.provisional, "a pause on disk owes the network nothing");
    }

    #[test]
    fn an_unexplained_pause_crosses_the_wire_as_a_discriminator() {
        let json = serde_json::to_string(&PackageStateDto::Paused).unwrap();
        assert_eq!(json, r#"{"kind":"paused"}"#);
    }

    #[tokio::test]
    async fn the_package_payload_carries_no_prose() {
        // §2. `paused_reason` was prose on the wire, shipped by Plan 1 and logged by
        // plan 2's final review as this plan's to fix. The pause now travels typed on
        // the watcher payload, where §8 decision 3 puts it; carrying it on both is
        // two payloads resolving the same fact, which is what §1 forbids. A pause
        // with a message is the one that used to reach the wire verbatim, so it is
        // the fixture that can catch a regression.
        let m = mock_clean_roster();
        let paused_reasons: HashMap<quilt_uri::Namespace, PausedReason> = HashMap::from([(
            quilt_uri::Namespace::try_from("team/one").expect("a valid fixture namespace"),
            PausedReason::Other("workflow rejected metadata".to_string()),
        )]);
        let payload = get_main_page_packages_from_model(
            &m,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &paused_reasons,
        )
        .await
        .expect("list");

        let json = serde_json::to_string(&payload).unwrap();
        assert!(!json.contains("pausedReason"), "{json}");
        assert!(!json.contains("pausedKind"), "{json}");
        assert!(
            !json.contains("workflow rejected metadata"),
            "the words live in `kit::render`, not on this payload: {json}"
        );
    }

    /// A `ManifestUri` in an explicit bucket, so two rows can sit on one host in
    /// different buckets — the shape the readable-bucket intersection is about.
    fn make_manifest_uri_in_bucket(bucket: &str, namespace: &str) -> quilt_uri::ManifestUri {
        quilt_uri::ManifestUri {
            origin: Some("test.quilt.dev".parse().unwrap()),
            bucket: bucket.to_string(),
            namespace: namespace.try_into().unwrap(),
            hash: "abcdef".to_string(),
        }
    }

    /// A user holding two roles, the second of which is active.
    fn two_roles() -> RoleInfo {
        RoleInfo {
            current: "ReadOnly".to_string(),
            available: vec!["ReadWrite".to_string(), "ReadOnly".to_string()],
        }
    }

    /// A roster of two packages on one host — `team/open` in `reachable`,
    /// `team/locked` in `locked` — with the role's readable set and role list
    /// under the test's control.
    fn mock_two_bucket_roster(
        readable: Result<Vec<String>, ()>,
        roles: Option<RoleInfo>,
    ) -> crate::model::MockQuiltModel {
        let mut model = crate::model::mocks::create();
        let pkgs = vec![
            make_installed_package(("team", "open")),
            make_installed_package(("team", "locked")),
        ];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let ns = pkg.namespace.to_string();
                let bucket = if ns == "team/locked" {
                    "locked"
                } else {
                    "reachable"
                };
                Ok(quilt::lineage::PackageLineage::from_remote(
                    make_manifest_uri_in_bucket(bucket, &ns),
                    "abcdef".to_string(),
                ))
            });
        // `.times(1)`: one query per HOST, not per package — judgement 1. Both
        // rows in this roster share one host, so a per-row implementation would
        // call this twice and mockall would fail the test.
        model
            .expect_readable_buckets()
            .times(1)
            .returning(move |_| {
                readable
                    .clone()
                    .map_err(|()| Error::General("bucket list unavailable".to_string()))
            });
        if let Some(roles) = roles {
            model
                .expect_refresh_roles()
                .returning(move |_| Ok(roles.clone()));
        } else {
            model
                .expect_refresh_roles()
                .returning(|_| Err(Error::General("role query unavailable".to_string())));
        }
        // Reading the role can expire the stored credentials, so the cache finishes
        // the flush by dropping the clients. See `RoleCache::get`.
        model.expect_clear_remote_client_cache().returning(|_| ());
        model
    }

    async fn roster(m: &impl model::QuiltModel, roles: &RoleCache) -> Vec<MainPagePackage> {
        get_main_page_packages_from_model(
            m,
            roles,
            &crate::telemetry::Telemetry::default(),
            &HashMap::new(),
        )
        .await
        .expect("list")
        .packages
    }

    fn row<'a>(rows: &'a [MainPagePackage], namespace: &str) -> &'a MainPagePackage {
        rows.iter()
            .find(|p| p.namespace.to_string() == namespace)
            .expect("row present")
    }

    #[tokio::test]
    async fn a_bucket_outside_the_readable_set_resolves_to_a_denial() {
        let m = mock_two_bucket_roster(Ok(vec!["reachable".to_string()]), Some(two_roles()));
        let rows = roster(&m, &RoleCache::default()).await;

        assert_eq!(
            row(&rows, "team/locked").state,
            PackageStateDto::RoleDenied {
                role: Some("ReadOnly".to_string())
            }
        );
        assert_eq!(
            row(&rows, "team/locked").role_switch_host.as_deref(),
            Some("test.quilt.dev"),
            "the user holds a second role, so the switch is not a dead end"
        );
        assert_ne!(
            row(&rows, "team/open").state,
            PackageStateDto::RoleDenied {
                role: Some("ReadOnly".to_string())
            },
            "a readable bucket must not be greyed"
        );
    }

    #[tokio::test]
    async fn a_failed_bucket_query_denies_nothing() {
        // A failed query is not evidence that nothing is readable. Treating it as
        // an empty set would grey every row; the only safe degrade is reactive-only
        // marking, where the per-row status call marks what is really denied.
        let m = mock_two_bucket_roster(Err(()), Some(two_roles()));
        let rows = roster(&m, &RoleCache::default()).await;

        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|p| !matches!(p.state, PackageStateDto::RoleDenied { .. })),
            "an unanswerable bucket query must not grey the whole roster"
        );
    }

    #[tokio::test]
    async fn a_denial_whose_role_query_failed_is_still_a_denial() {
        let m = mock_two_bucket_roster(Ok(vec!["reachable".to_string()]), None);
        let rows = roster(&m, &RoleCache::default()).await;

        assert_eq!(
            row(&rows, "team/locked").state,
            PackageStateDto::RoleDenied { role: None },
            "the bucket refused; that is known whether or not the role can be named"
        );
        assert_eq!(
            row(&rows, "team/locked").role_switch_host,
            None,
            "no switch affordance: we cannot tell whether another role is held"
        );
    }

    #[tokio::test]
    async fn a_single_role_user_is_offered_no_switch() {
        let solo = RoleInfo {
            current: "ReadOnly".to_string(),
            available: vec!["ReadOnly".to_string()],
        };
        let m = mock_two_bucket_roster(Ok(vec!["reachable".to_string()]), Some(solo));
        let rows = roster(&m, &RoleCache::default()).await;

        assert_eq!(
            row(&rows, "team/locked").state,
            PackageStateDto::RoleDenied {
                role: Some("ReadOnly".to_string())
            }
        );
        assert_eq!(
            row(&rows, "team/locked").role_switch_host,
            None,
            "a switch affordance that leads nowhere is worse than none"
        );
    }

    /// A host that accepts the connection and then says nothing — the
    /// captive-portal shape, which a connect timeout does not cover.
    ///
    /// Hand-written rather than mocked: a mockall expectation resolves
    /// synchronously, so it cannot model a call that hangs.
    struct SilentHost {
        domain: tokio::sync::Mutex<quilt::LocalDomain>,
    }

    impl Default for SilentHost {
        fn default() -> Self {
            Self {
                domain: tokio::sync::Mutex::new(quilt::LocalDomain::new(std::path::PathBuf::new())),
            }
        }
    }

    #[allow(
        clippy::unused_async_trait_impl,
        reason = "`readable_buckets` awaits; the other two do not. Rewriting just those would leave one impl split between `async fn` and `fn -> impl Future`, which reads worse than either consistent choice."
    )]
    impl model::QuiltModel for SilentHost {
        type Locked = quilt::LockedPackage;

        fn get_quilt(&self) -> &tokio::sync::Mutex<quilt::LocalDomain> {
            &self.domain
        }

        async fn get_installed_packages_list(&self) -> Result<Vec<quilt::InstalledPackage>, Error> {
            Ok(vec![
                make_installed_package(("team", "open")),
                make_installed_package(("team", "locked")),
            ])
        }

        async fn get_installed_package_lineage(
            &self,
            package: &quilt::InstalledPackage,
        ) -> Result<quilt::lineage::PackageLineage, Error> {
            let ns = package.namespace.to_string();
            Ok(quilt::lineage::PackageLineage::from_remote(
                make_manifest_uri_in_bucket("locked", &ns),
                "abcdef".to_string(),
            ))
        }

        async fn readable_buckets(&self, _host: &Host) -> Result<Vec<String>, Error> {
            // Absolute, not a multiple of `BUCKET_LIST_BUDGET`: scaling with the
            // production constant would make the elapsed assertion below hold no
            // matter what the constant is, which proves nothing about the timeout.
            // Far beyond any budget the roster could reasonably wait.
            tokio::time::sleep(Duration::from_secs(200)).await;
            Ok(Vec::new())
        }
    }

    #[tokio::test(start_paused = true)]
    async fn an_unanswerable_host_does_not_hold_the_roster() {
        let m = SilentHost::default();
        let started = tokio::time::Instant::now();
        let rows = roster(&m, &RoleCache::default()).await;

        assert_eq!(rows.len(), 2, "the roster paints from local data");
        assert!(
            rows.iter()
                .all(|p| !matches!(p.state, PackageStateDto::RoleDenied { .. })),
            "a query that never answered is not evidence of denial"
        );
        assert!(
            started.elapsed() <= BUCKET_LIST_BUDGET,
            "the roster waited {:?} on a host that never answers",
            started.elapsed(),
        );
    }

    /// A host that answers the readable-bucket query and then says nothing to
    /// `/me` — the half of the same shape `SilentHost` does not cover, and the
    /// one that reaches the role query behind a denial.
    ///
    /// Hand-written for `SilentHost`'s reason: a mockall expectation resolves
    /// synchronously, so it cannot model a call that hangs.
    struct SilentRole {
        domain: tokio::sync::Mutex<quilt::LocalDomain>,
    }

    impl Default for SilentRole {
        fn default() -> Self {
            Self {
                domain: tokio::sync::Mutex::new(quilt::LocalDomain::new(std::path::PathBuf::new())),
            }
        }
    }

    #[allow(
        clippy::unused_async_trait_impl,
        reason = "`refresh_roles` awaits; the others do not — see `SilentHost`."
    )]
    impl model::QuiltModel for SilentRole {
        type Locked = quilt::LockedPackage;

        fn get_quilt(&self) -> &tokio::sync::Mutex<quilt::LocalDomain> {
            &self.domain
        }

        async fn get_installed_packages_list(&self) -> Result<Vec<quilt::InstalledPackage>, Error> {
            Ok(vec![make_installed_package(("team", "locked"))])
        }

        async fn get_installed_package_lineage(
            &self,
            package: &quilt::InstalledPackage,
        ) -> Result<quilt::lineage::PackageLineage, Error> {
            let ns = package.namespace.to_string();
            Ok(quilt::lineage::PackageLineage::from_remote(
                make_manifest_uri_in_bucket("locked", &ns),
                "abcdef".to_string(),
            ))
        }

        async fn readable_buckets(&self, _host: &Host) -> Result<Vec<String>, Error> {
            // Answers at once, and does not name `locked` — so the row is
            // denied and the role query behind the wording is reached.
            Ok(vec!["reachable".to_string()])
        }

        async fn refresh_roles(&self, _host: &Host) -> Result<quilt::auth::RoleInfo, Error> {
            // Absolute, for the reason `SilentHost::readable_buckets` gives.
            tokio::time::sleep(Duration::from_secs(200)).await;
            unreachable!("the budget above cuts this off")
        }
    }

    #[tokio::test(start_paused = true)]
    async fn a_role_query_that_never_answers_does_not_hold_the_roster() {
        // `BUCKET_LIST_BUDGET` covers the role query as well as the bucket
        // query. Without that, a host that answers the bucket query and then
        // hangs on `/me` keeps the main screen blank for as long as the HTTP
        // stack allows.
        //
        // The result is `RoleDenied` with no role. The bucket refused, so the
        // denial is certain; only the role name is missing.
        let m = SilentRole::default();
        let started = tokio::time::Instant::now();
        let rows = roster(&m, &RoleCache::default()).await;

        assert_eq!(
            row(&rows, "team/locked").state,
            PackageStateDto::RoleDenied { role: None },
            "the denial stands; the role query is what failed"
        );
        assert!(
            started.elapsed() <= BUCKET_LIST_BUDGET,
            "the roster waited {:?} on a role query that never answers",
            started.elapsed(),
        );
    }

    #[tokio::test]
    async fn the_role_behind_a_denial_is_fetched_once_per_host() {
        // Two denied rows on one host. `RoleCache` is why this is one `/me` and not
        // one per row — the pile-up it exists to prevent.
        let mut model = crate::model::mocks::create();
        let pkgs = vec![
            make_installed_package(("team", "one")),
            make_installed_package(("team", "two")),
        ];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    make_manifest_uri_in_bucket("locked", &pkg.namespace.to_string()),
                    "abcdef".to_string(),
                ))
            });
        model
            .expect_readable_buckets()
            .returning(|_| Ok(vec!["elsewhere".to_string()]));
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        model.expect_refresh_roles().returning(move |_| {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(two_roles())
        });
        model.expect_clear_remote_client_cache().returning(|_| ());

        let rows = roster(&model, &RoleCache::default()).await;

        assert_eq!(rows.len(), 2);
        assert!(
            rows.iter()
                .all(|p| matches!(p.state, PackageStateDto::RoleDenied { .. }))
        );
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "one role query per host per load, however many rows deny"
        );
    }

    /// COPIED judgement 4's own regression guard: `roles.invalidate(None)` runs
    /// at the top of every load. Without it, a `RoleCache` shared across two
    /// loads would serve the first load's cached role forever, and
    /// `refresh_roles` would be called once rather than once per load.
    ///
    /// v1's analogue is `package_list.rs:1126-1132`; this is v2's own version,
    /// against v2's own `roster` helper.
    #[tokio::test]
    async fn a_second_load_re_fetches_the_role_rather_than_serving_the_cache() {
        let mut model = crate::model::mocks::create();
        model.expect_get_installed_packages_list().returning(|| {
            Ok(vec![
                make_installed_package(("team", "open")),
                make_installed_package(("team", "locked")),
            ])
        });
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let ns = pkg.namespace.to_string();
                let bucket = if ns == "team/locked" {
                    "locked"
                } else {
                    "reachable"
                };
                Ok(quilt::lineage::PackageLineage::from_remote(
                    make_manifest_uri_in_bucket(bucket, &ns),
                    "abcdef".to_string(),
                ))
            });
        model
            .expect_readable_buckets()
            .returning(|_| Ok(vec!["reachable".to_string()]));
        let calls = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let counted = calls.clone();
        model.expect_refresh_roles().returning(move |_| {
            counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            Ok(two_roles())
        });
        model.expect_clear_remote_client_cache().returning(|_| ());

        let roles = RoleCache::default();
        roster(&model, &roles).await;
        roster(&model, &roles).await;

        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            2,
            "roles.invalidate(None) must run at the top of every load, or the second load would serve the first load's cached role name"
        );
    }

    /// A single package on `test.quilt.dev` whose status call answers with `status`,
    /// with the host's roles under the test's control.
    fn mock_one_package(
        status: Result<quilt::lineage::InstalledPackageStatus, Error>,
        roles: Option<RoleInfo>,
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
        // `return_once`, not `returning`: `Error` is not `Clone`, and each of these
        // tests makes exactly one status call. This is the idiom `model/mocks.rs`
        // already uses for this same method. `.times(1)` makes that "exactly one"
        // an assertion rather than a description: without it, mockall's default
        // `TimesRange` is satisfied at zero calls too, so a caller that skipped
        // the status call entirely would pass silently.
        model
            .expect_get_installed_package_status()
            .times(1)
            .return_once(move |_, _| status);
        if let Some(roles) = roles {
            model
                .expect_refresh_roles()
                .returning(move |_| Ok(roles.clone()));
        }
        model.expect_clear_remote_client_cache().returning(|_| ());
        model
    }

    fn status_with(
        upstream: UpstreamState,
        changed_files: usize,
    ) -> quilt::lineage::InstalledPackageStatus {
        let mut changes = quilt::lineage::ChangeSet::new();
        for i in 0..changed_files {
            changes.insert(
                std::path::PathBuf::from(format!("f{i}.csv")),
                quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
            );
        }
        quilt::lineage::InstalledPackageStatus::new(upstream, changes)
    }

    async fn refresh(m: &impl model::QuiltModel, roles: &RoleCache) -> MainPagePackageRefresh {
        refresh_with_pause(m, roles, None).await
    }

    /// The heavy phase with a pause in hand — the second half of the fold, which
    /// must reach the same answer as the light phase's.
    async fn refresh_with_pause(
        m: &impl model::QuiltModel,
        roles: &RoleCache,
        paused: Option<&PausedReason>,
    ) -> MainPagePackageRefresh {
        let ns: quilt_uri::Namespace = "team/one".try_into().unwrap();
        refresh_main_page_package_from_model(
            m,
            roles,
            &crate::telemetry::Telemetry::default(),
            &ns,
            paused,
        )
        .await
        .expect("refresh")
    }

    #[tokio::test]
    async fn a_measured_working_tree_reports_how_many_files_changed() {
        // The one count the wire carries, because the UI has no collection to measure.
        let m = mock_one_package(Ok(status_with(UpstreamState::UpToDate, 3)), None);
        assert_eq!(
            refresh(&m, &RoleCache::default()).await.state,
            PackageStateDto::PendingChanges { files: 3 }
        );
    }

    #[tokio::test]
    async fn a_clean_tree_that_is_up_to_date_is_latest() {
        let m = mock_one_package(Ok(status_with(UpstreamState::UpToDate, 0)), None);
        assert_eq!(
            refresh(&m, &RoleCache::default()).await.state,
            PackageStateDto::Latest
        );
    }

    #[tokio::test]
    async fn the_heavy_phase_reaches_the_same_answer_as_the_light_one() {
        // The two phases differ in what they MEASURE, never in how they decide. A
        // fold in one and not the other is a row that flips from "conflicts in 2
        // files" to "Latest" as it settles. This is the same conflict, and the same
        // expectation, as `a_pull_conflict_pause_resolves_the_row_to_pull_conflict`
        // — over a status call that `a_clean_tree_that_is_up_to_date_is_latest`
        // shows resolves to `Latest` on its own.
        let m = mock_one_package(Ok(status_with(UpstreamState::UpToDate, 0)), None);
        let files = vec!["a.csv".to_string(), "b.csv".to_string()];

        let refreshed = refresh_with_pause(
            &m,
            &RoleCache::default(),
            Some(&PausedReason::PullConflict(files.clone())),
        )
        .await;

        assert_eq!(refreshed.state, PackageStateDto::PullConflict { files });
    }

    #[tokio::test]
    async fn a_denial_outranks_a_pull_conflict_pause() {
        // The named invariant `a_denial_outranks_a_pause_even_a_pull_conflict`, from
        // v1's `installed_packages_list.rs`. Denial is rank 1 and the conflict is
        // rank 2, so the fold must sit AFTER the denial arm has had its chance — an
        // early return at the top of the heavy phase would silently invert this.
        let m = mock_one_package(Err(access_denied_error()), Some(two_roles()));

        let refreshed = refresh_with_pause(
            &m,
            &RoleCache::default(),
            Some(&PausedReason::PullConflict(vec!["a.csv".to_string()])),
        )
        .await;

        assert_eq!(
            refreshed.state,
            PackageStateDto::RoleDenied {
                role: Some("ReadOnly".to_string())
            },
            "rank 1 outranks rank 2"
        );
    }

    #[tokio::test]
    async fn a_denial_resolves_the_row_to_no_access_never_to_an_error() {
        // Not an auth failure: credential vending succeeded and the active role
        // simply cannot reach this bucket. Resolving to `Unknown` here — whose words
        // are "Sync stopped" — is what sent the original bug reporter into an
        // unrecoverable re-login loop, because the re-vend hands back the same role.
        let m = mock_one_package(Err(access_denied_error()), Some(two_roles()));
        let refreshed = refresh(&m, &RoleCache::default()).await;

        assert_eq!(
            refreshed.state,
            PackageStateDto::RoleDenied {
                role: Some("ReadOnly".to_string())
            }
        );
        assert_eq!(
            refreshed.role_switch_host.as_deref(),
            Some("test.quilt.dev")
        );
    }

    #[tokio::test]
    async fn a_generic_status_failure_is_propagated_not_a_denial() {
        // A failure to reach the remote is not evidence about the package. Manufacturing
        // `Unknown` here would overwrite the light phase's cached-correct guess with an
        // assertion the call never earned, so the command surfaces the error instead and
        // `PackageListRow`'s `Err` arm keeps the row provisional. It is still not a
        // denial: a generic failure must not be mistaken for one and routed through the
        // access-denied arm's role-naming path.
        let m = mock_one_package(Err(Error::General("network down".to_string())), None);
        let ns: quilt_uri::Namespace = "team/one".try_into().unwrap();

        let err = refresh_main_page_package_from_model(
            &m,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &ns,
            None,
        )
        .await
        .expect_err("a generic status failure must surface as an error, not a manufactured state");

        assert!(
            !err.is_access_denied(),
            "a generic failure must not take the denial path"
        );
    }

    #[tokio::test]
    async fn a_behind_result_passes_through_the_resolver_and_clears_the_switch_host() {
        // A `Behind` status is not a denial, so it maps straight through
        // `PackageState::resolve` like any other successful call, and `role_switch_host` is
        // unconditionally `None` on this arm regardless of what roles the caller
        // holds — the success arm never reads the pre-filter mark. This is a
        // `Behind` pass-through check, not proof that a row the light phase greyed
        // can come back: that needs an actual light-phase row plus a second call to
        // `apply`, which is Task 5's job.
        let m = mock_one_package(Ok(status_with(UpstreamState::Behind, 0)), Some(two_roles()));
        let refreshed = refresh(&m, &RoleCache::default()).await;

        assert_eq!(refreshed.state, PackageStateDto::Behind);
        assert_eq!(refreshed.role_switch_host, None);
    }

    #[tokio::test]
    async fn a_local_only_package_needs_no_status_call() {
        // No expectation is set for `get_installed_package_status`; mockall panics if
        // it is called. `Local` ignores the file count, so making the call would be
        // a local hash walk whose answer nothing reads.
        let mut model = crate::model::mocks::create();
        model
            .expect_get_installed_package()
            .returning(|ns| Ok(Some(make_installed_package(ns.clone()))));
        model
            .expect_get_installed_package_lineage()
            .returning(|_| Ok(quilt::lineage::PackageLineage::default()));

        assert_eq!(
            refresh(&model, &RoleCache::default()).await.state,
            PackageStateDto::NoRemote
        );
    }

    #[tokio::test]
    async fn a_remote_with_no_catalog_host_never_reaches_the_network() {
        // No expectation for `get_installed_package_status`: mockall panics if it is
        // called. There is nowhere to vend credentials from, so the call cannot
        // succeed — and the hash comparison behind it answers a question the app
        // cannot act on.
        let mut model = crate::model::mocks::create();
        model
            .expect_get_installed_package()
            .returning(|ns| Ok(Some(make_installed_package(ns.clone()))));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    make_manifest_uri_no_origin(&pkg.namespace.to_string()),
                    "abcdef".to_string(),
                ))
            });

        assert_eq!(
            refresh(&model, &RoleCache::default()).await.state,
            PackageStateDto::Unknown
        );
    }

    #[tokio::test]
    async fn a_package_that_is_not_installed_is_an_error_not_a_state() {
        let mut model = crate::model::mocks::create();
        model.expect_get_installed_package().returning(|_| Ok(None));
        let ns: quilt_uri::Namespace = "team/gone".try_into().unwrap();

        let err = refresh_main_page_package_from_model(
            &model,
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &ns,
            None,
        )
        .await
        .expect_err("an uninstalled package has no state to report");

        // Not just `is_err()`: a refactor that made the lineage lookup fail first
        // (unreachable today, since it is never called with no installed package,
        // but not enforced by the type system) would still return SOME error and
        // this test would no longer be testing what its name claims.
        assert!(
            matches!(
                err,
                Error::Quilt(quilt::Error::InstallPackage(
                    quilt::InstallPackageError::NotInstalled(_)
                ))
            ),
            "expected InstallPackageError::NotInstalled, got {err:?}"
        );
    }

    #[tokio::test]
    async fn a_reachable_but_unpublished_remote_still_makes_the_call() {
        // The mirror of `a_local_only_package_needs_no_status_call` and
        // `a_remote_with_no_catalog_host_never_reaches_the_network` above: THIS
        // remote has both a bucket and a catalog host, so it is reachable, and
        // `refresh_main_page_package_from_model`'s doc comment says the call still
        // happens — `Local` on an existing, reachable remote resolves to
        // `Unpublished`, not `NoRemote`. Without this test that claim survived only
        // as prose: nothing would catch a refactor that widened the skip condition
        // from `!has_remote` to "any `Local` upstream", which would silently stop
        // refreshing every package with a bucket nobody has pushed to yet.
        let m = mock_one_package(Ok(status_with(UpstreamState::Local, 0)), None);
        assert_eq!(
            refresh(&m, &RoleCache::default()).await.state,
            PackageStateDto::Unpublished
        );
    }

    #[test]
    fn an_armed_toggle_is_exactly_the_one_that_carries_a_deadline() {
        // The biconditional, over every input combination. §4.2 says `deadline` is
        // `armed` only; two fields that can disagree about the same fact is §1's
        // rule at payload scale, so this is derived once and asserted here.
        let at = Utc::now() + Duration::from_secs(30);
        let interval = Duration::from_secs(30);
        for (enabled, any_paused, deadline, expected) in [
            (true, false, Some(at), ToggleActivity::Armed),
            (true, false, None, ToggleActivity::Idle),
            (true, true, Some(at), ToggleActivity::Paused),
            (true, true, None, ToggleActivity::Paused),
            (false, false, Some(at), ToggleActivity::Idle),
            (false, true, Some(at), ToggleActivity::Idle),
        ] {
            let state = toggle_state(enabled, any_paused, deadline, interval);
            assert_eq!(
                state.activity,
                expected,
                "enabled={enabled} paused={any_paused} deadline={}",
                deadline.is_some()
            );
            assert_eq!(
                state.deadline.is_some(),
                expected == ToggleActivity::Armed,
                "a deadline crosses the wire exactly when the toggle is armed"
            );
        }
    }

    #[test]
    fn a_toggle_that_is_off_is_idle_not_paused() {
        // The loop arms `next_pull_at` regardless of settings — it sleeps whether or
        // not either direction is enabled — so an unguarded derivation would report
        // a disabled toggle as armed, counting down to a tick that does nothing.
        // And a setting that is off has no machinery to have stopped.
        let state = toggle_state(
            false,
            true,
            Some(Utc::now() + Duration::from_secs(30)),
            Duration::from_secs(30),
        );
        assert_eq!(state.activity, ToggleActivity::Idle);
        assert!(!state.enabled);
    }

    #[test]
    fn the_interval_crosses_as_milliseconds_because_that_is_what_the_ring_needs() {
        // `Countdown.interval` is "the whole wait, in milliseconds" — a determinate
        // ring cannot be drawn from a remaining time alone.
        let state = toggle_state(true, false, None, Duration::from_secs(300));
        assert!((state.interval_ms - 300_000.0).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn a_pause_stops_both_toggles_and_appears_once_in_the_list() {
        // R2: `run_once` skips a paused namespace ABOVE both the pull branch and the
        // publish branch, so a pause of any reason stops both directions. And the
        // paused list and both activities come from ONE read of the map — the
        // 2026-07-11 bug was two sources answering the same question, so this test
        // asserts they agree by construction rather than by coincidence.
        let facts = WatcherFacts {
            pull_enabled: true,
            publish_enabled: true,
            paused: vec![(
                ("team", "plate-07").into(),
                PausedReason::Other("workflow rejected metadata".to_string()),
            )],
            next_pull_at: Some(Utc::now() + Duration::from_secs(30)),
            publish_arm_at: None,
            pull_interval: Duration::from_secs(30),
            publish_interval: Duration::from_secs(300),
        };
        let payload = MainPageWatcher::from(facts);
        assert_eq!(payload.pull.activity, ToggleActivity::Paused);
        assert_eq!(payload.publish.activity, ToggleActivity::Paused);
        assert_eq!(payload.paused.len(), 1);
        assert_eq!(payload.paused[0].namespace.to_string(), "team/plate-07");
        assert_eq!(
            payload.paused[0].reason,
            PausedDto::Other {
                message: "workflow rejected metadata".to_string()
            }
        );
    }

    #[tokio::test]
    async fn the_watcher_payload_serializes_the_wire_shape() {
        // The contract Task 5's `MainPageWatcherData` must deserialize. Pinned as a
        // whole-payload literal, so a renamed field or a changed case fails here
        // rather than silently at the Tauri boundary.
        let facts = WatcherFacts {
            pull_enabled: true,
            publish_enabled: true,
            paused: vec![(
                ("team", "plate-07").into(),
                PausedReason::PullConflict(vec!["a.csv".to_string(), "b.csv".to_string()]),
            )],
            next_pull_at: Some(chrono::DateTime::from_timestamp_millis(1_754_500_030_000).unwrap()),
            publish_arm_at: None,
            pull_interval: Duration::from_secs(30),
            publish_interval: Duration::from_secs(300),
        };
        assert_eq!(
            serde_json::to_value(MainPageWatcher::from(facts)).unwrap(),
            serde_json::json!({
                "pull": {
                    "enabled": true,
                    "activity": "paused",
                    "deadline": null,
                    "intervalMs": 30_000.0
                },
                "publish": {
                    "enabled": true,
                    "activity": "paused",
                    "deadline": null,
                    "intervalMs": 300_000.0
                },
                "paused": [{
                    "namespace": "team/plate-07",
                    "reason": {"kind": "pull_conflict", "files": ["a.csv", "b.csv"]}
                }]
            })
        );
    }

    #[tokio::test]
    async fn an_armed_payload_carries_its_deadline_as_epoch_millis() {
        // The other half of the pin: the arm above is paused, so nothing in it
        // exercises the `armed` shape or the millisecond conversion.
        let at = chrono::DateTime::from_timestamp_millis(1_754_500_030_000).unwrap();
        let facts = WatcherFacts {
            pull_enabled: true,
            publish_enabled: false,
            paused: Vec::new(),
            next_pull_at: Some(at),
            publish_arm_at: None,
            pull_interval: Duration::from_secs(30),
            publish_interval: Duration::from_secs(300),
        };
        let json = serde_json::to_value(MainPageWatcher::from(facts)).unwrap();
        assert_eq!(json["pull"]["activity"], "armed");
        // `PartialEq<f64> for Value` coerces through `as_f64`, so the literal below
        // compares equal to an integer `Number` too. The UI's field is an `f64`;
        // this is what pins the form rather than only the value.
        assert!(
            json["pull"]["deadline"].is_f64(),
            "the deadline crosses as a JSON float, not an integer"
        );
        assert_eq!(json["pull"]["deadline"], 1_754_500_030_000.0);
        assert_eq!(json["publish"]["activity"], "idle");
        assert_eq!(json["publish"]["deadline"], serde_json::Value::Null);
    }

    // ── The recent files feed ──

    /// A lineage carrying named paths at explicit epoch-second timestamps —
    /// the variant [`lineage_with`] cannot express, because it names every
    /// path `f{i}.csv` and these tests need to assert on which name is
    /// newest. Seconds rather than `lineage_with`'s milliseconds: these values
    /// only ever round-trip through this fixture and `recent_files`'s own
    /// `epoch_millis` conversion, so the smaller unit keeps the test numbers
    /// readable.
    fn lineage_with_named_paths(paths: &[(&str, u64)]) -> quilt::lineage::PackageLineage {
        let mut lineage = quilt::lineage::PackageLineage::default();
        for (name, secs) in paths {
            lineage.paths.insert(
                std::path::PathBuf::from(name),
                quilt::lineage::PathState {
                    timestamp: chrono::DateTime::from_timestamp(
                        i64::try_from(*secs).expect("test timestamp fits in i64 seconds"),
                        0,
                    )
                    .expect("test timestamp is a valid instant"),
                    // `Multihash` by name would mean taking the `multihash` crate as a
                    // direct dependency of this one — see `lineage_with`'s identical note.
                    #[allow(
                        clippy::default_trait_access,
                        reason = "the type is not nameable here without a new dependency"
                    )]
                    hash: Default::default(),
                },
            );
        }
        lineage
    }

    /// A roster of installed packages, each with a lineage whose `paths` are
    /// exactly the given names at the given epoch-second timestamps. The
    /// companion to [`lineage_with_named_paths`] that also wires up
    /// `get_installed_packages_list`, matching the mock-building pattern
    /// `mock_clean_roster` and `mock_two_bucket_roster` use above.
    fn mock_with_lineages(packages: &[(&str, Vec<(&str, u64)>)]) -> crate::model::MockQuiltModel {
        let mut model = crate::model::mocks::create();

        let installed: Vec<quilt::InstalledPackage> = packages
            .iter()
            .map(|(namespace, _)| {
                let (prefix, name) = namespace
                    .split_once('/')
                    .expect("test namespace must be `prefix/name`");
                make_installed_package((prefix, name))
            })
            .collect();

        let lineages: HashMap<String, quilt::lineage::PackageLineage> = packages
            .iter()
            .map(|(namespace, paths)| ((*namespace).to_string(), lineage_with_named_paths(paths)))
            .collect();

        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(installed));
        model
            .expect_get_installed_package_lineage()
            .returning(move |pkg| {
                Ok(lineages
                    .get(&pkg.namespace.to_string())
                    .cloned()
                    .expect("lineage requested for a namespace the fixture did not set up"))
            });

        model
    }

    /// The same shape as [`mock_with_lineages`] for two packages, except
    /// `failing_namespace`'s lineage read errors — the rule [`recent_files`]
    /// shares with `load_rows`: warn and skip, don't fail the whole walk.
    fn mock_with_one_failing_lineage(
        failing_namespace: &str,
        good_namespace: &str,
        good_path: &str,
        good_secs: u64,
    ) -> crate::model::MockQuiltModel {
        let mut model = crate::model::mocks::create();

        let (failing_prefix, failing_name) = failing_namespace
            .split_once('/')
            .expect("test namespace must be `prefix/name`");
        let (good_prefix, good_name) = good_namespace
            .split_once('/')
            .expect("test namespace must be `prefix/name`");
        let installed = vec![
            make_installed_package((failing_prefix, failing_name)),
            make_installed_package((good_prefix, good_name)),
        ];

        let good_namespace = good_namespace.to_string();
        let good_lineage = lineage_with_named_paths(&[(good_path, good_secs)]);

        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(installed));
        model
            .expect_get_installed_package_lineage()
            .returning(move |pkg| {
                if pkg.namespace.to_string() == good_namespace {
                    Ok(good_lineage.clone())
                } else {
                    Err(access_denied_error())
                }
            });

        model
    }

    #[tokio::test]
    async fn recent_files_are_flat_and_newest_first_across_packages() {
        // §3.2's "Why flat": one recent file in the third package must outrank
        // three older files in the first. A per-package grouping would fail
        // exactly here.
        let m = mock_with_lineages(&[
            ("user/alpha", vec![("old.csv", 1_000), ("older.csv", 500)]),
            ("user/beta", vec![("newest.csv", 9_000)]),
        ]);

        let files = recent_files(&m).await.unwrap();

        let names: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(names, vec!["newest.csv", "old.csv", "older.csv"]);
        assert_eq!(
            files[0].namespace.to_string(),
            "user/beta",
            "the owning package travels"
        );
    }

    #[tokio::test]
    #[allow(
        clippy::cast_precision_loss,
        reason = "RECENT_FILES_LIMIT + 9 is 59, which is exact in f64"
    )]
    async fn recent_files_are_capped_at_the_limit_and_the_cap_keeps_the_newest() {
        // "Bounded and pre-selected by the backend" (§4.5). The cap must be
        // applied AFTER the global sort, or it would keep an arbitrary 50.
        let names: Vec<String> = (0..RECENT_FILES_LIMIT + 10)
            .map(|i| format!("f{i}.csv"))
            .collect();
        let many: Vec<(&str, u64)> = names
            .iter()
            .enumerate()
            .map(|(i, name)| (name.as_str(), i as u64))
            .collect();
        let m = mock_with_lineages(&[("user/alpha", many)]);

        let files = recent_files(&m).await.unwrap();

        assert_eq!(files.len(), RECENT_FILES_LIMIT);
        assert!(
            (files[0].changed_at - (RECENT_FILES_LIMIT + 9) as f64 * 1000.0).abs() < f64::EPSILON,
            "the cap kept the newest, not the first fifty walked"
        );
    }

    #[tokio::test]
    async fn a_package_with_no_installed_paths_contributes_nothing_and_does_not_fail() {
        // A package installed with no paths yet is a real shape, not an error.
        let m = mock_with_lineages(&[
            ("user/empty", vec![]),
            ("user/alpha", vec![("one.csv", 1_000)]),
        ]);

        let files = recent_files(&m).await.unwrap();

        assert_eq!(files.len(), 1);
        assert_eq!(files[0].namespace.to_string(), "user/alpha");
    }

    #[tokio::test]
    async fn one_unreadable_lineage_does_not_blank_the_whole_feed() {
        // The same rule `load_rows` follows (`main_page.rs:467-471`): warn and
        // skip. One bad lineage must not cost the user every other file.
        let m = mock_with_one_failing_lineage("user/broken", "user/alpha", "one.csv", 1_000);

        let files = recent_files(&m).await.unwrap();

        assert_eq!(files.len(), 1, "the readable package still arrives");
        assert_eq!(files[0].namespace.to_string(), "user/alpha");
    }
}
