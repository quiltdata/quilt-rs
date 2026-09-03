use quilt_uri::{Host, S3PackageUri};
use serde::{Deserialize, Serialize};

use crate::tauri;

// ── Response types ──────────────────────────────────────────

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
pub struct InstalledPackageData {
    pub namespace: String,
    pub uri: Option<S3PackageUri>,
    pub status: String,
    /// Hash of the revision currently installed locally, if any. Feeds the
    /// version-mismatch deep-link banner on `pages/installed_package.rs`.
    pub installed_hash: Option<String>,
    /// Commit message of the revision currently installed locally, if any.
    /// See `installed_hash`.
    pub installed_message: Option<String>,
    /// Package has been pushed — the remote is pinned to its push history
    /// and can't be edited. The toolbar's remote button becomes a read-only
    /// "Show remote" view.
    pub remote_locked: bool,
    /// Package has a local commit. Setting a remote only re-commits (creating
    /// a new revision) when there is one, so the Set-remote notice is gated
    /// on this.
    pub has_local_commit: bool,
    /// The active role cannot reach this package's bucket, worded as the
    /// roster words it. The status banner states this instead of the
    /// "unable to check remote status" copy, and must not offer Login: the
    /// session is healthy, so signing in again re-vends the same role.
    pub no_access_reason: Option<String>,
    pub entries: Vec<EntryData>,
    pub has_remote_entries: bool,
    pub ignored_count: usize,
    pub unmodified_count: usize,
    pub filter_unmodified: bool,
    pub filter_ignored: bool,
    /// This package's standing sync scope.
    pub syncs_entire_package: bool,
    /// Whether the experiment that offers the scope control is on.
    pub entire_package_sync_enabled: bool,
}

#[derive(Clone, Debug, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct EntryData {
    pub filename: String,
    pub size: u64,
    pub status: String,
    pub junky_pattern: Option<String>,
    pub ignored_by: Option<String>,
    pub namespace: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitData {
    pub namespace: String,
    pub uri: Option<S3PackageUri>,
    pub status: String,
    pub message: String,
    pub user_meta: String,
    pub user_meta_error: Option<String>,
    /// The previous revision's stamped workflow selection (its `id`), if any.
    pub workflow: Option<WorkflowData>,
    /// The bucket's workflow-selection situation for the commit dialog.
    pub workflows: CommitWorkflows,
    /// The active role cannot reach this package's bucket, worded as the
    /// roster words it — the same field name, shape and wording as
    /// [`InstalledPackageData::no_access_reason`]. Committing is not offline
    /// work (the workflow gate reads the bucket's config first), so this
    /// disables the commit affordances and supplies their tooltip.
    pub no_access_reason: Option<String>,
    pub entries: Vec<EntryData>,
    pub ignored_count: usize,
    pub unmodified_count: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowData {
    pub id: Option<String>,
}

/// A workflow declared under `workflows:` in the bucket's config, surfaced to
/// the commit dialog so the user can pick one. UI-side mirror of the backend
/// `CommitWorkflowInfo`. Distinct from [`WorkflowData`], which is the previous
/// revision's stamped selection.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowInfo {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    /// Catalog HTTPS link to the workflow's declared metadata schema object,
    /// pre-formatted by the backend. `None` when the workflow declares no
    /// metadata schema (or there is no catalog host to link against).
    pub metadata_schema_url: Option<String>,
    /// Catalog HTTPS link to the workflow's declared entries schema object.
    pub entries_schema_url: Option<String>,
}

/// The bucket's workflow-selection situation, as sent by the backend
/// (`quilt_sync::commands::commit_data::CommitWorkflows`). The serde
/// attributes MUST stay identical to the backend so the tagged JSON crosses
/// the Tauri boundary unchanged. Splits the three cases the commit dialog
/// renders distinctly:
/// - `Available` — the bucket has a config; offer its workflow choices.
/// - `NotConfigured` — the bucket is ungoverned; no choice to make.
/// - `Unavailable` — a transient failure loading the config; commit will retry
///   the bucket default.
/// - `Invalid` — the config is malformed; commits will fail until it is fixed.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CommitWorkflows {
    Available {
        workflows: Vec<WorkflowInfo>,
        default_workflow: Option<String>,
        is_workflow_required: bool,
        /// Catalog HTTPS link to the bucket's `.quilt/workflows/config.yml`
        /// object, pre-formatted by the backend. `None` when there is no
        /// catalog host to link against.
        config_url: Option<String>,
    },
    NotConfigured,
    Unavailable,
    Invalid {
        reason: String,
        /// Catalog HTTPS link to the bucket's `.quilt/workflows/config.yml`
        /// object, pre-formatted by the backend. `None` when there is no
        /// catalog host to link against.
        config_url: Option<String>,
    },
}

/// Which commit-dialog input a [`CommitViolation`] belongs under, so the UI can
/// render each advisory violation beneath the field the user must fix. UI-side
/// mirror of the backend `quilt_sync::commands::commit_data::ViolationField`;
/// the serde attributes MUST match so the tagged JSON crosses the Tauri boundary
/// unchanged.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ViolationField {
    Message,
    Metadata,
    Name,
}

/// A single advisory workflow violation for the commit dialog. UI-side mirror of
/// the backend `CommitViolation`.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct CommitViolation {
    pub field: ViolationField,
    pub message: String,
}

