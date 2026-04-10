use serde::{Deserialize, Serialize};

use crate::tauri;

// ── Response types ──────────────────────────────────────────

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackageData {
    pub namespace: String,
    pub uri: String,
    pub status: String,
    pub origin_url: Option<String>,
    pub origin_host: Option<String>,
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
    pub origin_url: Option<String>,
    pub junky_pattern: Option<String>,
    pub ignored_by: Option<String>,
    pub namespace: String,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitData {
    pub namespace: String,
    #[allow(dead_code)]
    pub uri: String,
    pub status: String,
    pub origin_url: Option<String>,
    #[allow(dead_code)]
    pub origin_host: Option<String>,
    pub message: String,
    pub user_meta: String,
    pub user_meta_error: Option<String>,
    pub workflow: Option<WorkflowData>,
    pub entries: Vec<EntryData>,
    pub ignored_count: usize,
    pub unmodified_count: usize,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct WorkflowData {
    pub id: Option<String>,
    pub url: Option<String>,
    #[allow(dead_code)]
    pub config_url: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeData {
    pub namespace: String,
    pub origin_url: Option<String>,
    #[allow(dead_code)]
    pub origin_host: Option<String>,
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
    pub origin_url: Option<String>,
    pub origin_host: Option<String>,
    pub remote_display: Option<String>,
}

#[derive(Clone, Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePackageResult {
    pub namespace: String,
    pub notification: Option<String>,
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

pub async fn handle_remote_package(uri: String) -> Result<RemotePackageResult, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        uri: String,
    }
    tauri::invoke("handle_remote_package", &Args { uri }).await
}

// ── Package actions ─────────────────────────────────────────

pub async fn package_commit(
    namespace: String,
    message: String,
    metadata: String,
    workflow: Option<String>,
) -> Result<String, String> {
    #[derive(Serialize)]
    #[serde(rename_all = "camelCase")]
    struct Args {
        namespace: String,
        message: String,
        metadata: String,
        workflow: Option<String>,
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
    tauri::invoke("package_create", &Args { namespace, source, message }).await
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

// ── Origin/remote ───────────────────────────────────────────

pub async fn set_origin(namespace: String, origin: String) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        origin: String,
    }
    tauri::invoke("set_origin", &Args { namespace, origin }).await
}

pub async fn set_remote(
    namespace: String,
    origin: String,
    bucket: String,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        namespace: String,
        origin: String,
        bucket: String,
    }
    tauri::invoke(
        "set_remote",
        &Args {
            namespace,
            origin,
            bucket,
        },
    )
    .await
}

// ── Auth ────────────────────────────────────────────────────

pub async fn login(
    host: String,
    code: String,
    back: Option<String>,
) -> Result<String, String> {
    #[derive(Serialize)]
    struct Args {
        host: String,
        code: String,
        back: Option<String>,
    }
    tauri::invoke("login", &Args { host, code, back }).await
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
    tauri::invoke(
        "open_in_default_application",
        &Args { namespace, path },
    )
    .await
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
