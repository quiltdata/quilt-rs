use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::str::FromStr;

use rfd::FileDialog;
use serde::Serialize;
use tauri::Manager;
use tauri_plugin_updater::UpdaterExt;
use tokio::sync;

use crate::app;
use crate::commit_message;
use crate::model;
use crate::oauth::OAuthState;
use crate::publish_settings::PublishSettings;
use crate::publish_settings::SharedPublishSettings;
use crate::quilt;
use crate::routes;
use crate::Error;

use crate::changelog;
use crate::model::QuiltModel;
use crate::notify::Notify;
use crate::telemetry::diagnostics;
use crate::telemetry::{mixpanel::LoginFlow, prelude::*, MixpanelEvent};

fn get_default_home_dir(app_handle: &tauri::AppHandle) -> Result<PathBuf, Error> {
    let path_resolver = app_handle.path();
    let user_home = path_resolver.home_dir()?;
    Ok(user_home.join("QuiltSync"))
}

// ── Installed Package data for Leptos UI ──

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackageEntryData {
    pub filename: String,
    pub size: u64,
    pub status: String,
    pub origin_url: Option<String>,
    pub junky_pattern: Option<String>,
    pub ignored_by: Option<String>,
    pub namespace: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackageData {
    pub namespace: String,
    pub uri: String,
    pub status: String,
    pub origin_url: Option<String>,
    pub origin_host: Option<String>,
    pub entries: Vec<InstalledPackageEntryData>,
    pub has_remote_entries: bool,
    pub ignored_count: usize,
    pub unmodified_count: usize,
    pub filter_unmodified: bool,
    pub filter_ignored: bool,
}

async fn get_installed_package_data_from_model(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
    namespace: &quilt::uri::Namespace,
    filter: routes::EntriesFilter,
) -> Result<InstalledPackageData, Error> {
    let installed_package = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(
            namespace.to_owned(),
        ))
    })?;

    let lineage = m.get_installed_package_lineage(&installed_package).await?;

    let (uri, origin_host) =
        crate::debug_tools::resolve_uri_and_host(lineage.remote_uri.as_ref(), namespace);
    if let Some(host) = &origin_host {
        tracing.add_host(host);
    }

    let pkg_status = if lineage.remote_uri.is_none() || origin_host.is_some() {
        match m
            .get_installed_package_status(&installed_package, None)
            .await
        {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!("Failed to get package status: {err}");
                quilt::lineage::InstalledPackageStatus::error()
            }
        }
    } else {
        quilt::lineage::InstalledPackageStatus::error()
    };

    let modified_entries = &pkg_status.changes;
    let installed_paths = &lineage.paths;
    let manifest_entries = m.get_installed_package_records(&installed_package).await?;

    let junky_map: std::collections::HashMap<_, _> = pkg_status
        .junky_changes
        .iter()
        .map(|(p, pat)| (p.clone(), pat.clone()))
        .collect();

    let mut entries_list = Vec::new();
    for (filename, change) in modified_entries {
        let entry_uri = quilt::uri::S3PackageUri {
            path: Some(filename.to_owned()),
            ..uri.clone()
        };
        let origin = match &origin_host {
            Some(host) => entry_uri.display_for_host(host).ok().map(|u| u.to_string()),
            None => None,
        };
        let (status_str, size) = match change {
            quilt::lineage::Change::Added(r) => ("added", r.size),
            quilt::lineage::Change::Modified(r) => ("modified", r.size),
            quilt::lineage::Change::Removed(r) => ("deleted", r.size),
        };
        entries_list.push(InstalledPackageEntryData {
            filename: filename.display().to_string(),
            size,
            status: status_str.to_string(),
            origin_url: origin,
            junky_pattern: junky_map.get(filename).cloned(),
            ignored_by: None,
            namespace: namespace.to_string(),
        });
        if entries_list.len() > 1000 {
            break;
        }
    }
    for filename in installed_paths.keys() {
        if modified_entries.contains_key(filename) {
            continue;
        }
        if let Some(row) = manifest_entries.get(filename) {
            let entry_uri = quilt::uri::S3PackageUri {
                path: Some(filename.to_owned()),
                ..uri.clone()
            };
            let origin = match &origin_host {
                Some(host) => entry_uri.display_for_host(host).ok().map(|u| u.to_string()),
                None => None,
            };
            entries_list.push(InstalledPackageEntryData {
                filename: filename.display().to_string(),
                size: row.size,
                status: "pristine".to_string(),
                origin_url: origin,
                junky_pattern: None,
                ignored_by: None,
                namespace: namespace.to_string(),
            });
        }
        if entries_list.len() > 1000 {
            break;
        }
    }
    for (filename, row) in &manifest_entries {
        if installed_paths.contains_key(filename) || modified_entries.contains_key(filename) {
            continue;
        }
        let entry_uri = quilt::uri::S3PackageUri {
            path: Some(filename.clone()),
            ..uri.clone()
        };
        let origin = match &origin_host {
            Some(host) => entry_uri.display_for_host(host).ok().map(|u| u.to_string()),
            None => None,
        };
        entries_list.push(InstalledPackageEntryData {
            filename: filename.display().to_string(),
            size: row.size,
            status: "remote".to_string(),
            origin_url: origin,
            junky_pattern: None,
            ignored_by: None,
            namespace: namespace.to_string(),
        });
        if entries_list.len() > 1000 {
            break;
        }
    }
    for (filename, pattern, size) in &pkg_status.ignored_files {
        entries_list.push(InstalledPackageEntryData {
            filename: filename.display().to_string(),
            size: *size,
            status: "pristine".to_string(),
            origin_url: None,
            junky_pattern: None,
            ignored_by: Some(pattern.clone()),
            namespace: namespace.to_string(),
        });
        if entries_list.len() > 1000 {
            break;
        }
    }

    entries_list.sort_by(|a, b| a.filename.cmp(&b.filename));

    let origin_url = match &origin_host {
        Some(host) => uri.display_for_host(host).ok().map(|u| u.to_string()),
        None => None,
    };

    // Compute counts from the full source data, not the capped entries_list,
    // so the filter toolbar is shown even when the list is truncated.
    let ignored_count = pkg_status.ignored_files.len();
    let unmodified_count = installed_paths
        .keys()
        .filter(|f| !modified_entries.contains_key(*f))
        .count()
        + manifest_entries
            .keys()
            .filter(|f| !installed_paths.contains_key(*f) && !modified_entries.contains_key(*f))
            .count();

    let has_remote_entries = manifest_entries
        .keys()
        .any(|f| !installed_paths.contains_key(f) && !modified_entries.contains_key(f));

    let status_str = match pkg_status.upstream_state {
        quilt::lineage::UpstreamState::UpToDate => "up_to_date",
        quilt::lineage::UpstreamState::Ahead => "ahead",
        quilt::lineage::UpstreamState::Behind => "behind",
        quilt::lineage::UpstreamState::Diverged => "diverged",
        quilt::lineage::UpstreamState::Local => "local",
        quilt::lineage::UpstreamState::Error => "error",
    };

    Ok(InstalledPackageData {
        namespace: namespace.to_string(),
        uri: uri.to_string(),
        status: status_str.to_string(),
        origin_url,
        origin_host: origin_host.map(|h| h.to_string()),
        entries: entries_list,
        has_remote_entries,
        ignored_count,
        unmodified_count,
        filter_unmodified: filter.unmodified,
        filter_ignored: filter.ignored,
    })
}

#[tauri::command]
pub async fn get_installed_package_data(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
    filter: Option<String>,
) -> Result<InstalledPackageData, String> {
    let namespace: quilt::uri::Namespace = namespace
        .try_into()
        .map_err(|e: quilt::Error| e.to_string())?;
    let filter = filter
        .map(|f| routes::EntriesFilter::from_filter_str(&f))
        .unwrap_or_default();

    get_installed_package_data_from_model(&*m, &tracing, &namespace, filter)
        .await
        .map_err(|e| e.to_frontend_string())
}

// ── Settings data for Leptos UI ──

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PublishSettingsData {
    pub message_template: String,
    pub default_workflow: String,
    pub default_metadata: String,
}

impl From<PublishSettings> for PublishSettingsData {
    fn from(s: PublishSettings) -> Self {
        Self {
            message_template: s.message_template.unwrap_or_default(),
            default_workflow: s.default_workflow.unwrap_or_default(),
            default_metadata: s.default_metadata.unwrap_or_default(),
        }
    }
}

#[derive(Serialize)]
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
    pub changelog: Vec<changelog::ChangelogEntry>,
    pub publish: PublishSettingsData,
}

#[tauri::command]
pub async fn get_settings_data(
    m: tauri::State<'_, model::Model>,
    app: tauri::State<'_, app::App>,
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    publish: tauri::State<'_, SharedPublishSettings>,
) -> Result<SettingsData, String> {
    let app: &app::App = &app;

    let app_handle = app_handle.lock().await;
    let data_dir = app_handle
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?;

    let home_dir = m
        .get_quilt()
        .lock()
        .await
        .get_home()
        .await
        .ok()
        .map(|h| h.as_ref().display().to_string());

    let auth_hosts = quilt::paths::list_auth_hosts(&data_dir);
    let log_level = tracing.log_level();
    let publish_data = PublishSettingsData::from(publish.read().await.clone());

    Ok(SettingsData {
        version: app.version.to_string(),
        home_dir,
        data_dir: data_dir.display().to_string(),
        auth_hosts,
        log_level,
        logs_dir: app.logs_dir.path().display().to_string(),
        logs_dir_is_temporary: matches!(app.logs_dir, crate::telemetry::LogsDir::Temporary(_)),
        os: std::env::consts::OS.to_string(),
        changelog: changelog::latest_entries(),
        publish: publish_data,
    })
}