/// Caller intent for resolving a package's workflow, sent with a commit.
///
/// UI-side mirror of `quilt_rs::io::remote::WorkflowIntent`. The serde
/// attributes MUST stay identical to the backend so the tagged JSON crosses
/// the Tauri boundary unchanged:
/// - `{"kind":"bucket-default"}` — no opinion; honour the bucket default.
/// - `{"kind":"no-workflow"}` — explicit opt-out.
/// - `{"kind":"named","id":"x"}` — an exact workflow id.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "id", rename_all = "kebab-case")]
pub enum WorkflowIntent {
    BucketDefault,
    NoWorkflow,
    Named(String),
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeData {
    pub namespace: String,
    pub uri: Option<S3PackageUri>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginData {
    pub host: String,
    pub back: String,
    pub catalog_url: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginErrorData {
    pub title: String,
    pub message: String,
    pub login_host: String,
}

#[derive(Clone, Debug, Deserialize, Default, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct PublishSettingsData {
    pub message_template: String,
    pub default_workflow: String,
    pub default_metadata: String,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutosyncSettingsData {
    pub pull_enabled: bool,
    pub push_enabled: bool,
    pub pull_interval_secs: u64,
    pub idle_timeout_secs: u64,
    pub close_to_tray: bool,
}

impl Default for AutosyncSettingsData {
    fn default() -> Self {
        Self {
            pull_enabled: false,
            push_enabled: false,
            pull_interval_secs: 30,
            idle_timeout_secs: 300,
            close_to_tray: false,
        }
    }
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FsWatcherSettingsData {
    pub enabled: bool,
}

impl Default for FsWatcherSettingsData {
    fn default() -> Self {
        Self { enabled: true }
    }
}

/// Opt-ins for behaviour that is not finished being designed. Everything here
/// is off unless the user went looking for it.
#[derive(Clone, Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalSettingsData {
    pub entire_package_sync: bool,
    #[serde(default)]
    pub main_page_v2: bool,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SettingsData {
    pub version: String,
    pub home_dir: Option<String>,
    pub data_dir: String,
    pub auth_hosts: Vec<String>,
    pub log_level: String,
    pub logs_dir: String,
    pub logs_dir_is_temporary: bool,
    pub os: String,
    pub changelog: Vec<ChangelogEntry>,
    pub publish: PublishSettingsData,
    pub autosync: AutosyncSettingsData,
    pub fswatcher: FsWatcherSettingsData,
    pub experimental: ExperimentalSettingsData,
}

#[derive(Clone, Debug, Deserialize)]
pub struct ChangelogEntry {
    pub version: String,
    pub date: String,
    pub body: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupData {
    pub default_home: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackagesListData {
    pub packages: Vec<PackageItemData>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageItemData {
    pub namespace: String,
    pub status: String,
    pub has_changes: bool,
    /// Package has a local commit. Setting a remote only re-commits (creating
    /// a new revision) when there is one, so the Set-remote notice is gated
    /// on this.
    pub has_local_commit: bool,
    pub uri: Option<S3PackageUri>,
    pub remote_display: Option<String>,
    /// The autosync watcher's `Other` pause message for this namespace,
    /// fetched with the list data. `Some` means the row is autosync-paused
    /// for a reason the status string cannot carry, so it renders red with
    /// this reason as its third line; `None` means no such pause. Sourced
    /// directly from the backend's authoritative paused map — there is no
    /// frontend cache to go stale.
    pub paused_reason: Option<String>,
    /// The stable reason discriminant for the pause (`"pullConflict"`,
    /// `"other"`, …), paired with `paused_reason`. `Some` exactly when
    /// `paused_reason` is; lets the row pick conflict- vs. generic guidance
    /// without re-parsing the message.
    pub paused_kind: Option<String>,
    /// The active role cannot reach this row's bucket. The row explains
    /// itself with `no_access_reason` and must **not** offer Login: the
    /// session is fine, so signing in again re-vends the same denied role.
    pub no_access: bool,
    /// Why the row is marked, naming the active role. `Some` exactly when
    /// `no_access` is.
    pub no_access_reason: Option<String>,
    /// The host whose role selector the switch affordance opens. `Some`
    /// only when the user holds more than one role there, so a single-role
    /// user never gets a dead-end button.
    pub role_switch_host: Option<String>,
}

/// A deep-link banner to surface on the installed-package page after
/// resolving a `quilt+s3://` remote package URI. UI-side mirror of the
/// backend `quilt_sync::commands::remote_package::RemoteBanner`; the serde
/// attributes MUST match so the tagged JSON crosses the Tauri boundary
/// unchanged:
/// - `differentVersion` — the requested revision differs from what's
///   installed locally.
/// - `localOnly` — the package has no remote; it's local-only.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RemoteBanner {
    DifferentVersion {
        requested_hash: String,
        requested_bucket: String,
        requested_origin: Option<Host>,
        installed_hash: String,
    },
    LocalOnly,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePackageResult {
    pub namespace: String,
    pub banner: Option<RemoteBanner>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
}

// ── Data-fetching commands ──────────────────────────────────

pub async fn get_installed_package_data(
    namespace: String,
    filter: Option<String>,
) -> Result<InstalledPackageData, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        filter: Option<String>,
    }
    tauri::invoke("get_installed_package_data", &Args { namespace, filter }).await
}

pub async fn get_commit_data(namespace: String) -> Result<CommitData, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
    }
    tauri::invoke("get_commit_data", &Args { namespace }).await
}

/// Fetch and cache the selected workflow's rules for live commit-dialog
/// validation. Call when the workflow selection changes; the fetch runs once per
/// `(namespace, workflow)` and later calls hit the backend cache. Returns
/// whether the workflow has rules to validate against.
///
/// Pass `refresh = true` on the dialog's first load of a session so the backend
/// drops the namespace's cached entries and re-fetches — the cache is
/// app-lifetime state, so this is how a config.yml change since the last open is
/// picked up. Later loads within the session pass `false`.
pub async fn load_workflow_rules(
    namespace: String,
    workflow_id: String,
    refresh: bool,
) -> Result<bool, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        workflow_id: String,
        refresh: bool,
    }
    tauri::invoke(
        "load_workflow_rules",
        &Args {
            namespace,
            workflow_id,
            refresh,
        },
    )
    .await
}

/// Validate the current commit-dialog input against the cached rules for the
/// selected workflow. Pure cache read on the backend — no network I/O — so it is
/// safe to call on every (debounced) keystroke. Returns advisory violations
/// routed per field; empty means the input satisfies the workflow.
pub async fn validate_commit_candidate(
    namespace: String,
    workflow_id: String,
    message: String,
    user_meta: String,
    name: String,
) -> Result<Vec<CommitViolation>, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        workflow_id: String,
        message: String,
        user_meta: String,
        name: String,
    }
    tauri::invoke(
        "validate_commit_candidate",
        &Args {
            namespace,
            workflow_id,
            message,
            user_meta,
            name,
        },
    )
    .await
}

pub async fn get_merge_data(namespace: String) -> Result<MergeData, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
    }
    tauri::invoke("get_merge_data", &Args { namespace }).await
}

pub async fn get_login_data(host: String, back: String) -> Result<LoginData, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        host: String,
        back: String,
    }
    tauri::invoke("get_login_data", &Args { host, back }).await
}

pub async fn get_login_error_data(
    host: String,
    title: Option<String>,
    error: String,
) -> Result<LoginErrorData, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        host: String,
        title: Option<String>,
        error: String,
    }
    tauri::invoke("get_login_error_data", &Args { host, title, error }).await
}

pub async fn get_settings_data() -> Result<SettingsData, String> {
    tauri::invoke_unit("get_settings_data").await
}

pub async fn get_setup_data() -> Result<SetupData, String> {
    tauri::invoke_unit("get_setup_data").await
}

pub async fn get_installed_packages_list_data() -> Result<InstalledPackagesListData, String> {
    tauri::invoke_unit("get_installed_packages_list_data").await
}

/// v2's package list, light phase. Every row arrives `provisional`.
pub async fn get_main_page_packages() -> Result<MainPagePackagesData, String> {
    tauri::invoke_unit("get_main_page_packages").await
}

