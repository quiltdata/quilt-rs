//! Settings data and update commands for the Leptos UI.

use serde::Deserialize;
use serde::Serialize;
use tauri::Manager;
use tokio::sync;

use crate::app;
use crate::autopull::AutosyncSettings;
use crate::autopull::PullSettings;
use crate::autopull::PushSettings;
use crate::autopull::SharedAutosyncSettings;
use crate::autopull::Watcher;
use crate::changelog;
use crate::fswatcher::FsWatcherSettings;
use crate::fswatcher::SharedFsWatcherSettings;
use crate::model;
use crate::model::QuiltModel;
use crate::publish_settings::PublishSettings;
use crate::publish_settings::SharedPublishSettings;
use crate::quilt;
use crate::telemetry::Telemetry;

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

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutosyncSettingsData {
    pub pull_enabled: bool,
    pub push_enabled: bool,
    pub pull_interval_secs: u64,
    pub idle_timeout_secs: u64,
    pub close_to_tray: bool,
}

impl From<AutosyncSettings> for AutosyncSettingsData {
    fn from(s: AutosyncSettings) -> Self {
        // `pull_interval_secs` projects `focused_secs`. On a hand-edited
        // JSON where focused != unfocused, the UI shows the focused value
        // — what an active user experiences — and a Save will write both
        // fields to the same value via `merge_autosync_settings_data`.
        Self {
            pull_enabled: s.pull.enabled,
            push_enabled: s.push.enabled,
            pull_interval_secs: s.pull.focused_secs,
            idle_timeout_secs: s.push.idle_timeout_secs,
            close_to_tray: s.close_to_tray,
        }
    }
}

fn validate_autosync_settings_data(data: &AutosyncSettingsData) -> Result<(), String> {
    if data.pull_interval_secs == 0 {
        return Err("pull_interval_secs must be > 0".to_string());
    }
    if data.idle_timeout_secs == 0 {
        return Err("idle_timeout_secs must be > 0".to_string());
    }
    Ok(())
}

/// Project `AutosyncSettingsData` onto a full `AutosyncSettings` by
/// overwriting only the UI-owned fields on top of the current disk
/// state. `closed_secs` — and any future disk-only field — flows
/// through untouched.
fn merge_autosync_settings_data(
    current: &AutosyncSettings,
    incoming: &AutosyncSettingsData,
) -> AutosyncSettings {
    AutosyncSettings {
        pull: PullSettings {
            enabled: incoming.pull_enabled,
            focused_secs: incoming.pull_interval_secs,
            unfocused_secs: incoming.pull_interval_secs,
            closed_secs: current.pull.closed_secs,
        },
        push: PushSettings {
            enabled: incoming.push_enabled,
            idle_timeout_secs: incoming.idle_timeout_secs,
        },
        close_to_tray: incoming.close_to_tray,
    }
}

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct FsWatcherSettingsData {
    pub enabled: bool,
}