#[tauri::command]
pub async fn update_publish_settings(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    publish: tauri::State<'_, SharedPublishSettings>,
    message_template: String,
    default_workflow: String,
    default_metadata: String,
) -> Result<(), String> {
    // Validate metadata is parseable JSON (or empty = no metadata).
    if !default_metadata.is_empty() {
        serde_json::from_str::<serde_json::Value>(&default_metadata)
            .map_err(|e| format!("Invalid metadata JSON: {e}"))?;
    }

    let new = PublishSettings {
        message_template: opt_from_string(message_template),
        default_workflow: opt_from_string(default_workflow),
        default_metadata: opt_from_string(default_metadata),
    };

    let app_handle = app_handle.lock().await;
    let data_dir = app_handle
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?;

    new.save(&data_dir).await.map_err(|e| e.to_string())?;
    *publish.write().await = new;
    Ok(())
}

fn opt_from_string(s: String) -> Option<String> {
    if s.trim().is_empty() {
        None
    } else {
        Some(s)
    }
}

// ── Login data for Leptos UI ──

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginData {
    pub host: String,
    pub back: String,
    pub catalog_url: String,
}

#[tauri::command]
pub async fn get_login_data(host: String, back: String) -> Result<LoginData, String> {
    let catalog_url = format!("https://{host}/code");
    Ok(LoginData {
        host,
        back,
        catalog_url,
    })
}

// ── Login error data for Leptos UI ──

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LoginErrorData {
    pub title: String,
    pub message: String,
    pub login_host: String,
}

#[tauri::command]
pub async fn get_login_error_data(
    host: String,
    title: Option<String>,
    error: String,
) -> Result<LoginErrorData, String> {
    Ok(LoginErrorData {
        title: title.unwrap_or_else(|| "Login failed".into()),
        message: error,
        login_host: host,
    })
}

// ── Merge data for Leptos UI ──

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeData {
    pub namespace: String,
    pub origin_url: Option<String>,
    pub origin_host: Option<String>,
}

async fn get_merge_data_from_model(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
    namespace: &quilt::uri::Namespace,
) -> Result<MergeData, Error> {
    let installed_package = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(
            namespace.to_owned(),
        ))
    })?;

    let lineage = m.get_installed_package_lineage(&installed_package).await?;

    let (uri, origin_host) =
        crate::debug_tools::resolve_uri_and_host(lineage.remote_uri.as_ref(), namespace);
    if let Some(host) = &origin_host {
        tracing.add_host(host);
    }

    let origin_url = origin_host
        .as_ref()
        .and_then(|host| uri.display_for_host(host).ok())
        .map(|u| u.to_string());

    Ok(MergeData {
        namespace: namespace.to_string(),
        origin_url,
        origin_host: origin_host.map(|h| h.to_string()),
    })
}

#[tauri::command]
pub async fn get_merge_data(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<MergeData, String> {
    let namespace: quilt::uri::Namespace = namespace
        .try_into()
        .map_err(|e: quilt::Error| e.to_string())?;

    get_merge_data_from_model(&*m, &tracing, &namespace)
        .await
        .map_err(|e| e.to_frontend_string())
}

// ── Commit data for Leptos UI ──

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitData {
    pub namespace: String,
    pub uri: String,
    pub status: String,
    pub origin_url: Option<String>,
    pub origin_host: Option<String>,
    pub message: String,
    pub user_meta: String,
    pub user_meta_error: Option<String>,
    pub workflow: Option<CommitWorkflowData>,
    pub entries: Vec<InstalledPackageEntryData>,
    pub ignored_count: usize,
    pub unmodified_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitWorkflowData {
    pub id: Option<String>,
    pub url: Option<String>,
    pub config_url: Option<String>,
}

async fn get_commit_data_from_model(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
    namespace: &quilt::uri::Namespace,
) -> Result<CommitData, Error> {
    let installed_package = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(
            namespace.to_owned(),
        ))
    })?;

    let status = m
        .get_installed_package_status(&installed_package, None)
        .await?;

    let pkg_status_str = match status.upstream_state {
        quilt::lineage::UpstreamState::UpToDate => "up_to_date",
        quilt::lineage::UpstreamState::Ahead => "ahead",
        quilt::lineage::UpstreamState::Behind => "behind",
        quilt::lineage::UpstreamState::Diverged => "diverged",
        quilt::lineage::UpstreamState::Local => "local",
        quilt::lineage::UpstreamState::Error => "error",
    };

    let lineage = m.get_installed_package_lineage(&installed_package).await?;

    let (uri, origin_host) =
        crate::debug_tools::resolve_uri_and_host(lineage.remote_uri.as_ref(), namespace);
    if let Some(host) = &origin_host {
        tracing.add_host(host);
    }

    // Build lookup maps for junky files
    let junky_map: std::collections::HashMap<_, _> = status
        .junky_changes
        .iter()
        .map(|(p, pat)| (p.clone(), pat.clone()))
        .collect();

    // Modified entries
    let mut entries_list = Vec::new();
    for (filename, change) in &status.changes {
        let entry_uri = quilt::uri::S3PackageUri {
            path: Some(filename.clone()),
            ..uri.clone()
        };
        let origin = match &origin_host {
            Some(host) => entry_uri.display_for_host(host).ok().map(|u| u.to_string()),
            None => None,
        };
        let (status_str, size) = match change {
            quilt::lineage::Change::Added(r) => ("added", r.size),
            quilt::lineage::Change::Modified(r) => ("modified", r.size),
            quilt::lineage::Change::Removed(r) => ("deleted", r.size),
        };
        entries_list.push(InstalledPackageEntryData {
            filename: filename.display().to_string(),
            size,
            status: status_str.to_string(),
            origin_url: origin,
            junky_pattern: junky_map.get(filename).cloned(),
            ignored_by: None,
            namespace: namespace.to_string(),
        });
        if entries_list.len() > 1000 {
            break;
        }
    }

    // Unmodified entries (from manifest, not changed)
    let manifest_entries = m.get_installed_package_records(&installed_package).await?;
    for (filename, row) in &manifest_entries {
        if status.changes.contains_key(filename) {
            continue;
        }
        let entry_uri = quilt::uri::S3PackageUri {
            path: Some(filename.clone()),
            ..uri.clone()
        };
        let origin = match &origin_host {
            Some(host) => entry_uri.display_for_host(host).ok().map(|u| u.to_string()),
            None => None,
        };
        entries_list.push(InstalledPackageEntryData {
            filename: filename.display().to_string(),
            size: row.size,
            status: if lineage.paths.contains_key(filename) {
                "pristine"
            } else {
                "remote"
            }
            .to_string(),
            origin_url: origin,
            junky_pattern: None,
            ignored_by: None,
            namespace: namespace.to_string(),
        });
        if entries_list.len() > 1000 {
            break;
        }
    }

    // Ignored files
    for (filename, pattern, size) in &status.ignored_files {
        entries_list.push(InstalledPackageEntryData {
            filename: filename.display().to_string(),
            size: *size,
            status: "pristine".to_string(),
            origin_url: None,
            junky_pattern: None,
            ignored_by: Some(pattern.clone()),
            namespace: namespace.to_string(),
        });
        if entries_list.len() > 1000 {
            break;
        }
    }

    entries_list.sort_by(|a, b| a.filename.cmp(&b.filename));

    // Compute counts from the full source data, not the capped entries_list,
    // so the filter toolbar is shown even when the list is truncated.
    let ignored_count = status.ignored_files.len();
    let unmodified_count = manifest_entries
        .keys()
        .filter(|f| !status.changes.contains_key(*f))
        .count();

    let origin_url = origin_host
        .as_ref()
        .and_then(|host| uri.display_for_host(host).ok())
        .map(|u| u.to_string());

    // Generate commit message from changes
    let message = crate::commit_message::generate(&status.changes);

    // Load remote manifest for user_meta and workflow
    let (user_meta, user_meta_error, workflow) =
        match lineage.remote_uri.as_ref().filter(|r| !r.hash.is_empty()) {
            Some(remote_uri) => {
                let remote_manifest = m.browse_remote_manifest(remote_uri).await?;
                let (meta_value, meta_error) = match &remote_manifest.header.user_meta {
                    Some(meta) => match serde_json::to_string(meta) {
                        Ok(v) => (v, None),
                        Err(_) => (String::new(), Some("Failed to stringify meta".to_string())),
                    },
                    None => (String::new(), None),
                };
                let workflow = origin_host.as_ref().and_then(|host| {
                    remote_manifest
                        .header
                        .workflow
                        .as_ref()
                        .map(|w| CommitWorkflowData {
                            id: w.id.as_ref().map(|id| id.id.clone()),
                            url: w.config.display_for_host(host).ok().map(|u| u.to_string()),
                            config_url: None,
                        })
                });
                (meta_value, meta_error, workflow)
            }
            None => (String::new(), None, None),
        };

    Ok(CommitData {
        namespace: namespace.to_string(),
        uri: uri.to_string(),
        status: pkg_status_str.to_string(),
        origin_url,
        origin_host: origin_host.map(|h| h.to_string()),
        message,
        user_meta,
        user_meta_error,
        workflow,
        entries: entries_list,
        ignored_count,
        unmodified_count,
    })
}