/// v2's package list, heavy phase. One invocation per row — see
/// `pages::main_page::PackageListRow`, which fires this and calls `RowSignals::apply`
/// on the answer.
pub async fn refresh_main_page_package(
    namespace: String,
) -> Result<MainPagePackageRefreshData, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
    }
    tauri::invoke("refresh_main_page_package", &Args { namespace }).await
}

/// What the heavy phase corrects on a row. Mirrors `MainPagePackageRefresh` on
/// the Tauri side; the fields the light phase already delivered are not resent.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainPagePackageRefreshData {
    pub state: crate::kit::PackageState,
    pub role_switch_host: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainPagePackagesData {
    pub packages: Vec<MainPagePackageData>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainPagePackageData {
    pub namespace: String,
    pub state: crate::kit::PackageState,
    /// Epoch milliseconds, from the last commit or the last installed path — see
    /// `last_changed` on the Tauri side. `None` only when nothing has ever been
    /// written to the package.
    pub changed_at: Option<f64>,
    /// The queue's group key for `RoleDenied` (§4.3, R2): one host can hold
    /// both readable and unreadable buckets, so `derive_queue` groups a
    /// denial by this rather than by `host`.
    pub bucket: Option<String>,
    /// The catalog this package points at, as the queue's join key: R3 groups
    /// an `Unknown` package under its host's `Signed out from {host}` cause
    /// only when the accounts payload agrees that host is signed out.
    pub host: Option<String>,
    pub provisional: bool,
    /// The host whose role selector the row's switch affordance opens. The list
    /// carries it into `RowSignals` and settles it on refresh; the switch
    /// control itself is the queue's (Plan 4), so no `#[expect(dead_code)]`
    /// here — the field is carried and settled, never read.
    pub role_switch_host: Option<String>,
}

/// Whether a direction's machinery is counting down, waiting, or stopped.
///
/// A closed set with **no catch-all**, unlike [`PausedReasonData`]: these three
/// are the whole vocabulary of the toggle's trailing slot (§4.2), a fourth would
/// be a design change rather than a wire addition, and `#[serde(other)]` does
/// not apply to a plainly-serialized unit enum anyway. Drift is caught by
/// `main_page_watcher_data_wire_form_is_verbatim`.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ToggleActivityData {
    Armed,
    Idle,
    Paused,
}

/// UI-side mirror of the backend's `ToggleState`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ToggleStateData {
    pub enabled: bool,
    pub activity: ToggleActivityData,
    /// Epoch milliseconds. `Some` exactly when `activity` is `Armed` — the
    /// backend derives both from one expression.
    pub deadline: Option<f64>,
    pub interval_ms: f64,
}

/// Why the watcher stopped syncing one package. UI-side mirror of the backend's
/// `PausedDto`.
///
/// `Unrecognised` is `#[serde(other)]`: a reason added to the backend without an
/// arm here degrades to one variant instead of failing the whole payload and
/// taking the card with it. It carries no data, because `#[serde(other)]`
/// accepts only unit variants — and does not need to, since the fixed words for
/// an unexplained pause are the UI's anyway.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum PausedReasonData {
    PendingChanges,
    PendingCommit,
    Diverged,
    PullConflict {
        files: Vec<String>,
    },
    RoleDenied {
        role: Option<String>,
    },
    Other {
        message: String,
    },
    #[serde(other)]
    Unrecognised,
}

/// The Autosync card carries this list but renders nothing from it: a pause's
/// namespace and reason are intended for the queue, and nothing in this build
/// reads either — hence the suppression.
///
/// `expect` rather than `allow`, and only outside `cfg(test)`: the wire-form
/// test does read these fields, so the attribute has to be absent there to stay
/// fulfilled. In the shipped build it hard-errors the moment a real caller
/// appears, so the suppression cannot outlive its reason.
#[cfg_attr(not(test), expect(dead_code))]
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PausedPackageData {
    pub namespace: String,
    pub reason: PausedReasonData,
}

/// Payload 3: the watcher's own state. Returned by `get_main_page_watcher`.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MainPageWatcherData {
    pub pull: ToggleStateData,
    pub publish: ToggleStateData,
    /// Intended for the queue, which joins it against the package list by
    /// namespace to ask whether a pause is explained by a row the user can
    /// already see (§4.3). Carried and pinned here; no reader in this build —
    /// hence the field-level suppression, `expect`-shaped so it reports itself
    /// the moment one arrives. Absent under `cfg(test)`, where the wire-form
    /// test reads the field.
    #[cfg_attr(not(test), expect(dead_code))]
    pub paused: Vec<PausedPackageData>,
}

/// v2's watcher state: both toggles and the pause list. Drawn by
/// `pages::main_page::autosync::AutosyncCard`.
pub async fn get_main_page_watcher() -> Result<MainPageWatcherData, String> {
    tauri::invoke_unit("get_main_page_watcher").await
}

/// One host in the Accounts card, as it arrives.
///
/// Mirrors `quilt_sync::commands::main_page::AccountHost`. `currentRole` is
/// `null` both before the role is resolved and when the query failed — the two
/// are told apart by `provisional`, not by the role.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AccountHostData {
    pub host: String,
    pub signed_in: bool,
    pub current_role: Option<String>,
    pub roles: Vec<String>,
    pub provisional: bool,
}

/// Payload of `get_main_page_accounts`: the Accounts card's light phase.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct MainPageAccountsData {
    pub hosts: Vec<AccountHostData>,
}

/// v2's accounts list, light phase: one row per host, `signedIn` only.
pub async fn get_main_page_accounts() -> Result<MainPageAccountsData, String> {
    tauri::invoke_unit("get_main_page_accounts").await
}

/// v2's accounts list, heavy phase: fills in the given host's role.
pub async fn refresh_main_page_account(host: String) -> Result<AccountHostData, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        host: String,
    }
    tauri::invoke("refresh_main_page_account", &Args { host }).await
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshedPackageStatus {
    pub status: String,
    pub has_changes: bool,
    /// See [`PackageItemData::no_access`]. Carried here too, so a denial
    /// the heavy phase discovers reaches a row the light phase's bucket
    /// list had cleared.
    pub no_access: bool,
    pub no_access_reason: Option<String>,
    pub role_switch_host: Option<String>,
}

/// Payload of the `package-status-changed` Tauri event. Same shape as
/// [`RefreshedPackageStatus`] plus the namespace it applies to, so the
/// list/detail pages can match a row by namespace.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PackageStatusEvent {
    pub namespace: String,
    pub status: String,
    pub has_changes: bool,
    /// Digest of the observation this event reports (see the backend's
    /// `PackageStatusEvent`). The page acts only when it differs from the last
    /// one it acted on, so a tick that re-reports an unchanged tree — same
    /// fingerprint — leaves the page untouched instead of rebuilding it.
    pub fingerprint: String,
}

pub const PACKAGE_STATUS_EVENT: &str = "package-status-changed";