impl From<FsWatcherSettings> for FsWatcherSettingsData {
    fn from(s: FsWatcherSettings) -> Self {
        Self { enabled: s.enabled }
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
    pub autosync: AutosyncSettingsData,
    pub fswatcher: FsWatcherSettingsData,
}

#[tauri::command]
pub async fn get_settings_data(
    m: tauri::State<'_, model::Model>,
    app: tauri::State<'_, app::App>,
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    publish: tauri::State<'_, SharedPublishSettings>,
    autosync_settings: tauri::State<'_, SharedAutosyncSettings>,
    fswatcher_settings: tauri::State<'_, SharedFsWatcherSettings>,
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
    let log_level = Telemetry::log_level();
    let publish_data = PublishSettingsData::from(publish.read().await.clone());
    let autosync_data = AutosyncSettingsData::from(autosync_settings.read().await.clone());
    let fswatcher_data = FsWatcherSettingsData::from(fswatcher_settings.read().await.clone());

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
        autosync: autosync_data,
        fswatcher: fswatcher_data,
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
    // Validate metadata is parseable JSON (or empty/whitespace = no metadata).
    // `opt_from_string` below trims whitespace-only input down to `None`, so
    // we mirror that here: a whitespace-only blob is treated as "no metadata"
    // rather than being handed to `serde_json::from_str` (which rejects it).
    if !default_metadata.trim().is_empty() {
        serde_json::from_str::<serde_json::Value>(&default_metadata)
            .map_err(|e| format!("Invalid metadata JSON: {e}"))?;
    }

    let new = PublishSettings {
        message_template: opt_from_string(&message_template),
        default_workflow: opt_from_string(&default_workflow),
        default_metadata: opt_from_string(&default_metadata),
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

fn opt_from_string(s: &str) -> Option<String> {
    let trimmed = s.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_string())
}

#[tauri::command]
pub async fn update_autosync_settings(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    autosync_settings: tauri::State<'_, SharedAutosyncSettings>,
    watcher: tauri::State<'_, Watcher>,
    settings: AutosyncSettingsData,
) -> Result<(), String> {
    validate_autosync_settings_data(&settings)?;

    let app_handle = app_handle.lock().await;
    let data_dir = app_handle
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?;

    let prev_any_enabled = {
        let mut current = autosync_settings.write().await;
        let merged = merge_autosync_settings_data(&current, &settings);
        merged.save(&data_dir).await.map_err(|e| e.to_string())?;
        let prev = current.pull.enabled || current.push.enabled;
        *current = merged;
        prev
    };
    // Flipping the overall "off → on" edge clears the paused set: a
    // re-enable in either direction is a signal that the user wants
    // the watcher to retry every namespace, not just the ones they
    // touched manually.
    if !prev_any_enabled && (settings.pull_enabled || settings.push_enabled) {
        watcher.clear_all_paused().await;
    }
    Ok(())
}

/// Point-in-time view of the autosync watcher's per-namespace state.
///
/// Used by the UI to re-hydrate paused-state banners after navigation:
/// listening for the `autosync-paused` event only catches pauses that
/// fire while a page is mounted, while the watcher's state persists
/// across page loads.
#[tauri::command]
pub async fn get_autosync_snapshot(
    watcher: tauri::State<'_, Watcher>,
) -> Result<crate::autopull::reporter::WatcherSnapshot, String> {
    Ok(watcher.snapshot().await)
}

#[tauri::command]
pub async fn update_fswatcher_settings(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    fswatcher_settings: tauri::State<'_, SharedFsWatcherSettings>,
    enabled: bool,
) -> Result<(), String> {
    let app_handle = app_handle.lock().await;
    let data_dir = app_handle
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?;

    // Preserve the on-disk `debounce_ms` (not surfaced in the UI yet).
    let new = {
        let current = fswatcher_settings.read().await.clone();
        FsWatcherSettings {
            enabled,
            debounce_ms: current.debounce_ms,
        }
    };

    new.save(&data_dir).await.map_err(|e| e.to_string())?;
    *fswatcher_settings.write().await = new;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    use crate::Error;

    #[test]
    fn data_from_settings_projects_focused_onto_pull_interval() {
        let s = AutosyncSettings {
            pull: PullSettings {
                enabled: true,
                focused_secs: 45,
                unfocused_secs: 999, // hand-edited divergence
                closed_secs: 600,
            },
            push: PushSettings {
                enabled: true,
                idle_timeout_secs: 22,
            },
            close_to_tray: false,
        };
        let data = AutosyncSettingsData::from(s);
        assert!(data.pull_enabled);
        assert!(data.push_enabled);
        assert_eq!(
            data.pull_interval_secs, 45,
            "focused wins on divergent files — that is what an active user feels",
        );
        assert_eq!(data.idle_timeout_secs, 22);
    }

    #[tokio::test]
    async fn merge_preserves_closed_secs() -> Result<(), Error> {
        // Save a settings file with a non-default `closed_secs`, then run
        // the merge logic with arbitrary UI-owned values, and verify
        // `closed_secs` survives on disk.
        let dir = TempDir::new().unwrap();
        let initial = AutosyncSettings {
            pull: PullSettings {
                enabled: true,
                focused_secs: 30,
                unfocused_secs: 120,
                closed_secs: 999,
            },
            push: PushSettings {
                enabled: false,
                idle_timeout_secs: 30,
            },
            close_to_tray: false,
        };
        initial.save(dir.path()).await?;

        let incoming = AutosyncSettingsData {
            pull_enabled: false,
            push_enabled: true,
            pull_interval_secs: 7,
            idle_timeout_secs: 9,
            close_to_tray: false,
        };
        let merged = merge_autosync_settings_data(&initial, &incoming);

        assert!(!merged.pull.enabled);
        assert!(merged.push.enabled);
        assert_eq!(merged.pull.focused_secs, 7);
        assert_eq!(
            merged.pull.unfocused_secs, 7,
            "UI ties focused == unfocused"
        );
        assert_eq!(
            merged.pull.closed_secs, 999,
            "closed_secs must flow through untouched"
        );
        assert_eq!(merged.push.idle_timeout_secs, 9);
        Ok(())
    }

    #[test]
    fn validate_rejects_zero_pull_interval() {
        let bad = AutosyncSettingsData {
            pull_enabled: true,
            push_enabled: true,
            pull_interval_secs: 0,
            idle_timeout_secs: 30,
            close_to_tray: false,
        };
        assert!(validate_autosync_settings_data(&bad).is_err());
    }

    #[test]
    fn validate_rejects_zero_idle_timeout() {
        let bad = AutosyncSettingsData {
            pull_enabled: true,
            push_enabled: true,
            pull_interval_secs: 30,
            idle_timeout_secs: 0,
            close_to_tray: false,
        };
        assert!(validate_autosync_settings_data(&bad).is_err());
    }

    #[test]
    fn validate_accepts_positive_values() {
        let ok = AutosyncSettingsData {
            pull_enabled: true,
            push_enabled: true,
            pull_interval_secs: 1,
            idle_timeout_secs: 1,
            close_to_tray: false,
        };
        assert!(validate_autosync_settings_data(&ok).is_ok());
    }

    #[test]
    fn autosync_settings_data_preserves_close_to_tray() {
        let s = AutosyncSettings {
            close_to_tray: true,
            ..Default::default()
        };
        let data = AutosyncSettingsData::from(s);
        assert!(data.close_to_tray);
    }

    #[test]
    fn merge_preserves_close_to_tray_from_incoming() {
        let initial = AutosyncSettings::default();
        let incoming = AutosyncSettingsData {
            pull_enabled: true,
            push_enabled: false,
            pull_interval_secs: 30,
            idle_timeout_secs: 60,
            close_to_tray: true,
        };
        let merged = merge_autosync_settings_data(&initial, &incoming);
        assert!(merged.close_to_tray);
    }
}