#[tauri::command]
pub async fn get_commit_data(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<CommitData, String> {
    let namespace: quilt::uri::Namespace = namespace
        .try_into()
        .map_err(|e: quilt::Error| e.to_string())?;

    get_commit_data_from_model(&*m, &tracing, &namespace)
        .await
        .map_err(|e| e.to_frontend_string())
}

// ── Installed Packages List data for Leptos UI ──

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackagesListData {
    pub packages: Vec<InstalledPackageListItem>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackageListItem {
    pub namespace: String,
    pub status: String,
    pub has_changes: bool,
    pub origin_url: Option<String>,
    pub origin_host: Option<String>,
    pub remote_display: Option<String>,
}

async fn get_installed_packages_list_data_from_model(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
) -> Result<InstalledPackagesListData, Error> {
    let list = m.get_installed_packages_list().await?;
    let mut packages = Vec::new();
    for installed_package in list {
        match load_package_item(m, tracing, &installed_package).await {
            Ok(item) => packages.push(item),
            Err(err) => {
                tracing::warn!(
                    "Failed to load package {}: {err}",
                    installed_package.namespace,
                );
            }
        }
    }
    Ok(InstalledPackagesListData { packages })
}

async fn load_package_item(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
    installed_package: &quilt::InstalledPackage,
) -> Result<InstalledPackageListItem, Error> {
    let lineage = m.get_installed_package_lineage(installed_package).await?;

    let remote_uri = match lineage.remote_uri.as_ref() {
        Some(uri) => uri,
        None => {
            return Ok(InstalledPackageListItem {
                namespace: installed_package.namespace.to_string(),
                status: "local".to_string(),
                has_changes: false,
                origin_url: None,
                origin_host: None,
                remote_display: None,
            });
        }
    };

    if remote_uri.origin.is_none() {
        return Ok(InstalledPackageListItem {
            namespace: installed_package.namespace.to_string(),
            status: "error".to_string(),
            has_changes: false,
            origin_url: None,
            origin_host: None,
            remote_display: Some(remote_uri.to_string()),
        });
    }

    let origin_host = crate::debug_tools::try_remote_origin_host(remote_uri)?;
    tracing.add_host(&origin_host);
    let uri = quilt::uri::S3PackageUri::from(remote_uri);
    let origin_url = uri.display_for_host(&origin_host)?;
    let remote_display = remote_uri.to_string();
    let upstream_state: quilt::lineage::UpstreamState = lineage.into();
    let has_changes = false; // Refined by refresh_package_status

    Ok(InstalledPackageListItem {
        namespace: installed_package.namespace.to_string(),
        status: upstream_state.to_string(),
        has_changes,
        origin_url: Some(origin_url.to_string()),
        origin_host: Some(origin_host.to_string()),
        remote_display: Some(remote_display),
    })
}

#[tauri::command]
pub async fn get_installed_packages_list_data(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<InstalledPackagesListData, String> {
    get_installed_packages_list_data_from_model(&*m, &tracing)
        .await
        .map_err(|e| e.to_frontend_string())
}

// ── Refresh package status (heavy phase) ──

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RefreshedPackageStatus {
    pub status: String,
    pub has_changes: bool,
}

async fn refresh_package_status_from_model(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
    namespace: &quilt::uri::Namespace,
) -> Result<RefreshedPackageStatus, Error> {
    let installed_package = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(
            namespace.to_owned(),
        ))
    })?;

    let lineage = m.get_installed_package_lineage(&installed_package).await?;

    let Some(remote_uri) = lineage.remote_uri.as_ref() else {
        let has_changes = match m
            .get_installed_package_status(&installed_package, None)
            .await
        {
            Ok(s) => !s.changes.is_empty(),
            Err(err) => {
                tracing::warn!(
                    "Failed to get status for {}: {err}",
                    installed_package.namespace,
                );
                false
            }
        };
        return Ok(RefreshedPackageStatus {
            status: "local".to_string(),
            has_changes,
        });
    };
    if remote_uri.origin.is_none() {
        return Ok(RefreshedPackageStatus {
            status: "error".to_string(),
            has_changes: false,
        });
    }

    if let Ok(host) = crate::debug_tools::try_remote_origin_host(remote_uri) {
        tracing.add_host(&host);
    }

    let (upstream_state, has_changes) = match m
        .get_installed_package_status(&installed_package, None)
        .await
    {
        Ok(s) => (s.upstream_state, !s.changes.is_empty()),
        Err(err) => {
            tracing::warn!(
                "Failed to get status for {}: {err}",
                installed_package.namespace,
            );
            (quilt::lineage::UpstreamState::Error, false)
        }
    };

    Ok(RefreshedPackageStatus {
        status: upstream_state.to_string(),
        has_changes,
    })
}

#[tauri::command]
pub async fn refresh_package_status(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<RefreshedPackageStatus, String> {
    let namespace: quilt::uri::Namespace = namespace
        .try_into()
        .map_err(|e: quilt::Error| e.to_string())?;

    refresh_package_status_from_model(&*m, &tracing, &namespace)
        .await
        .map_err(|e| e.to_frontend_string())
}

// ── Setup data for Leptos UI ──

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetupData {
    pub default_home: String,
}

#[tauri::command]
pub async fn get_setup_data(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
) -> Result<SetupData, String> {
    let app_handle = app_handle.lock().await;
    let home = get_default_home_dir(&app_handle).map_err(|e| e.to_string())?;
    Ok(SetupData {
        default_home: home.display().to_string(),
    })
}

async fn package_commit_command(
    m: &model::Model,
    namespace: &str,
    message: &str,
    metadata: &str,
    workflow: Option<String>,
) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    if message.is_empty() {
        return Err(Error::Commit("Message is required".to_string()));
    }

    model::package_commit(m, namespace.clone(), message, metadata, workflow, None).await?;
    Ok(())
}

#[tauri::command]
pub async fn package_commit(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
    message: String,
    metadata: String,
    workflow: Option<String>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackageCommitted).await;

    let msg_init = format!("Committing package {namespace}");
    let msg_ok = format!("Successfully committed {namespace}");
    let msg_err = |err: &Error| format!("Failed to commit: {err}");

    Notify::new(msg_init).map(
        package_commit_command(&m, &namespace, &message, &metadata, workflow).await,
        msg_ok,
        msg_err,
    )
}

async fn open_directory_picker_command(app_handle: &tauri::AppHandle) -> Result<PathBuf, Error> {
    let paths = app_handle.path();
    let home_dir = paths.home_dir()?;

    let canonical_home = home_dir.join("QuiltSync");
    let canonical_home_already_exists = canonical_home.exists();

    if !canonical_home_already_exists {
        if let Err(e) = fs::create_dir_all(&canonical_home) {
            return Err(Error::from(e));
        }
    }

    let window = app_handle
        .get_webview_window("main")
        .ok_or(crate::error::TauriUiError::Window)?;

    let result = match FileDialog::new()
        .set_directory(&canonical_home)
        .set_parent(&window)
        .pick_folder()
    {
        Some(path) => {
            debug!("Successfully selected {}", path.display());
            Ok(path)
        }
        None => {
            debug!("User cancelled directory selection");
            Err(Error::TauriUi(crate::error::TauriUiError::UserCancelled))
        }
    };

    // Cleanup logic: remove temporary canonical directory if needed
    let should_delete_canonical = match &result {
        Ok(path) => path != &canonical_home,
        Err(_) => true,
    };

    if !canonical_home_already_exists && should_delete_canonical {
        if let Err(e) = fs::remove_dir(&canonical_home) {
            error!("Failed to remove temporary QuiltSync directory: {}", e);
        }
    }

    result
}

#[tauri::command]
pub async fn open_directory_picker(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<PathBuf, String> {
    tracing.track(MixpanelEvent::DirectoryPickerOpened).await;

    let app_handle = &app_handle.lock().await;

    match open_directory_picker_command(app_handle).await {
        Ok(path) => Ok(path),
        Err(err) => {
            error!("Failed to open directory picker: {}", err);
            Err(err.to_string())
        }
    }
}

async fn erase_auth_command(app_handle: &tauri::AppHandle, host: &str) -> Result<(), Error> {
    let local_data_dir = app_handle.path().app_local_data_dir()?;
    let auth_dir = local_data_dir.join(quilt::paths::AUTH_DIR);

    if host.is_empty() {
        // Global erase (backward compat)
        if auth_dir.exists() {
            std::fs::remove_dir_all(&auth_dir)?;
        }
    } else {
        // Per-host erase — canonicalize and verify containment
        let host_dir = auth_dir.join(host);
        if host_dir.exists() {
            let canonical = host_dir.canonicalize()?;
            let canonical_auth = auth_dir.canonicalize()?;
            if !canonical.starts_with(&canonical_auth) {
                return Err(Error::General(format!("Invalid host: {host}")));
            }
            std::fs::remove_dir_all(&canonical)?;
        }
    }
    Ok(())
}

#[tauri::command]
pub async fn erase_auth(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    host: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::AuthErased).await;

    let app_handle = app_handle.lock().await;

    let msg_init = format!("Erasing auth for {host}");
    let msg_ok = format!("Successfully erased auth for {host}");
    let msg_err = |err: &Error| format!("Failed to erase auth: {err}");

    Notify::new(msg_init).map(
        erase_auth_command(&app_handle, &host).await,
        msg_ok,
        msg_err,
    )
}

async fn debug_dot_quilt_command(app_handle: &tauri::AppHandle) -> Result<(), Error> {
    let local_data_dir = app_handle.path().app_local_data_dir()?;
    let dot_quilt_dir = local_data_dir.join(".quilt");

    opener::open_browser(&dot_quilt_dir)?;
    Ok(())
}