/// Payload of the `autosync-published` Tauri event — emitted after a
/// background autosync tick successfully publishes a package. UI listens
/// for this on the installed-packages list page and surfaces it as a
/// toast, mirroring the manual Commit & Push success notification.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishedEvent {
    pub namespace: String,
    pub message: String,
}

pub const AUTOSYNC_PUBLISHED_EVENT: &str = "autosync-published";

/// Payload of the `autosync-paused` Tauri event — emitted when the
/// background watcher pauses a namespace. The `reason` field is a stable
/// string discriminant; `message` is populated only for `reason = "other"`
/// (workflow rejection, hash mismatch, JSON parse failure, etc.) and is
/// what the per-package banner renders so the user knows what to fix.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PausedEvent {
    pub namespace: String,
    pub reason: String,
    pub message: Option<String>,
}

pub const AUTOSYNC_PAUSED_EVENT: &str = "autosync-paused";

/// Point-in-time view of the autosync watcher's per-namespace state.
/// Returned by `get_autosync_snapshot`; the UI uses it to re-hydrate
/// the paused banner when a page mounts after the watcher already
/// paused a namespace (the `autosync-paused` event alone would miss
/// those pauses).
#[derive(Clone, Debug, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct WatcherSnapshot {
    pub paused: Vec<PausedEvent>,
}

pub async fn get_autosync_snapshot() -> Result<WatcherSnapshot, String> {
    #[derive(Serialize)]
    struct Args {}
    tauri::invoke("get_autosync_snapshot", &Args {}).await
}

pub async fn refresh_package_status(namespace: String) -> Result<RefreshedPackageStatus, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
    }
    tauri::invoke("refresh_package_status", &Args { namespace }).await
}

pub async fn handle_remote_package(uri: String) -> Result<RemotePackageResult, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        uri: String,
    }
    tauri::invoke("handle_remote_package", &Args { uri }).await
}

/// Fetch the commit message for a specific revision of a package, so the
/// installed-package page can show what the requested (but not installed)
/// revision says, alongside the currently installed one.
pub async fn get_revision_message(
    bucket: String,
    namespace: String,
    hash: String,
    catalog: Option<String>,
) -> Result<Option<String>, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        bucket: String,
        namespace: String,
        hash: String,
        catalog: Option<String>,
    }
    tauri::invoke(
        "get_revision_message",
        &Args {
            bucket,
            namespace,
            hash,
            catalog,
        },
    )
    .await
}

// ── Auto-update ────────────────────────────────────────────

pub async fn check_for_update() -> Result<Option<UpdateInfo>, String> {
    tauri::invoke_unit("check_for_update").await
}

pub async fn download_and_install_update() -> Result<(), String> {
    tauri::invoke_unit("download_and_install_update").await
}

// ── Package actions ─────────────────────────────────────────

pub async fn package_commit(
    namespace: String,
    message: String,
    metadata: String,
    workflow: WorkflowIntent,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        message: String,
        metadata: String,
        workflow: WorkflowIntent,
        uri: Option<S3PackageUri>,
    }
    tauri::invoke(
        "package_commit",
        &Args {
            namespace,
            message,
            metadata,
            workflow,
            uri,
        },
    )
    .await
}

pub async fn package_push(namespace: String, uri: Option<S3PackageUri>) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        uri: Option<S3PackageUri>,
    }
    tauri::invoke("package_push", &Args { namespace, uri }).await
}

pub async fn package_publish(
    namespace: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        uri: Option<S3PackageUri>,
    }
    tauri::invoke("package_publish", &Args { namespace, uri }).await
}

pub async fn package_commit_and_push(
    namespace: String,
    message: String,
    metadata: String,
    workflow: WorkflowIntent,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        message: String,
        metadata: String,
        workflow: WorkflowIntent,
        uri: Option<S3PackageUri>,
    }
    tauri::invoke(
        "package_commit_and_push",
        &Args {
            namespace,
            message,
            metadata,
            workflow,
            uri,
        },
    )
    .await
}

pub async fn update_publish_settings(
    message_template: String,
    default_workflow: String,
    default_metadata: String,
) -> Result<(), String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        message_template: String,
        default_workflow: String,
        default_metadata: String,
    }
    tauri::invoke(
        "update_publish_settings",
        &Args {
            message_template,
            default_workflow,
            default_metadata,
        },
    )
    .await
}

pub async fn update_autosync_settings(settings: AutosyncSettingsData) -> Result<(), String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        settings: AutosyncSettingsData,
    }
    tauri::invoke("update_autosync_settings", &Args { settings }).await
}

/// Flip one direction of autosync. `None` leaves a direction alone — the v2 main
/// page knows two booleans, and `update_autosync_settings` takes all five settings
/// fields, so writing through that one would mean fetching a payload this page has
/// no other use for.
pub async fn set_autosync_direction(pull: Option<bool>, push: Option<bool>) -> Result<(), String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        pull: Option<bool>,
        push: Option<bool>,
    }
    tauri::invoke("set_autosync_direction", &Args { pull, push }).await
}

pub async fn update_fswatcher_settings(enabled: bool) -> Result<(), String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        enabled: bool,
    }
    tauri::invoke("update_fswatcher_settings", &Args { enabled }).await
}

/// Turn an experiment on or off. `None` leaves a flag as it is — a caller that
/// knows about one experiment must not reset another.
pub async fn update_experimental_settings(
    entire_package_sync: Option<bool>,
    main_page_v2: Option<bool>,
) -> Result<(), String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        entire_package_sync: Option<bool>,
        main_page_v2: Option<bool>,
    }
    tauri::invoke(
        "update_experimental_settings",
        &Args {
            entire_package_sync,
            main_page_v2,
        },
    )
    .await
}

/// Record whether a package keeps its whole contents. Storage only — catching
/// up on files already listed is a separate `package_install_paths` call.
pub async fn package_set_sync_scope(namespace: String, entire_package: bool) -> Result<(), String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        entire_package: bool,
    }
    tauri::invoke(
        "package_set_sync_scope",
        &Args {
            namespace,
            entire_package,
        },
    )
    .await
}

/// Payload of the `fswatcher-subscriber-error` Tauri event. Surfaced as a
/// one-shot toast (e.g. for `kind == "inotify_limit"` on Linux).
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SubscriberErrorEvent {
    pub kind: String,
    pub message: String,
    pub namespace: Option<String>,
}

pub const FSWATCHER_SUBSCRIBER_ERROR_EVENT: &str = "fswatcher-subscriber-error";

pub async fn package_pull(namespace: String, uri: Option<S3PackageUri>) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        uri: Option<S3PackageUri>,
    }
    tauri::invoke("package_pull", &Args { namespace, uri }).await
}

