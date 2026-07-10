use quilt_uri::S3PackageUri;
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
    /// Package has been pushed — the remote is pinned to its push history
    /// and can't be edited. The toolbar's remote button becomes a read-only
    /// "Show remote" view.
    pub remote_locked: bool,
    pub entries: Vec<EntryData>,
    pub has_remote_entries: bool,
    pub ignored_count: usize,
    pub unmodified_count: usize,
    pub filter_unmodified: bool,
    pub filter_ignored: bool,
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
    pub uri: Option<S3PackageUri>,
    pub remote_display: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePackageResult {
    pub namespace: String,
    pub notification: Option<String>,
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

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshedPackageStatus {
    pub status: String,
    pub has_changes: bool,
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
) -> Result<String, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        message: String,
        metadata: String,
        workflow: WorkflowIntent,
    }
    tauri::invoke(
        "package_commit",
        &Args {
            namespace,
            message,
            metadata,
            workflow,
        },
    )
    .await
}

pub async fn package_push(namespace: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
    }
    tauri::invoke("package_push", &Args { namespace }).await
}

pub async fn package_publish(namespace: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
    }
    tauri::invoke("package_publish", &Args { namespace }).await
}

pub async fn package_commit_and_push(
    namespace: String,
    message: String,
    metadata: String,
    workflow: WorkflowIntent,
) -> Result<String, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        message: String,
        metadata: String,
        workflow: WorkflowIntent,
    }
    tauri::invoke(
        "package_commit_and_push",
        &Args {
            namespace,
            message,
            metadata,
            workflow,
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

pub async fn update_fswatcher_settings(enabled: bool) -> Result<(), String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        enabled: bool,
    }
    tauri::invoke("update_fswatcher_settings", &Args { enabled }).await
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

pub async fn package_pull(namespace: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
    }
    tauri::invoke("package_pull", &Args { namespace }).await
}

pub async fn package_uninstall(namespace: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
    }
    tauri::invoke("package_uninstall", &Args { namespace }).await
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

pub async fn certify_latest(namespace: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
    }
    tauri::invoke("certify_latest", &Args { namespace }).await
}

pub async fn reset_local(namespace: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
    }
    tauri::invoke("reset_local", &Args { namespace }).await
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

// ── Setup ───────────────────────────────────────────────────

pub async fn setup(directory: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        directory: String,
    }
    tauri::invoke("setup", &Args { directory }).await
}

// ── Quiltignore ─────────────────────────────────────────────

pub async fn add_to_quiltignore(namespace: String, pattern: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        pattern: String,
    }
    tauri::invoke("add_to_quiltignore", &Args { namespace, pattern }).await
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

pub async fn open_in_file_browser(namespace: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
    }
    tauri::invoke("open_in_file_browser", &Args { namespace }).await
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
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        path: String,
    }
    tauri::invoke("open_in_default_application", &Args { namespace, path }).await
}

pub async fn reveal_in_file_browser(namespace: String, path: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        path: String,
    }
    tauri::invoke("reveal_in_file_browser", &Args { namespace, path }).await
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
    use super::{CommitViolation, CommitWorkflows, ViolationField, WorkflowInfo, WorkflowIntent};

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
}