#[tauri::command]
pub async fn debug_dot_quilt(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::DebugDotQuiltOpened).await;
    let app_handle = app_handle.lock().await;

    let msg_init = "Opening .quilt directory".to_string();
    let msg_ok = "Successfully opened .quilt directory".to_string();
    let msg_err = |err: &Error| format!("Failed to open directory: {err}");

    Notify::new(msg_init).map(debug_dot_quilt_command(&app_handle).await, msg_ok, msg_err)
}

async fn debug_logs_command(app: &app::App) -> Result<(), Error> {
    let logs_dir = &app.logs_dir;
    opener::open_browser(logs_dir.path())?;
    Ok(())
}

#[tauri::command]
pub async fn debug_logs(
    app: tauri::State<'_, app::App>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::DebugLogsOpened).await;
    let app: &app::App = &app;

    let msg_init = "Opening logs directory".to_string();
    let msg_ok = "Successfully opened logs directory".to_string();
    let msg_err = |err: &Error| format!("Failed to open logs directory: {err}");

    Notify::new(msg_init).map(debug_logs_command(app).await, msg_ok, msg_err)
}

async fn open_home_dir_command(m: &model::Model) -> Result<(), Error> {
    let home = m.get_quilt().lock().await.get_home().await?;
    let home_path: &std::path::PathBuf = home.as_ref();
    opener::open_browser(home_path)?;
    Ok(())
}

#[tauri::command]
pub async fn open_home_dir(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::FileBrowserOpened).await;

    let msg_init = "Opening home directory".to_string();
    let msg_ok = "Successfully opened home directory".to_string();
    let msg_err = |err: &Error| format!("Failed to open home directory: {err}");

    Notify::new(msg_init).map(open_home_dir_command(&m).await, msg_ok, msg_err)
}

async fn open_data_dir_command(app_handle: &tauri::AppHandle) -> Result<(), Error> {
    let local_data_dir = app_handle.path().app_local_data_dir()?;
    opener::open_browser(&local_data_dir)?;
    Ok(())
}

#[tauri::command]
pub async fn open_data_dir(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::FileBrowserOpened).await;
    let app_handle = app_handle.lock().await;

    let msg_init = "Opening data directory".to_string();
    let msg_ok = "Successfully opened data directory".to_string();
    let msg_err = |err: &Error| format!("Failed to open data directory: {err}");

    Notify::new(msg_init).map(open_data_dir_command(&app_handle).await, msg_ok, msg_err)
}

async fn collect_diagnostic_logs_command(
    app_handle: &tauri::AppHandle,
    m: &model::Model,
    app: &app::App,
) -> Result<PathBuf, Error> {
    let info = diagnostics::collect(app_handle, m, app).await?;
    tokio::task::spawn_blocking(move || diagnostics::save_diagnostic_zip(info))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub async fn collect_diagnostic_logs(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    m: tauri::State<'_, model::Model>,
    app: tauri::State<'_, app::App>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::DiagnosticLogsSaved).await;
    let app_handle = app_handle.lock().await;
    let app: &app::App = &app;

    match collect_diagnostic_logs_command(&app_handle, &m, app).await {
        Ok(zip_path) => Ok(zip_path.display().to_string()),
        Err(err) => Err(err.to_string()),
    }
}

#[tauri::command]
pub async fn send_crash_report(
    zip_path: String,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::CrashReportSent).await;

    let zip_path = PathBuf::from(zip_path);
    if zip_path.file_name() != Some("quiltsync-diagnostic.zip".as_ref()) {
        return Err("Invalid diagnostic zip filename".to_string());
    }

    let msg_init = "Sending crash report".to_string();
    let msg_ok = "Successfully sent crash report".to_string();
    let msg_err = |err: &Error| format!("Failed to send crash report: {err}");

    let result =
        tokio::task::spawn_blocking(move || diagnostics::send_crash_report(zip_path.as_path()))
            .await
            .map_err(|e| Error::General(e.to_string()))
            .and_then(|r| r);

    Notify::new(msg_init).map(result, msg_ok, msg_err)
}

async fn reveal_in_file_browser_command(
    m: &model::Model,
    namespace: &str,
    path: &str,
) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    m.reveal_in_file_browser(&namespace, &PathBuf::from(path))
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn reveal_in_file_browser(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
    path: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::FileRevealed).await;

    let msg_init = format!("Revealing {path} in file browser for {namespace}");
    let msg_ok = format!("Successfully opened {path} in file browser");
    let msg_err = |err: &Error| format!("Failed to open directory: {err}");

    Notify::new(msg_init).map(
        reveal_in_file_browser_command(&m, &namespace, &path).await,
        msg_ok,
        msg_err,
    )
}

async fn open_in_file_browser_command(m: &model::Model, namespace: &str) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    m.open_in_file_browser(&namespace).await?;
    Ok(())
}

#[tauri::command]
pub async fn open_in_file_browser(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::FileBrowserOpened).await;

    let msg_init = format!("Opening file manager for {namespace}");
    let msg_ok = format!("Successfully opened file manager for {namespace}");
    let msg_err = |err: &Error| format!("Failed to open file manager: {err}");

    Notify::new(msg_init).map(
        open_in_file_browser_command(&m, &namespace).await,
        msg_ok,
        msg_err,
    )
}

async fn open_in_default_application_command(
    m: &model::Model,
    namespace: &str,
    path: &str,
) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    m.open_in_default_application(&namespace, &PathBuf::from(path))
        .await?;
    Ok(())
}

#[tauri::command]
pub async fn open_in_default_application(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
    path: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::DefaultApplicationOpened).await;

    let msg_init = format!("Opening {path} with default application for {namespace}");
    let msg_ok = format!("Successfully opened {path} with default application");
    let msg_err = |err: &Error| format!("Failed to open application: {err}");

    Notify::new(msg_init).map(
        open_in_default_application_command(&m, &namespace, &path).await,
        msg_ok,
        msg_err,
    )
}

async fn open_in_web_browser_command(url: &str) -> Result<(), Error> {
    model::open_in_web_browser(url)?;
    Ok(())
}

#[tauri::command]
pub async fn open_in_web_browser(
    url: String,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::WebBrowserOpened).await;
    let msg_init = format!("Opening URL {url}");
    let msg_ok = format!("Successfully opened {url}");
    let msg_err = |err: &Error| format!("Failed to open URL: {err}");

    Notify::new(msg_init).map(open_in_web_browser_command(&url).await, msg_ok, msg_err)
}

async fn certify_latest_command(m: &model::Model, namespace: &str) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    model::package_revision_certify_latest(m, namespace.clone()).await?;
    Ok(())
}

#[tauri::command]
pub async fn certify_latest(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::LatestCertified).await;

    let msg_init = format!("Certifying latest for {namespace}");
    let msg_ok = format!("Successfully certified latest for {namespace}");
    let msg_err = |err: &Error| format!("Failed to certify latest: {err}");

    Notify::new(msg_init).map(
        certify_latest_command(&m, &namespace).await,
        msg_ok,
        msg_err,
    )
}

async fn reset_local_command(m: &model::Model, namespace: &str) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    model::package_revision_reset_local(m, namespace.clone()).await?;
    Ok(())
}

#[tauri::command]
pub async fn reset_local(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::LocalReset).await;

    let msg_init = format!("Resetting local for {namespace}");
    let msg_ok = format!("Successfully reset local for {namespace}");
    let msg_err = |err: &Error| format!("Failed to reset local: {err}");

    Notify::new(msg_init).map(reset_local_command(&m, &namespace).await, msg_ok, msg_err)
}

async fn package_push_command(
    m: &model::Model,
    namespace: &str,
) -> Result<quilt::PushOutcome, Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    model::package_push(m, &namespace, None).await
}

#[tauri::command]
pub async fn package_push(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackagePushed).await;

    let msg_init = format!("Pushing package {namespace}");

    let result = package_push_command(&m, &namespace).await;
    // TODO: push-not-certified should be surfaced as a warning, not a success.
    // Currently both outcomes go through the success path because converting to
    // Err skips on_done()/refetch and leaves the UI stale.
    let msg_ok = match &result {
        Ok(outcome) if outcome.certified_latest => {
            format!("Successfully pushed package {namespace}")
        }
        Ok(_) => {
            format!("Pushed {namespace}, but could not update latest: remote has newer changes")
        }
        _ => String::new(),
    };
    let msg_err = |err: &Error| format!("Failed to push package: {err}");

    Notify::new(msg_init).map(result.map(|_| ()), msg_ok, msg_err)
}

async fn package_publish_command(
    m: &model::Model,
    settings: &SharedPublishSettings,
    namespace: &str,
    status: &quilt::lineage::InstalledPackageStatus,
) -> Result<quilt::PushOutcome, Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    let settings = settings.read().await.clone();

    let changes_summary = commit_message::generate(&status.changes);
    let message = commit_message::render_publish_message(
        settings.message_template.as_deref().unwrap_or_default(),
        &commit_message::PublishMessageContext {
            namespace: &namespace,
            changes_summary,
        },
    );
    let metadata = settings.default_metadata.clone().unwrap_or_default();
    let workflow = settings.default_workflow.clone();

    model::package_publish(m, namespace, &message, &metadata, workflow, None).await
}