/// The dry-run verdict of what a Pull would do right now. UI-side mirror of the
/// engine's `quilt_rs::flow::PullOutcome` (the UI crate cannot depend on the
/// engine crate). The serde shape is the engine enum's default externally
/// tagged form with no field renaming; it MUST stay identical so the tagged
/// JSON crosses the Tauri boundary unchanged. Paths arrive as strings (the
/// engine's `PathBuf`s serialize as such); the UI only displays them.
///
/// The literals are anchored identically in the backend's
/// `pull_outcome_wire_form_is_verbatim`; if the two drift, the two-phase Pull
/// affordance silently misreads the outcome at the boundary.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
pub enum PullOutcome {
    UpToDate,
    CleanUpdate,
    KeepsLocalChanges {
        added: Vec<String>,
        modified: Vec<String>,
        removed: Vec<String>,
    },
    Blocked {
        conflicts: Vec<String>,
    },
}

impl PullOutcome {
    /// Whether a Pull can proceed for this dry-run outcome. Only `Blocked` — a
    /// real two-sided conflict — disables the button; every other outcome,
    /// including `KeepsLocalChanges`, pulls safely.
    #[must_use]
    pub fn is_pullable(&self) -> bool {
        !matches!(self, PullOutcome::Blocked { .. })
    }
}

/// Tri-state view of the dry-run pull check for the Pull affordance. Replaces
/// the old `Option<PullOutcome>`, where `None` conflated "still loading" with
/// "the fetch failed" — a single failed dry-run then stranded the Pull button
/// on "Checking for updates…" forever, with no retry. `Loading` and `Failed`
/// both keep Pull disabled (fail-safe); `Failed` additionally offers a retry.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PullCheck {
    /// The dry-run has not resolved yet (genuine in-flight state).
    Loading,
    /// The dry-run fetch errored; the button area offers a retry.
    Failed,
    /// The dry-run resolved to a concrete outcome.
    Ready(PullOutcome),
}

impl PullCheck {
    /// Whether Pull should be enabled: only a resolved, pullable outcome.
    #[must_use]
    pub fn pull_enabled(&self) -> bool {
        matches!(self, PullCheck::Ready(o) if o.is_pullable())
    }

    /// Whether the dry-run failed, so a retry affordance should show.
    #[must_use]
    pub fn is_failed(&self) -> bool {
        matches!(self, PullCheck::Failed)
    }
}

pub async fn package_pull_outcome(namespace: String) -> Result<PullOutcome, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
    }
    tauri::invoke("package_pull_outcome", &Args { namespace }).await
}

pub async fn package_uninstall(
    namespace: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        uri: Option<S3PackageUri>,
    }
    tauri::invoke("package_uninstall", &Args { namespace, uri }).await
}

pub async fn package_install_paths(uri: String, paths: Vec<String>) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        uri: String,
        paths: Vec<String>,
    }
    tauri::invoke("package_install_paths", &Args { uri, paths }).await
}

pub async fn package_create(
    namespace: String,
    source: Option<String>,
    message: Option<String>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        source: Option<String>,
        message: Option<String>,
    }
    tauri::invoke(
        "package_create",
        &Args {
            namespace,
            source,
            message,
        },
    )
    .await
}

// ── Merge actions ───────────────────────────────────────────

pub async fn certify_latest(
    namespace: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        uri: Option<S3PackageUri>,
    }
    tauri::invoke("certify_latest", &Args { namespace, uri }).await
}

pub async fn reset_local(namespace: String, uri: Option<S3PackageUri>) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        uri: Option<S3PackageUri>,
    }
    tauri::invoke("reset_local", &Args { namespace, uri }).await
}

// ── Remote ──────────────────────────────────────────────────

/// Response from the `set_remote` command. UI-side mirror of the backend
/// `quilt_sync::commands::package_ops::SetRemoteResponse`; the serde attributes
/// MUST match so the typed payload crosses the Tauri boundary unchanged.
/// `resolution_warning` is `Some(reason)` when the remote was set but the
/// bucket's default workflow could not be resolved — the popup raises a warning
/// notice instead of a plain success.
#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SetRemoteResponse {
    pub message: String,
    pub resolution_warning: Option<String>,
}

pub async fn set_remote(
    namespace: String,
    origin: String,
    bucket: String,
    workflow: WorkflowIntent,
) -> Result<SetRemoteResponse, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        origin: String,
        bucket: String,
        workflow: WorkflowIntent,
    }
    tauri::invoke(
        "set_remote",
        &Args {
            namespace,
            origin,
            bucket,
            workflow,
        },
    )
    .await
}

/// Fetch a bucket's declared workflows before its remote is set, so the
/// set-remote popup can present the same tri-state control the commit dialog
/// uses. Mirrors the backend `get_bucket_workflows` command; a fetch failure
/// maps to [`CommitWorkflows::Unavailable`] rather than an error.
pub async fn get_bucket_workflows(host: String, bucket: String) -> Result<CommitWorkflows, String> {
    #[derive(Serialize)]
    struct Args {
        host: String,
        bucket: String,
    }
    tauri::invoke("get_bucket_workflows", &Args { host, bucket }).await
}

// ── Auth ────────────────────────────────────────────────────

pub async fn login(host: String, code: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        host: String,
        code: String,
    }
    tauri::invoke("login", &Args { host, code }).await
}

pub async fn login_oauth(host: String, back: Option<String>) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        host: String,
        back: Option<String>,
    }
    tauri::invoke("login_oauth", &Args { host, back }).await
}

pub async fn erase_auth(host: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        host: String,
    }
    tauri::invoke("erase_auth", &Args { host }).await
}

/// The roles a host's stack grants the logged-in user. UI-side mirror of the
/// backend `quilt_sync::commands::auth::RolesData`; the serde attributes MUST
/// match so the payload crosses the Tauri boundary unchanged. `available`
/// includes `current`, so a single-element list means there is nothing to
/// switch to.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RolesData {
    pub current: String,
    pub available: Vec<String>,
}

/// Fetch the host's roles. Takes that host's credential single-flight lock, so
/// call it lazily — never as part of a bulk page load.
pub async fn get_roles(host: String) -> Result<RolesData, String> {
    #[derive(Serialize)]
    struct Args {
        host: String,
    }
    tauri::invoke("get_roles", &Args { host }).await
}

/// Switch the host's primary role. The backend expires the stored credentials
/// and clears the host's cached S3 clients, so the UI has no cache work to do.
/// Returns the roles as confirmed by the stack — the confirmed `current` can
/// differ from the requested role, so render what comes back, not what was
/// asked for.
pub async fn switch_role(host: String, role: String) -> Result<RolesData, String> {
    #[derive(Serialize)]
    struct Args {
        host: String,
        role: String,
    }
    tauri::invoke("switch_role", &Args { host, role }).await
}

// ── Setup ───────────────────────────────────────────────────

pub async fn setup(directory: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        directory: String,
    }
    tauri::invoke("setup", &Args { directory }).await
}

// ── Quiltignore ─────────────────────────────────────────────

pub async fn add_to_quiltignore(
    namespace: String,
    pattern: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        pattern: String,
        uri: Option<S3PackageUri>,
    }
    tauri::invoke(
        "add_to_quiltignore",
        &Args {
            namespace,
            pattern,
            uri,
        },
    )
    .await
}

pub async fn test_quiltignore_pattern(pattern: String, path: String) -> Result<bool, String> {
    #[derive(Serialize)]
    struct Args {
        pattern: String,
        path: String,
    }
    tauri::invoke("test_quiltignore_pattern", &Args { pattern, path }).await
}

// ── File/browser ────────────────────────────────────────────

pub async fn open_in_file_browser(
    namespace: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        uri: Option<S3PackageUri>,
    }
    tauri::invoke("open_in_file_browser", &Args { namespace, uri }).await
}

pub async fn open_in_web_browser(url: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        url: String,
    }
    tauri::invoke("open_in_web_browser", &Args { url }).await
}

pub async fn open_in_default_application(
    namespace: String,
    path: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        path: String,
        uri: Option<S3PackageUri>,
    }
    tauri::invoke(
        "open_in_default_application",
        &Args {
            namespace,
            path,
            uri,
        },
    )
    .await
}

pub async fn reveal_in_file_browser(
    namespace: String,
    path: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        path: String,
        uri: Option<S3PackageUri>,
    }
    tauri::invoke(
        "reveal_in_file_browser",
        &Args {
            namespace,
            path,
            uri,
        },
    )
    .await
}

pub async fn open_directory_picker() -> Result<String, String> {
    tauri::invoke_unit("open_directory_picker").await
}

// ── Debug/diagnostics ───────────────────────────────────────

pub async fn debug_dot_quilt() -> Result<String, String> {
    tauri::invoke_unit("debug_dot_quilt").await
}

pub async fn debug_logs() -> Result<String, String> {
    tauri::invoke_unit("debug_logs").await
}

pub async fn open_home_dir() -> Result<String, String> {
    tauri::invoke_unit("open_home_dir").await
}

pub async fn open_data_dir() -> Result<String, String> {
    tauri::invoke_unit("open_data_dir").await
}

pub async fn collect_diagnostic_logs() -> Result<String, String> {
    tauri::invoke_unit("collect_diagnostic_logs").await
}

pub async fn send_crash_report(zip_path: String) -> Result<String, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        zip_path: String,
    }
    tauri::invoke("send_crash_report", &Args { zip_path }).await
}

#[cfg(test)]
mod tests {
    use super::{
        CommitViolation, CommitWorkflows, PackageItemData, PullOutcome, RolesData, ViolationField,
        WorkflowInfo, WorkflowIntent,
    };
    use wasm_bindgen_test::*;

    /// The mirror struct must deserialize the exact JSON the backend
    /// (`quilt_sync::commands::package_list::InstalledPackageListItem`)
    /// serializes. This literal is anchored identically in the backend's
    /// `package_item_data_wire_form_is_verbatim`; if the two drift, the
    /// list silently drops the pause reason (or a whole field) at the
    /// Tauri boundary — the exact class of bug this data-driven design
    /// exists to prevent.
    #[test]
    fn package_item_data_wire_form_is_verbatim() {
        let item = serde_json::from_str::<PackageItemData>(
            r#"{"namespace":"acme/data","status":"paused","hasChanges":false,"hasLocalCommit":false,"uri":null,"remoteDisplay":null,"pausedReason":"workflow rejected metadata","pausedKind":"other","noAccess":true,"noAccessReason":"Current role ReadOnly has no access to this bucket","roleSwitchHost":"acme.quilt.dev"}"#,
        )
        .unwrap();
        assert_eq!(item.namespace, "acme/data");
        assert_eq!(item.status, "paused");
        assert!(!item.has_changes);
        assert!(!item.has_local_commit);
        assert!(item.uri.is_none());
        assert!(item.remote_display.is_none());
        assert_eq!(
            item.paused_reason.as_deref(),
            Some("workflow rejected metadata")
        );
        assert_eq!(item.paused_kind.as_deref(), Some("other"));
        assert!(item.no_access);
        assert_eq!(
            item.no_access_reason.as_deref(),
            Some("Current role ReadOnly has no access to this bucket")
        );
        assert_eq!(item.role_switch_host.as_deref(), Some("acme.quilt.dev"));
    }