#[tauri::command]
pub async fn package_publish(
    m: tauri::State<'_, model::Model>,
    settings: tauri::State<'_, SharedPublishSettings>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackagePublished).await;

    let namespace_parsed: quilt::uri::Namespace = match namespace.as_str().try_into() {
        Ok(n) => n,
        Err(e) => return Err(Error::from(e).to_frontend_string()),
    };
    let status = match get_status_for_publish(&m, &namespace_parsed).await {
        Ok(s) => s,
        Err(e) => return Err(e.to_frontend_string()),
    };
    let committed = !status.changes.is_empty();

    let msg_init = format!("Publishing package {namespace}");
    let result = package_publish_command(&m, &settings, &namespace, &status).await;

    if committed {
        tracing.track(MixpanelEvent::PackageCommitted).await;
    }
    if result.is_ok() {
        tracing.track(MixpanelEvent::PackagePushed).await;
    }

    let msg_ok = match &result {
        Ok(outcome) if outcome.certified_latest => {
            format!("Successfully published package {namespace}")
        }
        Ok(_) => {
            format!("Published {namespace}, but could not update latest: remote has newer changes")
        }
        _ => String::new(),
    };
    let msg_err = |err: &Error| format!("Failed to publish package: {err}");

    Notify::new(msg_init).map(result.map(|_| ()), msg_ok, msg_err)
}

async fn get_status_for_publish(
    m: &model::Model,
    namespace: &quilt::uri::Namespace,
) -> Result<quilt::lineage::InstalledPackageStatus, Error> {
    let installed = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(namespace.clone()))
    })?;
    m.get_installed_package_status(&installed, None).await
}

async fn package_commit_and_push_command(
    m: &model::Model,
    namespace: &str,
    message: &str,
    metadata: &str,
    workflow: Option<String>,
) -> Result<quilt::PushOutcome, Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    if message.is_empty() {
        return Err(Error::Commit("Message is required".to_string()));
    }
    model::package_publish(m, namespace, message, metadata, workflow, None).await
}

#[tauri::command]
pub async fn package_commit_and_push(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
    message: String,
    metadata: String,
    workflow: Option<String>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackagePublished).await;
    tracing.track(MixpanelEvent::PackageCommitted).await;

    let msg_init = format!("Publishing package {namespace}");
    let result =
        package_commit_and_push_command(&m, &namespace, &message, &metadata, workflow).await;

    if result.is_ok() {
        tracing.track(MixpanelEvent::PackagePushed).await;
    }

    let msg_ok = match &result {
        Ok(outcome) if outcome.certified_latest => {
            format!("Successfully published package {namespace}")
        }
        Ok(_) => {
            format!("Published {namespace}, but could not update latest: remote has newer changes")
        }
        _ => String::new(),
    };
    let msg_err = |err: &Error| format!("Failed to publish package: {err}");

    Notify::new(msg_init).map(result.map(|_| ()), msg_ok, msg_err)
}

async fn package_pull_command(m: &model::Model, namespace: &str) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    model::package_pull(m, &namespace, None).await?;
    Ok(())
}

#[tauri::command]
pub async fn package_pull(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackagePulled).await;

    let msg_init = format!("Pulling package {namespace}");
    let msg_ok = format!("Successfully pulled package {namespace}");
    let msg_err = |err: &Error| format!("Failed to pull package: {err}");

    Notify::new(msg_init).map(package_pull_command(&m, &namespace).await, msg_ok, msg_err)
}

async fn package_uninstall_command(m: &model::Model, namespace: &str) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    model::package_uninstall(m, namespace.clone()).await?;
    Ok(())
}

#[tauri::command]
pub async fn package_uninstall(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackageUninstalled).await;

    let msg_init = format!("Uninstalling package {namespace}");
    let msg_ok = format!("Successfully uninstalled package {namespace}");
    let msg_err = |err: &Error| format!("Failed to uninstall package: {err}");

    Notify::new(msg_init).map(
        package_uninstall_command(&m, &namespace).await,
        msg_ok,
        msg_err,
    )
}

async fn set_origin_command(m: &model::Model, namespace: &str, origin: &str) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    let origin = quilt::uri::Host::from_str(origin)?;
    model::set_origin(m, &namespace, origin).await?;
    Ok(())
}

#[tauri::command]
pub async fn set_origin(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
    origin: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::OriginSet).await;

    let msg_init = format!("Setting origin for {namespace}");
    let msg_ok = format!("Successfully set origin for {namespace}");
    let msg_err = |err: &Error| format!("Failed to set origin: {err}");

    Notify::new(msg_init).map(
        set_origin_command(&m, &namespace, &origin).await,
        msg_ok,
        msg_err,
    )
}

async fn set_remote_command(
    m: &model::Model,
    namespace: &str,
    origin: &str,
    bucket: &str,
) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    let origin = quilt::uri::Host::from_str(origin)?;
    model::set_remote(m, &namespace, origin, bucket.to_string()).await?;
    Ok(())
}

#[tauri::command]
pub async fn set_remote(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
    origin: String,
    bucket: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::RemoteSet).await;

    let msg_init = format!("Setting remote for {namespace}");
    let msg_ok = format!("Successfully set remote for {namespace}");
    let msg_err = |err: &Error| format!("Failed to set remote: {err}");

    Notify::new(msg_init).map(
        set_remote_command(&m, &namespace, &origin, &bucket).await,
        msg_ok,
        msg_err,
    )
}

async fn package_create_command(
    m: &model::Model,
    namespace: &str,
    source: Option<String>,
    message: Option<String>,
) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    let source = source.map(PathBuf::from);
    model::package_create(m, namespace, source, message).await?;
    Ok(())
}

#[tauri::command]
pub async fn package_create(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
    source: Option<String>,
    message: Option<String>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackageCreated).await;

    let msg_init = format!("Creating package {namespace}");
    let msg_ok = format!("Successfully created package {namespace}");
    let msg_err = |err: &Error| format!("Failed to create package: {err}");

    Notify::new(msg_init).map(
        package_create_command(&m, &namespace, source, message).await,
        msg_ok,
        msg_err,
    )
}

/// Navigate to a page after successful login.
pub(crate) fn navigate_after_login(
    app_handle: &tauri::AppHandle,
    path: routes::Paths,
) -> Result<(), Error> {
    debug!("Attempting to redirect after login to: {:?}", path);
    let win = app_handle
        .get_webview_window("main")
        .ok_or(crate::error::TauriUiError::Window)?;
    let win_url = win.url()?;
    let redirect_url = routes::from_url(path, win_url);
    debug!("Redirecting to: {}", redirect_url);
    win.navigate(redirect_url)?;
    Ok(())
}

/// Code-based login for legacy stacks that don't support Connect/OAuth.
async fn login_command(
    m: &model::Model,
    tracing: &crate::telemetry::Telemetry,
    host: &str,
    code: String,
) -> Result<(), Error> {
    let host = quilt::uri::Host::from_str(host)?;
    model::login(m, &host, code).await?;

    tracing
        .track(MixpanelEvent::UserLoggedIn {
            host: host.to_string(),
            flow: LoginFlow::Legacy,
        })
        .await;

    Ok(())
}

#[tauri::command]
pub async fn login(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    host: String,
    code: String,
) -> Result<String, String> {
    let msg_init = format!("Login with code for host {host}");
    let msg_ok = format!("Successfully logged in to {host}");
    let msg_err = |err: &Error| format!("Failed to login: {err}");

    Notify::new(msg_init).map(
        login_command(&m, &tracing, &host, code).await,
        msg_ok,
        msg_err,
    )
}

/// Initiate OAuth 2.1 login: register client via DCR if needed,
/// generate PKCE, store verifier, open browser.
#[tauri::command]
pub async fn login_oauth(
    m: tauri::State<'_, model::Model>,
    oauth_state: tauri::State<'_, OAuthState>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    host: String,
    back: Option<String>,
) -> Result<String, String> {
    let host_parsed = quilt::uri::Host::from_str(&host).map_err(|e| e.to_string())?;

    let redirect_uri = crate::oauth::redirect_uri(&host_parsed);
    let client_id = model::get_or_register_client(&*m, &host_parsed, &redirect_uri)
        .await
        .map_err(|e| e.to_string())?;

    let request = oauth_state
        .start_login(&host_parsed, &client_id, back)
        .await;

    model::open_in_web_browser(&request.authorize_url).map_err(|e| e.to_string())?;

    tracing
        .track(MixpanelEvent::OAuthLoginInitiated { host: host.clone() })
        .await;

    Ok(format!("Opening browser for OAuth login to {host}"))
}

async fn setup_command(m: &model::Model, directory: &str) -> Result<quilt::lineage::Home, Error> {
    if let Err(err) = fs::create_dir_all(directory) {
        if err.kind() != std::io::ErrorKind::AlreadyExists {
            return Err(Error::from(err));
        }
    }

    m.set_home(&directory).await
}

#[tauri::command]
pub async fn setup(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    directory: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::SetupCompleted).await;
    let msg_init = format!("Setup with directory {directory}");
    let msg_ok = format!("Successfully set up directory: {directory}");
    let msg_err = |err: &Error| format!("Failed to create QuiltSync directory: {err}");
    Notify::new(msg_init).map(setup_command(&m, &directory).await, msg_ok, msg_err)
}

async fn package_install_paths_command(
    m: &model::Model,
    uri: &str,
    paths: &[String],
) -> Result<(), Error> {
    let uri = quilt::uri::S3PackageUri::try_from(uri)?;
    let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    model::install_paths_only(m, &uri.namespace, paths).await?;
    Ok(())
}

#[tauri::command]
pub async fn package_install_paths(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    uri: String,
    paths: Vec<String>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackageInstalled).await;

    let msg_init = format!("Installing paths from {uri}");
    let msg_ok = format!("Successfully installed {} paths", paths.len());
    let msg_err = |err: &Error| format!("Failed to install paths: {err}");

    Notify::new(msg_init).map(
        package_install_paths_command(&m, &uri, &paths).await,
        msg_ok,
        msg_err,
    )
}

async fn add_to_quiltignore_command(
    m: &model::Model,
    namespace: &str,
    pattern: &str,
) -> Result<(), Error> {
    let namespace = quilt::uri::Namespace::try_from(namespace)?;
    let package_home = m.package_home(&namespace).await?;
    let quiltignore_path = package_home.join(".quiltignore");

    // Take only the first line to prevent injecting multiple rules
    let pattern = pattern.lines().next().unwrap_or(pattern);

    // Read first to check trailing newline, before opening for append
    let needs_newline = std::fs::read_to_string(&quiltignore_path)
        .map(|s| !s.is_empty() && !s.ends_with('\n'))
        .unwrap_or(false);

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&quiltignore_path)
        .map_err(|e| format!("Failed to open .quiltignore: {e}"))?;

    if needs_newline {
        writeln!(file).map_err(|e| e.to_string())?;
    }
    writeln!(file, "{pattern}").map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn add_to_quiltignore(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
    pattern: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::QuiltignorePatternAdded).await;

    let msg_init = format!("Adding {pattern} to .quiltignore");
    let msg_ok = format!("Added {pattern} to .quiltignore");
    let msg_err = |err: &Error| format!("Failed to update .quiltignore: {err}");

    Notify::new(msg_init).map(
        add_to_quiltignore_command(&m, &namespace, &pattern).await,
        msg_ok,
        msg_err,
    )
}

#[tauri::command]
pub async fn test_quiltignore_pattern(pattern: String, path: String) -> Result<bool, String> {
    Ok(quilt::junk::pattern_matches(&pattern, &path))
}

// ── Remote package handling for Leptos UI ──

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePackageResult {
    pub namespace: String,
    pub notification: Option<String>,
}

#[tauri::command]
pub async fn handle_remote_package(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    uri: String,
) -> Result<RemotePackageResult, String> {
    let s3_uri: quilt::uri::S3PackageUri = uri.parse().map_err(|e: quilt::Error| e.to_string())?;
    let namespace = s3_uri.namespace.to_string();

    let _tracing: &crate::telemetry::Telemetry = &tracing;

    match model::install_package_only(&*m, &s3_uri)
        .await
        .map_err(|e| e.to_frontend_string())?
    {
        model::InstallOutcome::DifferentVersion {
            requested_hash,
            installed_hash,
        } => {
            let short_requested: String = requested_hash.chars().take(8).collect();
            let short_installed: String = installed_hash.chars().take(8).collect();
            let notification = rust_i18n::t!(
                "installed_package_notification.different_version",
                requested => short_requested,
                installed => short_installed,
            )
            .to_string();
            Ok(RemotePackageResult {
                namespace,
                notification: Some(notification),
            })
        }
        model::InstallOutcome::LocalOnly => {
            let notification =
                rust_i18n::t!("installed_package_notification.local_only").to_string();
            Ok(RemotePackageResult {
                namespace,
                notification: Some(notification),
            })
        }
        model::InstallOutcome::Installed => {
            // If URI has a path, install it and open in default application
            if let Some(ref path) = s3_uri.path {
                let installed_package = m
                    .get_installed_package(&s3_uri.namespace)
                    .await
                    .map_err(|e| e.to_frontend_string())?
                    .ok_or_else(|| format!("Package {namespace} is not installed"))?;
                if !m
                    .is_path_installed(&installed_package, path)
                    .await
                    .map_err(|e| e.to_frontend_string())?
                {
                    m.package_install_paths(&installed_package, std::slice::from_ref(path))
                        .await
                        .map_err(|e| e.to_frontend_string())?;
                }
                m.open_in_default_application(&s3_uri.namespace, path)
                    .await
                    .map_err(|e| e.to_frontend_string())?;
            }
            Ok(RemotePackageResult {
                namespace,
                notification: None,
            })
        }
    }
}

// ── Auto-update ────────────────────────────────────────────

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
}