    /// The mirror types must deserialize the exact tagged JSON the backend
    /// (`quilt_sync::commands::commit_data::CommitViolation`) serializes. These
    /// literals are anchored identically in the backend's
    /// `commit_violation_wire_form_is_verbatim`; if they drift, the dialog routes
    /// live violations to the wrong field or drops them.
    #[test]
    fn commit_violation_wire_form_is_verbatim() {
        assert_eq!(
            serde_json::from_str::<CommitViolation>(r#"{"field":"metadata","message":"bad"}"#)
                .unwrap(),
            CommitViolation {
                field: ViolationField::Metadata,
                message: "bad".to_string(),
            }
        );
        assert_eq!(
            serde_json::from_str::<ViolationField>(r#""message""#).unwrap(),
            ViolationField::Message
        );
        assert_eq!(
            serde_json::from_str::<ViolationField>(r#""name""#).unwrap(),
            ViolationField::Name
        );
    }

    /// The mirror enum must deserialize the exact tagged JSON the backend
    /// (`quilt_sync::commands::commit_data::CommitWorkflows`) serializes. These
    /// literals are anchored identically in the backend's
    /// `commit_workflows_wire_form_is_verbatim`; if the two drift, the commit
    /// dialog silently loses the workflow list.
    #[test]
    fn commit_workflows_wire_form_is_verbatim() {
        assert_eq!(
            serde_json::from_str::<CommitWorkflows>(
                r#"{"state":"available","workflows":[{"id":"alpha","name":"Alpha","description":null,"metadataSchemaUrl":"https://catalog/b/bucket/tree/meta.json","entriesSchemaUrl":null}],"defaultWorkflow":"alpha","isWorkflowRequired":true,"configUrl":"https://catalog/b/bucket/tree/.quilt/workflows/config.yml"}"#
            )
            .unwrap(),
            CommitWorkflows::Available {
                workflows: vec![WorkflowInfo {
                    id: "alpha".to_string(),
                    name: Some("Alpha".to_string()),
                    description: None,
                    metadata_schema_url: Some(
                        "https://catalog/b/bucket/tree/meta.json".to_string()
                    ),
                    entries_schema_url: None,
                }],
                default_workflow: Some("alpha".to_string()),
                is_workflow_required: true,
                config_url: Some(
                    "https://catalog/b/bucket/tree/.quilt/workflows/config.yml".to_string()
                ),
            }
        );
        assert_eq!(
            serde_json::from_str::<CommitWorkflows>(r#"{"state":"notConfigured"}"#).unwrap(),
            CommitWorkflows::NotConfigured
        );
        assert_eq!(
            serde_json::from_str::<CommitWorkflows>(r#"{"state":"unavailable"}"#).unwrap(),
            CommitWorkflows::Unavailable
        );
        assert_eq!(
            serde_json::from_str::<CommitWorkflows>(
                r#"{"state":"invalid","reason":"bad schema","configUrl":"https://catalog/b/bucket/tree/.quilt/workflows/config.yml"}"#
            )
            .unwrap(),
            CommitWorkflows::Invalid {
                reason: "bad schema".to_string(),
                config_url: Some(
                    "https://catalog/b/bucket/tree/.quilt/workflows/config.yml".to_string()
                ),
            }
        );
    }

    /// The mirror enum must deserialize the exact externally-tagged JSON the
    /// backend (`quilt_rs::flow::PullOutcome`) serializes. These literals are
    /// anchored identically in the backend's `pull_outcome_wire_form_is_verbatim`;
    /// if the two drift, the two-phase Pull affordance silently misreads the
    /// dry-run outcome at the Tauri boundary.
    #[test]
    fn pull_outcome_wire_form_is_verbatim() {
        assert_eq!(
            serde_json::from_str::<PullOutcome>(r#""UpToDate""#).unwrap(),
            PullOutcome::UpToDate
        );
        assert_eq!(
            serde_json::from_str::<PullOutcome>(r#""CleanUpdate""#).unwrap(),
            PullOutcome::CleanUpdate
        );
        assert_eq!(
            serde_json::from_str::<PullOutcome>(
                r#"{"KeepsLocalChanges":{"added":["a.txt"],"modified":[],"removed":["c.txt"]}}"#
            )
            .unwrap(),
            PullOutcome::KeepsLocalChanges {
                added: vec!["a.txt".to_string()],
                modified: vec![],
                removed: vec!["c.txt".to_string()],
            }
        );
        assert_eq!(
            serde_json::from_str::<PullOutcome>(r#"{"Blocked":{"conflicts":["x.txt"]}}"#).unwrap(),
            PullOutcome::Blocked {
                conflicts: vec!["x.txt".to_string()],
            }
        );
    }

    #[test]
    fn pull_outcome_is_pullable_only_blocks_on_conflicts() {
        assert!(PullOutcome::UpToDate.is_pullable());
        assert!(PullOutcome::CleanUpdate.is_pullable());
        assert!(
            PullOutcome::KeepsLocalChanges {
                added: vec!["a.txt".to_string()],
                modified: vec![],
                removed: vec![],
            }
            .is_pullable()
        );
        assert!(
            !PullOutcome::Blocked {
                conflicts: vec!["x.txt".to_string()],
            }
            .is_pullable()
        );
    }

    /// The mirror struct must deserialize the exact JSON the backend
    /// (`quilt_sync::commands::auth::RolesData`) serializes. If the two drift,
    /// the Settings role switcher silently sees one role and hides itself.
    #[test]
    fn roles_data_wire_form_is_verbatim() {
        assert_eq!(
            serde_json::from_str::<RolesData>(
                r#"{"current":"ReadWrite","available":["ReadOnly","ReadWrite"]}"#
            )
            .unwrap(),
            RolesData {
                current: "ReadWrite".to_string(),
                available: vec!["ReadOnly".to_string(), "ReadWrite".to_string()],
            }
        );
    }

    /// The mirror struct must deserialize the exact JSON the backend's light-phase
    /// wire-shape test
    /// (`quilt_sync::commands::main_page::get_main_page_packages_from_model_serializes_the_wire_shape`)
    /// pins for one row. If the two drift, a light-phase row silently fails to
    /// deserialize at the Tauri boundary.
    #[wasm_bindgen_test]
    fn main_page_packages_data_wire_form_is_verbatim() {
        let data = serde_json::from_str::<super::MainPagePackagesData>(
            r#"{"packages":[{"namespace":"team/latest","state":{"kind":"latest"},"changedAt":null,"bucket":"test","host":"test.quilt.dev","provisional":true,"roleSwitchHost":null}]}"#,
        )
        .unwrap();
        assert_eq!(data.packages.len(), 1);
        let pkg = &data.packages[0];
        assert_eq!(pkg.namespace, "team/latest");
        assert_eq!(pkg.state, crate::kit::PackageState::Latest);
        assert_eq!(pkg.changed_at, None);
        assert_eq!(pkg.host.as_deref(), Some("test.quilt.dev"));
        assert!(pkg.provisional);
        assert_eq!(pkg.role_switch_host, None);
    }

    /// The mirror struct must deserialize the heavy phase's two never-before-seen
    /// kinds. UI-side mirror of the backend
    /// `quilt_sync::commands::main_page::MainPagePackageRefresh`; these two
    /// literals never crossed the wire before the heavy phase existed, because the
    /// light phase could not produce either. Deserializing to `PackageState::Unknown`
    /// is `#[serde(other)]`'s silent-drift failure mode — that is what this pins
    /// against, not just "it parses".
    #[test]
    fn main_page_package_refresh_data_wire_form_is_verbatim() {
        let pending = serde_json::from_str::<super::MainPagePackageRefreshData>(
            r#"{"state":{"kind":"pending_changes","files":3},"roleSwitchHost":null}"#,
        )
        .unwrap();
        assert!(
            !matches!(pending.state, crate::kit::PackageState::Unknown),
            "a kind drift would silently land here, via #[serde(other)]"
        );
        assert_eq!(
            pending.state,
            crate::kit::PackageState::PendingChanges { files: 3 }
        );

        let denied = serde_json::from_str::<super::MainPagePackageRefreshData>(
            r#"{"state":{"kind":"role_denied","role":null},"roleSwitchHost":"h"}"#,
        )
        .unwrap();
        assert!(
            !matches!(denied.state, crate::kit::PackageState::Unknown),
            "a kind drift would silently land here, via #[serde(other)]"
        );
        assert_eq!(
            denied.state,
            crate::kit::PackageState::RoleDenied { role: None }
        );
        assert_eq!(denied.role_switch_host.as_deref(), Some("h"));
    }

    /// The mirror enum must serialize to the exact tagged JSON the backend
    /// (`quilt_rs::io::remote::WorkflowIntent`) deserializes, and round-trip
    /// back. If these strings drift, the Tauri commit boundary breaks silently.
    #[test]
    fn workflow_intent_wire_form_is_verbatim() {
        let cases = [
            (
                WorkflowIntent::BucketDefault,
                r#"{"kind":"bucket-default"}"#,
            ),
            (WorkflowIntent::NoWorkflow, r#"{"kind":"no-workflow"}"#),
            (
                WorkflowIntent::Named("x".to_string()),
                r#"{"kind":"named","id":"x"}"#,
            ),
        ];
        for (intent, json) in cases {
            assert_eq!(serde_json::to_string(&intent).unwrap(), json);
            assert_eq!(
                serde_json::from_str::<WorkflowIntent>(json).unwrap(),
                intent
            );
        }
    }

    /// The mirror struct must deserialize the exact JSON the backend's
    /// `quilt_sync::commands::main_page::the_watcher_payload_serializes_the_wire_shape`
    /// pins. Character-for-character: a literal the backend does not emit looks like
    /// a guard and proves nothing (plan 2's Fix 2).
    #[wasm_bindgen_test]
    fn main_page_watcher_data_wire_form_is_verbatim() {
        let data = serde_json::from_str::<super::MainPageWatcherData>(
            r#"{"pull":{"enabled":true,"activity":"paused","deadline":null,"intervalMs":30000.0},"publish":{"enabled":true,"activity":"paused","deadline":null,"intervalMs":300000.0},"paused":[{"namespace":"team/plate-07","reason":{"kind":"pull_conflict","files":["a.csv","b.csv"]}}]}"#,
        )
        .unwrap();
        assert!(data.pull.enabled);
        assert_eq!(data.pull.activity, super::ToggleActivityData::Paused);
        assert_eq!(data.pull.deadline, None);
        assert!((data.pull.interval_ms - 30_000.0).abs() < f64::EPSILON);
        assert!((data.publish.interval_ms - 300_000.0).abs() < f64::EPSILON);
        assert_eq!(data.paused.len(), 1);
        assert_eq!(data.paused[0].namespace, "team/plate-07");
        match &data.paused[0].reason {
            super::PausedReasonData::PullConflict { files } => {
                assert_eq!(files, &["a.csv".to_string(), "b.csv".to_string()]);
            }
            other => panic!("expected a pull conflict with two paths, got {other:?}"),
        }
    }

    #[wasm_bindgen_test]
    fn an_armed_toggle_arrives_with_its_deadline() {
        let data = serde_json::from_str::<super::MainPageWatcherData>(
            r#"{"pull":{"enabled":true,"activity":"armed","deadline":1754500030000.0,"intervalMs":30000.0},"publish":{"enabled":false,"activity":"idle","deadline":null,"intervalMs":300000.0},"paused":[]}"#,
        )
        .unwrap();
        assert_eq!(data.pull.activity, super::ToggleActivityData::Armed);
        assert_eq!(data.pull.deadline, Some(1_754_500_030_000.0));
        assert_eq!(data.publish.activity, super::ToggleActivityData::Idle);
    }

    #[wasm_bindgen_test]
    fn a_conflict_arrives_as_a_list_with_its_commas_intact() {
        // `qhq-8mgw.9`'s whole point, asserted at the boundary that used to flatten
        // it: a filename containing ", " must not become two paths.
        let data = serde_json::from_str::<super::PausedReasonData>(
            r#"{"kind":"pull_conflict","files":["plate, run 3.csv","b.csv"]}"#,
        )
        .unwrap();
        match data {
            super::PausedReasonData::PullConflict { files } => {
                assert_eq!(files.len(), 2);
                assert_eq!(files[0], "plate, run 3.csv");
            }
            other => panic!("got {other:?}"),
        }
    }

    #[wasm_bindgen_test]
    fn a_reason_this_build_has_never_heard_of_degrades_instead_of_failing_the_payload() {
        // A reason added to the backend without an arm here would otherwise fail the
        // WHOLE payload and the card would vanish. Same treatment
        // `PackageState::Unknown` gets, for the same reason.
        assert_eq!(
            serde_json::from_str::<super::PausedReasonData>(r#"{"kind":"some_future_reason"}"#)
                .unwrap(),
            super::PausedReasonData::Unrecognised
        );
        // And the guard against `#[serde(other)]`'s silent-drift failure mode: a
        // kind we DO know must not land in the catch-all.
        assert_ne!(
            serde_json::from_str::<super::PausedReasonData>(r#"{"kind":"diverged"}"#).unwrap(),
            super::PausedReasonData::Unrecognised
        );
    }

    #[wasm_bindgen_test]
    fn the_three_named_fields_arrive_separately() {
        for (json, expected) in [
            (
                r#"{"kind":"other","message":"workflow rejected metadata"}"#,
                super::PausedReasonData::Other {
                    message: "workflow rejected metadata".to_string(),
                },
            ),
            (
                r#"{"kind":"role_denied","role":"analyst"}"#,
                super::PausedReasonData::RoleDenied {
                    role: Some("analyst".to_string()),
                },
            ),
            (
                r#"{"kind":"role_denied","role":null}"#,
                super::PausedReasonData::RoleDenied { role: None },
            ),
        ] {
            assert_eq!(
                serde_json::from_str::<super::PausedReasonData>(json).unwrap(),
                expected
            );
        }
    }

    /// The mirror must deserialize the exact JSON the backend's
    /// `the_accounts_payload_serializes_the_wire_shape` pins. Character-for-character:
    /// a literal the backend does not emit looks like a guard and proves nothing.
    #[wasm_bindgen_test]
    fn main_page_accounts_data_wire_form_is_verbatim() {
        let data = serde_json::from_str::<super::MainPageAccountsData>(
            r#"{"hosts":[{"host":"open.quiltdata.com","signedIn":true,"currentRole":null,"roles":[],"provisional":true},{"host":"solo.registry.io","signedIn":false,"currentRole":null,"roles":[],"provisional":false}]}"#,
        )
        .unwrap();
        assert_eq!(data.hosts.len(), 2);
        assert!(data.hosts[0].signed_in);
        assert!(
            data.hosts[0].provisional,
            "a signed-in host waits for its role"
        );
        assert_eq!(data.hosts[0].current_role, None);
        assert!(!data.hosts[1].signed_in);
        assert!(
            !data.hosts[1].provisional,
            "a signed-out host is already final"
        );
    }

    #[wasm_bindgen_test]
    fn a_settled_account_arrives_with_its_role_and_alternatives() {
        let host = serde_json::from_str::<super::AccountHostData>(
            r#"{"host":"open.quiltdata.com","signedIn":true,"currentRole":"analyst","roles":["analyst","admin"],"provisional":false}"#,
        )
        .unwrap();
        assert_eq!(host.current_role.as_deref(), Some("analyst"));
        assert_eq!(host.roles, vec!["analyst".to_string(), "admin".to_string()]);
        assert!(!host.provisional);
    }

    #[wasm_bindgen_test]
    fn a_nameless_role_is_null_not_an_empty_string() {
        // R5's wire form. `HostRow` maps this to "Role unavailable"; an empty string
        // would be indistinguishable from a role literally named "".
        let host = serde_json::from_str::<super::AccountHostData>(
            r#"{"host":"open.quiltdata.com","signedIn":true,"currentRole":null,"roles":[],"provisional":false}"#,
        )
        .unwrap();
        assert!(host.signed_in);
        assert_eq!(host.current_role, None);
        assert!(!host.provisional);
    }
}