#[tauri::command]
pub async fn check_for_update(app: tauri::AppHandle) -> Result<Option<UpdateInfo>, String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    match updater.check().await {
        Ok(Some(update)) => Ok(Some(UpdateInfo {
            version: update.version.clone(),
        })),
        Ok(None) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub async fn download_and_install_update(app: tauri::AppHandle) -> Result<(), String> {
    let updater = app.updater().map_err(|e| e.to_string())?;
    let update = updater
        .check()
        .await
        .map_err(|e| e.to_string())?
        .ok_or_else(|| "No update available".to_string())?;
    update
        .download_and_install(|_, _| {}, || {})
        .await
        .map_err(|e| e.to_string())?;
    app.restart();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::mocks;

    #[tokio::test]
    async fn test_get_login_error_data() -> Result<(), String> {
        let data = get_login_error_data(
            "test.quilt.dev".to_string(),
            Some("Login failed".to_string()),
            "Auth failed".to_string(),
        )
        .await?;
        assert_eq!(data.title, "Login failed");
        assert_eq!(data.message, "Auth failed");
        assert_eq!(data.login_host, "test.quilt.dev");
        Ok(())
    }

    #[tokio::test]
    async fn test_get_login_error_data_default_title() -> Result<(), String> {
        let data = get_login_error_data(
            "test.quilt.dev".to_string(),
            None,
            "Auth failed".to_string(),
        )
        .await?;
        assert_eq!(data.title, "Login failed");
        assert_eq!(data.message, "Auth failed");
        Ok(())
    }

    #[tokio::test]
    async fn test_get_merge_data() -> Result<(), String> {
        let mut model = mocks::create();
        mocks::mock_installed_package(&mut model);
        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_merge_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(data.namespace, "foo/bar");
        assert!(data.origin_url.is_some());
        assert!(data.origin_url.unwrap().contains("test.quilt.dev"));
        assert_eq!(data.origin_host, Some("test.quilt.dev".to_string()));
        Ok(())
    }

    #[tokio::test]
    async fn test_get_merge_data_not_installed() {
        let mut model = mocks::create();
        model.expect_get_installed_package().returning(|_| Ok(None));
        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("missing", "package").into();

        let result = get_merge_data_from_model(&model, &tracing, &namespace).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_installed_packages_list_data_empty() -> Result<(), String> {
        let mut model = mocks::create();
        mocks::mock_installed_packages_list(&mut model);
        let tracing = crate::telemetry::Telemetry::default();

        let data = get_installed_packages_list_data_from_model(&model, &tracing)
            .await
            .map_err(|e| e.to_string())?;

        assert!(data.packages.is_empty());
        Ok(())
    }

    /// Helper: create a quilt::InstalledPackage with a given namespace.
    fn make_installed_package(
        namespace: impl Into<quilt::uri::Namespace>,
    ) -> quilt::InstalledPackage {
        quilt::LocalDomain::new(std::path::PathBuf::new())
            .create_installed_package(namespace.into())
            .expect("Failed to create installed package")
    }

    /// Helper: create a ManifestUri with origin for a given namespace.
    fn make_manifest_uri(namespace: &str) -> quilt::uri::ManifestUri {
        quilt::uri::ManifestUri {
            origin: Some("test.quilt.dev".parse().unwrap()),
            bucket: "test".to_string(),
            namespace: namespace.try_into().unwrap(),
            hash: "abcdef".to_string(),
        }
    }

    /// Helper: create a ManifestUri **without** origin (triggers error state).
    fn make_manifest_uri_no_origin(namespace: &str) -> quilt::uri::ManifestUri {
        quilt::uri::ManifestUri {
            origin: None,
            bucket: "test".to_string(),
            namespace: namespace.try_into().unwrap(),
            hash: "abcdef".to_string(),
        }
    }

    #[tokio::test]
    async fn test_installed_packages_list_data_statuses() -> Result<(), String> {
        let mut model = mocks::create();

        let pkgs = vec![
            make_installed_package(("test", "ahead")),
            make_installed_package(("test", "behind")),
            make_installed_package(("test", "diverged")),
            make_installed_package(("test", "uptodate")),
        ];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));

        // Set up lineage so From<PackageLineage> produces the expected status.
        // Status is derived from base_hash vs current_hash (ahead) and
        // base_hash vs latest_hash (behind).
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let ns = pkg.namespace.to_string();
                let uri = make_manifest_uri(&ns);
                // base_hash comes from uri.hash ("abcdef")
                let lineage = match ns.as_str() {
                    // Ahead: current_hash != base_hash, base_hash == latest_hash
                    "test/ahead" => {
                        let mut l =
                            quilt::lineage::PackageLineage::from_remote(uri, "abcdef".into());
                        l.commit = Some(quilt::lineage::CommitState {
                            hash: "local1".into(),
                            ..Default::default()
                        });
                        l
                    }
                    // Behind: base_hash != latest_hash, current_hash == base_hash
                    "test/behind" => {
                        quilt::lineage::PackageLineage::from_remote(uri, "remote1".into())
                    }
                    // Diverged: both ahead and behind
                    "test/diverged" => {
                        let mut l =
                            quilt::lineage::PackageLineage::from_remote(uri, "remote2".into());
                        l.commit = Some(quilt::lineage::CommitState {
                            hash: "local2".into(),
                            ..Default::default()
                        });
                        l
                    }
                    // UpToDate: all hashes match
                    _ => quilt::lineage::PackageLineage::from_remote(uri, "abcdef".into()),
                };
                Ok(lineage)
            });

        let tracing = crate::telemetry::Telemetry::default();
        let data = get_installed_packages_list_data_from_model(&model, &tracing)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(data.packages.len(), 4);

        let find = |ns: &str| data.packages.iter().find(|p| p.namespace == ns).unwrap();

        let ahead = find("test/ahead");
        assert_eq!(ahead.status, "ahead");
        assert!(!ahead.has_changes); // Light phase always returns false
        assert!(ahead.origin_url.is_some());
        assert_eq!(ahead.origin_host.as_deref(), Some("test.quilt.dev"));
        assert!(ahead.remote_display.is_some());

        let behind = find("test/behind");
        assert_eq!(behind.status, "behind");
        assert!(behind.origin_url.is_some());

        let diverged = find("test/diverged");
        assert_eq!(diverged.status, "diverged");
        assert!(diverged.origin_url.is_some());

        let uptodate = find("test/uptodate");
        assert_eq!(uptodate.status, "up_to_date");
        assert!(uptodate.origin_url.is_some());

        Ok(())
    }

    #[tokio::test]
    async fn test_installed_packages_list_data_with_origin_shows_cached_status(
    ) -> Result<(), String> {
        let mut model = mocks::create();

        let pkgs = vec![make_installed_package(("test", "pkg"))];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));

        // Lineage indicates up_to_date (base == latest == remote hash)
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let data = get_installed_packages_list_data_from_model(&model, &tracing)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(data.packages.len(), 1);
        let pkg = &data.packages[0];
        assert_eq!(pkg.namespace, "test/pkg");
        // Light phase derives status from lineage (up_to_date, not error)
        assert_eq!(pkg.status, "up_to_date");
        assert!(!pkg.has_changes); // Always false in light phase
                                   // Should still have origin
        assert!(pkg.origin_url.is_some());
        assert_eq!(pkg.origin_host.as_deref(), Some("test.quilt.dev"));

        Ok(())
    }

    #[tokio::test]
    async fn test_installed_packages_list_data_no_origin() -> Result<(), String> {
        let mut model = mocks::create();

        let pkgs = vec![make_installed_package(("test", "noorigin"))];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));

        // Remote URI exists but has no origin → triggers early return with error status
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri_no_origin(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let data = get_installed_packages_list_data_from_model(&model, &tracing)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(data.packages.len(), 1);
        let pkg = &data.packages[0];
        assert_eq!(pkg.namespace, "test/noorigin");
        assert_eq!(pkg.status, "error");
        // No origin_url or origin_host (for Set Origin button in UI)
        assert!(pkg.origin_url.is_none());
        assert!(pkg.origin_host.is_none());
        // remote_display should still be present
        assert!(pkg.remote_display.is_some());

        Ok(())
    }

    #[tokio::test]
    async fn test_installed_packages_list_data_local_without_remote() -> Result<(), String> {
        let mut model = mocks::create();

        let pkgs = vec![make_installed_package(("test", "local"))];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));

        // No remote_uri at all → local-only package
        model
            .expect_get_installed_package_lineage()
            .returning(|_| Ok(quilt::lineage::PackageLineage::default()));

        let tracing = crate::telemetry::Telemetry::default();
        let data = get_installed_packages_list_data_from_model(&model, &tracing)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(data.packages.len(), 1);
        let pkg = &data.packages[0];
        assert_eq!(pkg.namespace, "test/local");
        assert_eq!(pkg.status, "local");
        assert!(pkg.origin_url.is_none());
        assert!(pkg.origin_host.is_none());
        assert!(pkg.remote_display.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn test_installed_packages_list_data_local_with_origin() -> Result<(), String> {
        let mut model = mocks::create();

        let pkgs = vec![make_installed_package(("test", "localpush"))];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));

        // Has remote URI with origin but never pushed (empty hash → Local)
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = quilt::uri::ManifestUri {
                    origin: Some("test.quilt.dev".parse().unwrap()),
                    bucket: "test".to_string(),
                    namespace: pkg.namespace.clone(),
                    hash: String::new(),
                };
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    String::new(),
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let data = get_installed_packages_list_data_from_model(&model, &tracing)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(data.packages.len(), 1);
        let pkg = &data.packages[0];
        assert_eq!(pkg.namespace, "test/localpush");
        assert_eq!(pkg.status, "local");
        assert!(!pkg.has_changes);
        // Has origin (for Push button and disabled Catalog button in UI)
        assert!(pkg.origin_url.is_some());
        assert_eq!(pkg.origin_host.as_deref(), Some("test.quilt.dev"));

        Ok(())
    }

    // ── refresh_package_status tests (heavy phase) ──

    #[tokio::test]
    async fn test_refresh_package_status_local_only_no_changes() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "local"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
        model
            .expect_get_installed_package_lineage()
            .returning(|_| Ok(quilt::lineage::PackageLineage::default()));
        model
            .expect_get_installed_package_status()
            .returning(|_, _| {
                Ok(quilt::lineage::InstalledPackageStatus::new(
                    quilt::lineage::UpstreamState::Local,
                    quilt::lineage::ChangeSet::new(),
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "local").into();
        let result = refresh_package_status_from_model(&model, &tracing, &ns)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "local");
        assert!(!result.has_changes);
        Ok(())
    }

    #[tokio::test]
    async fn test_refresh_package_status_local_only_with_changes() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "local"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
        model
            .expect_get_installed_package_lineage()
            .returning(|_| Ok(quilt::lineage::PackageLineage::default()));
        model
            .expect_get_installed_package_status()
            .returning(|_, _| {
                let mut changes = quilt::lineage::ChangeSet::new();
                changes.insert(
                    std::path::PathBuf::from("file.txt"),
                    quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
                );
                Ok(quilt::lineage::InstalledPackageStatus::new(
                    quilt::lineage::UpstreamState::Local,
                    changes,
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "local").into();
        let result = refresh_package_status_from_model(&model, &tracing, &ns)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "local");
        assert!(result.has_changes);
        Ok(())
    }

    #[tokio::test]
    async fn test_refresh_package_status_no_origin() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "noorigin"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri_no_origin(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "noorigin").into();
        let result = refresh_package_status_from_model(&model, &tracing, &ns)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "error");
        assert!(!result.has_changes);
        Ok(())
    }

    #[tokio::test]
    async fn test_refresh_package_status_with_changes() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "changed"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });
        model
            .expect_get_installed_package_status()
            .returning(|_, _| {
                let mut changes = quilt::lineage::ChangeSet::new();
                changes.insert(
                    std::path::PathBuf::from("file.txt"),
                    quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
                );
                Ok(quilt::lineage::InstalledPackageStatus::new(
                    quilt::lineage::UpstreamState::UpToDate,
                    changes,
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "changed").into();
        let result = refresh_package_status_from_model(&model, &tracing, &ns)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "up_to_date");
        assert!(result.has_changes);
        Ok(())
    }

    #[tokio::test]
    async fn test_refresh_package_status_without_changes() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "clean"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "remote1".to_string(),
                ))
            });
        model
            .expect_get_installed_package_status()
            .returning(|_, _| {
                Ok(quilt::lineage::InstalledPackageStatus::new(
                    quilt::lineage::UpstreamState::Behind,
                    Default::default(),
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "clean").into();
        let result = refresh_package_status_from_model(&model, &tracing, &ns)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "behind");
        assert!(!result.has_changes);
        Ok(())
    }

    #[tokio::test]
    async fn test_refresh_package_status_error_on_status_fetch() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "broken"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });
        model
            .expect_get_installed_package_status()
            .returning(|_, _| Err(crate::error::Error::General("network error".to_string())));

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "broken").into();
        let result = refresh_package_status_from_model(&model, &tracing, &ns)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "error");
        assert!(!result.has_changes);
        Ok(())
    }

    // ── Installed package data tests ──
    // (Adapted from pages/installed_package.rs: test_view, test_view_entries,
    //  test_view_no_origin, test_view_status_failed, test_view_local_only,
    //  test_view_local_with_origin_disables_catalog_button)

    #[tokio::test]
    async fn test_get_installed_package_data() -> Result<(), String> {
        let mut model = mocks::create();
        mocks::mock_installed_package(&mut model);
        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data =
            get_installed_package_data_from_model(&model, &tracing, &namespace, Default::default())
                .await
                .map_err(|e| e.to_string())?;

        assert_eq!(data.namespace, "foo/bar");
        assert!(data.origin_url.is_some());
        assert!(data.origin_url.unwrap().contains("test.quilt.dev"));
        assert_eq!(data.origin_host, Some("test.quilt.dev".to_string()));
        // Mock has one record "NAME" — should appear as an entry
        assert!(!data.entries.is_empty());
        let entry = data.entries.iter().find(|e| e.filename == "NAME");
        assert!(entry.is_some(), "Entry 'NAME' should be present");
        Ok(())
    }

    #[tokio::test]
    async fn test_get_installed_package_data_not_installed() {
        let mut model = mocks::create();
        model.expect_get_installed_package().returning(|_| Ok(None));
        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("missing", "package").into();

        let result =
            get_installed_package_data_from_model(&model, &tracing, &namespace, Default::default())
                .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_installed_package_data_no_origin() -> Result<(), String> {
        let mut model = mocks::create();

        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri_no_origin(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data =
            get_installed_package_data_from_model(&model, &tracing, &namespace, Default::default())
                .await
                .map_err(|e| e.to_string())?;

        assert_eq!(data.status, "error");
        assert!(data.origin_url.is_none());
        assert!(data.origin_host.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_get_installed_package_data_error_with_origin() -> Result<(), String> {
        let mut model = mocks::create();

        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });
        model
            .expect_get_installed_package_status()
            .returning(|_, _| Ok(quilt::lineage::InstalledPackageStatus::error()));
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data =
            get_installed_package_data_from_model(&model, &tracing, &namespace, Default::default())
                .await
                .map_err(|e| e.to_string())?;

        assert_eq!(data.status, "error");
        assert!(data.origin_url.is_some());
        assert_eq!(data.origin_host.as_deref(), Some("test.quilt.dev"));
        Ok(())
    }

    #[tokio::test]
    async fn test_get_installed_package_data_local_only() -> Result<(), String> {
        let mut model = mocks::create();

        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        // No remote URI → local-only package
        model
            .expect_get_installed_package_lineage()
            .returning(|_| Ok(quilt::lineage::PackageLineage::default()));
        model
            .expect_get_installed_package_status()
            .returning(|_, _| Ok(quilt::lineage::InstalledPackageStatus::local()));
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data =
            get_installed_package_data_from_model(&model, &tracing, &namespace, Default::default())
                .await
                .map_err(|e| e.to_string())?;

        assert_eq!(data.status, "local");
        assert!(data.origin_url.is_none());
        assert!(data.origin_host.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_get_installed_package_data_local_with_origin() -> Result<(), String> {
        let mut model = mocks::create();

        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = quilt::uri::ManifestUri {
                    origin: Some("test.quilt.dev".parse().unwrap()),
                    bucket: "test".to_string(),
                    namespace: pkg.namespace.clone(),
                    hash: String::new(),
                };
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    String::new(),
                ))
            });
        model
            .expect_get_installed_package_status()
            .returning(|_, _| Ok(quilt::lineage::InstalledPackageStatus::local()));
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data =
            get_installed_package_data_from_model(&model, &tracing, &namespace, Default::default())
                .await
                .map_err(|e| e.to_string())?;

        assert_eq!(data.status, "local");
        // Has origin for Push button and disabled Catalog button
        assert!(data.origin_url.is_some());
        assert_eq!(data.origin_host.as_deref(), Some("test.quilt.dev"));
        Ok(())
    }

    // (Adapted from pages/installed_package.rs: test_sizes)

    #[tokio::test]
    async fn test_get_installed_package_data_entry_sizes() -> Result<(), String> {
        let mut model = mocks::create();

        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });
        model
            .expect_get_installed_package_status()
            .returning(|_, _| Ok(quilt::lineage::InstalledPackageStatus::default()));

        let expected_sizes: Vec<(&str, u64)> = vec![
            ("empty.csv", 0),
            ("small.csv", 12),
            ("kilobytes.csv", 1_234),
            ("megabytes.csv", 12_345_678),
            ("petabytes.csv", 1_234_567_890_123_456),
        ];
        let records: std::collections::BTreeMap<std::path::PathBuf, quilt::manifest::ManifestRow> =
            expected_sizes
                .iter()
                .map(|(name, size)| {
                    let row = quilt::manifest::ManifestRow {
                        logical_key: std::path::PathBuf::from(name),
                        size: *size,
                        ..Default::default()
                    };
                    (std::path::PathBuf::from(name), row)
                })
                .collect();
        model
            .expect_get_installed_package_records()
            .return_once(move |_| Ok(records));

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data =
            get_installed_package_data_from_model(&model, &tracing, &namespace, Default::default())
                .await
                .map_err(|e| e.to_string())?;

        assert_eq!(data.entries.len(), expected_sizes.len());
        for (name, expected_size) in &expected_sizes {
            let entry = data
                .entries
                .iter()
                .find(|e| e.filename == *name)
                .unwrap_or_else(|| panic!("Entry '{name}' should be present"));
            assert_eq!(entry.size, *expected_size, "Size mismatch for '{name}'");
        }
        Ok(())
    }

    // ── Login data tests ──
    // (Adapted from pages/login.rs: test_login_page_rendering)

    #[tokio::test]
    async fn test_get_login_data() -> Result<(), String> {
        let data = get_login_data(
            "test.quilt.dev".to_string(),
            "/installed-packages-list".to_string(),
        )
        .await?;

        assert_eq!(data.host, "test.quilt.dev");
        assert_eq!(data.back, "/installed-packages-list");
        assert_eq!(data.catalog_url, "https://test.quilt.dev/code");
        Ok(())
    }

    // (Adapted from pages/login.rs: test_login_oauth_button_without_back)

    #[tokio::test]
    async fn test_get_login_data_empty_back() -> Result<(), String> {
        let data = get_login_data("test.quilt.dev".to_string(), String::new()).await?;

        assert_eq!(data.host, "test.quilt.dev");
        assert_eq!(data.back, "");
        assert_eq!(data.catalog_url, "https://test.quilt.dev/code");
        Ok(())
    }

    // ── Commit data tests ──

    #[tokio::test]
    async fn test_get_commit_data() -> Result<(), String> {
        let mut model = mocks::create();
        mocks::mock_installed_package(&mut model);
        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(data.namespace, "foo/bar");
        assert!(data.origin_url.is_some());
        assert!(data.origin_url.unwrap().contains("test.quilt.dev"));
        assert_eq!(data.origin_host, Some("test.quilt.dev".to_string()));
        Ok(())
    }

    #[tokio::test]
    async fn test_get_commit_data_not_installed() {
        let mut model = mocks::create();
        model.expect_get_installed_package().returning(|_| Ok(None));
        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("missing", "package").into();

        let result = get_commit_data_from_model(&model, &tracing, &namespace).await;
        assert!(result.is_err());
    }

    // (Adapted from pages/commit.rs: test_workflow_with_value)

    #[tokio::test]
    async fn test_get_commit_data_with_workflow() -> Result<(), String> {
        let mut model = mocks::create();

        let remote_manifest = quilt::uri::ManifestUri {
            bucket: "quilt-example".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "abcdef".to_string(),
            origin: Some("test.quilt.dev".parse().unwrap()),
        };
        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(move |_| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    remote_manifest.clone(),
                    remote_manifest.hash.clone(),
                ))
            });
        let status = Ok(quilt::lineage::InstalledPackageStatus::default());
        model
            .expect_get_installed_package_status()
            .return_once(move |_, _| status);
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));
        // Return a manifest with workflow data
        model.expect_browse_remote_manifest().returning(|_| {
            let config_uri = quilt::uri::S3Uri {
                bucket: "quilt-example".to_string(),
                key: ".quilt/workflows/config.yaml".to_string(),
                version: None,
            };
            Ok(quilt::manifest::Manifest {
                header: quilt::manifest::ManifestHeader {
                    version: "v0".to_string(),
                    message: None,
                    user_meta: None,
                    workflow: Some(quilt::manifest::Workflow {
                        config: config_uri,
                        id: Some(quilt::manifest::WorkflowId {
                            id: "gamma".to_string(),
                            metadata: None,
                        }),
                    }),
                },
                rows: Vec::new(),
            })
        });

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert!(data.workflow.is_some());
        let workflow = data.workflow.unwrap();
        assert_eq!(workflow.id, Some("gamma".to_string()));
        assert!(workflow.url.is_some());
        Ok(())
    }

    // (Adapted from pages/commit.rs: test_workflow_null_checked)

    #[tokio::test]
    async fn test_get_commit_data_workflow_null_id() -> Result<(), String> {
        let mut model = mocks::create();

        let remote_manifest = quilt::uri::ManifestUri {
            bucket: "quilt-example".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "abcdef".to_string(),
            origin: Some("test.quilt.dev".parse().unwrap()),
        };
        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(move |_| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    remote_manifest.clone(),
                    remote_manifest.hash.clone(),
                ))
            });
        let status = Ok(quilt::lineage::InstalledPackageStatus::default());
        model
            .expect_get_installed_package_status()
            .return_once(move |_, _| status);
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));
        // Workflow exists but has no ID (null/checked state)
        model.expect_browse_remote_manifest().returning(|_| {
            let config_uri = quilt::uri::S3Uri {
                bucket: "quilt-example".to_string(),
                key: ".quilt/workflows/config.yaml".to_string(),
                version: None,
            };
            Ok(quilt::manifest::Manifest {
                header: quilt::manifest::ManifestHeader {
                    version: "v0".to_string(),
                    message: None,
                    user_meta: None,
                    workflow: Some(quilt::manifest::Workflow {
                        config: config_uri,
                        id: None,
                    }),
                },
                rows: Vec::new(),
            })
        });

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert!(data.workflow.is_some());
        let workflow = data.workflow.unwrap();
        assert!(workflow.id.is_none());
        assert!(workflow.url.is_some());
        Ok(())
    }

    // (Adapted from pages/commit.rs: test_workflow_not_available)

    #[tokio::test]
    async fn test_get_commit_data_no_workflow() -> Result<(), String> {
        let mut model = mocks::create();

        let remote_manifest = quilt::uri::ManifestUri {
            bucket: "quilt-example".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "abcdef".to_string(),
            origin: Some("test.quilt.dev".parse().unwrap()),
        };
        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(move |_| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    remote_manifest.clone(),
                    remote_manifest.hash.clone(),
                ))
            });
        let status = Ok(quilt::lineage::InstalledPackageStatus::default());
        model
            .expect_get_installed_package_status()
            .return_once(move |_, _| status);
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));
        // No workflow in manifest
        model.expect_browse_remote_manifest().returning(|_| {
            Ok(quilt::manifest::Manifest {
                header: quilt::manifest::ManifestHeader {
                    version: "v0".to_string(),
                    message: None,
                    user_meta: None,
                    workflow: None,
                },
                rows: Vec::new(),
            })
        });

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert!(data.workflow.is_none());
        Ok(())
    }

    // ── has_changes tests ──
}
