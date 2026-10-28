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
use crate::design_settings::Design;
use crate::design_settings::DesignSettings;
use crate::experimental_settings::ExperimentalSettings;
use crate::experimental_settings::SharedExperimentalSettings;
use crate::fswatcher::FsWatcherSettings;
use crate::fswatcher::SharedFsWatcherSettings;
use crate::log_settings::LogEnv;
use crate::log_settings::LogLevel;
use crate::log_settings::LogSettings;
use crate::model;
use crate::model::QuiltModel;
use crate::publish_settings::PublishSettings;
use crate::publish_settings::SharedPublishSettings;
use crate::quilt;
use crate::tasks::settings::TaskSettings;

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
        // `pull_interval_secs` is `focused_secs`, the one interval.
        Self {
            pull_enabled: s.pull.enabled,
            push_enabled: s.push.enabled,
            pull_interval_secs: s.pull.focused_secs,
            idle_timeout_secs: s.push.idle_timeout_secs,
            close_to_tray: s.close_to_tray,
        }
    }
}

/// Only the quiet window is refused at 0: a check interval below a minute is
/// raised to one ([`AutosyncSettings::normalised`]).
fn validate_autosync_settings_data(data: &AutosyncSettingsData) -> Result<(), String> {
    if data.idle_timeout_secs == 0 {
        return Err("idle_timeout_secs must be > 0".to_string());
    }
    Ok(())
}

/// Project `AutosyncSettingsData` onto a full `AutosyncSettings` by
/// overwriting only the UI-owned fields on top of the current disk
/// state. `unfocused_secs` and `closed_secs` — and any future disk-only
/// field — flow through untouched. The interval comes out in whole minutes.
fn merge_autosync_settings_data(
    current: &AutosyncSettings,
    incoming: &AutosyncSettingsData,
) -> AutosyncSettings {
    AutosyncSettings {
        pull: PullSettings {
            enabled: incoming.pull_enabled,
            focused_secs: incoming.pull_interval_secs,
            unfocused_secs: current.pull.unfocused_secs,
            closed_secs: current.pull.closed_secs,
        },
        push: PushSettings {
            enabled: incoming.push_enabled,
            idle_timeout_secs: incoming.idle_timeout_secs,
        },
        close_to_tray: incoming.close_to_tray,
    }
    .normalised()
}

/// Some of autosync's settings, changed; every `None` stays as stored.
///
/// A page that saves one control at a time writes through this, so a write
/// never resets a field it does not own: the main page knows two booleans, and
/// a Settings row knows one value. `pull_interval_secs` is the one check
/// interval, in whole minutes once applied.
#[derive(Deserialize, Clone, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AutosyncPatch {
    pub pull_enabled: Option<bool>,
    pub push_enabled: Option<bool>,
    pub pull_interval_secs: Option<u64>,
    pub idle_timeout_secs: Option<u64>,
    pub close_to_tray: Option<bool>,
}

impl AutosyncPatch {
    /// `current` with this patch applied; a quiet window of 0 is refused.
    fn apply(&self, current: &AutosyncSettings) -> Result<AutosyncSettings, String> {
        if self.idle_timeout_secs == Some(0) {
            return Err("idle_timeout_secs must be > 0".to_string());
        }
        let mut next = current.clone();
        if let Some(enabled) = self.pull_enabled {
            next.pull.enabled = enabled;
        }
        if let Some(enabled) = self.push_enabled {
            next.push.enabled = enabled;
        }
        if let Some(secs) = self.pull_interval_secs {
            next.pull.focused_secs = secs;
        }
        if let Some(secs) = self.idle_timeout_secs {
            next.push.idle_timeout_secs = secs;
        }
        if let Some(on) = self.close_to_tray {
            next.close_to_tray = on;
        }
        Ok(next.normalised())
    }
}

/// Whether this transition is the "off -> on" edge that drops the paused set.
///
/// The rule `update_autosync_settings` already applies, named so both writers
/// apply the same one: a re-enable in either direction is a signal that the user
/// wants the watcher to retry every namespace.
///
/// Takes the two settings values rather than their four `enabled` flags:
/// `clippy::fn_params_excessive_bools` rejects a four-bool signature, and both
/// callers hold exactly these two values anyway.
const fn clears_pauses(prev: &AutosyncSettings, next: &AutosyncSettings) -> bool {
    !(prev.pull.enabled || prev.push.enabled) && (next.pull.enabled || next.push.enabled)
}

/// Whether a write turns publishing on. Every package with changes then waits a
/// full wait from now, however old its edits are.
const fn turns_publishing_on(prev: &AutosyncSettings, next: &AutosyncSettings) -> bool {
    !prev.push.enabled && next.push.enabled
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

#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct ExperimentalSettingsData {
    pub entire_package_sync: bool,
    pub agent_tasks: bool,
}

impl From<ExperimentalSettings> for ExperimentalSettingsData {
    fn from(s: ExperimentalSettings) -> Self {
        Self {
            entire_package_sync: s.entire_package_sync,
            agent_tasks: s.agent_tasks,
        }
    }
}

/// `task_settings.json` as written, for the Settings page.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct TaskSettingsData {
    /// Empty when there is no file, or it could not be read.
    pub settings: TaskSettings,
    /// Why the file could not be read. The page then offers no edit, so a
    /// save never replaces a file the user has to fix by hand.
    pub error: Option<String>,
}

impl TaskSettingsData {
    async fn read(data_dir: &std::path::Path) -> Self {
        match TaskSettings::read(data_dir).await {
            Ok(settings) => Self {
                settings,
                error: None,
            },
            Err(err) => Self {
                settings: TaskSettings::default(),
                error: Some(err.to_string()),
            },
        }
    }
}

/// `settings` as the file stores it, or why it cannot be saved: each bucket
/// as `s3://name`, each command with at least a program.
fn checked_task_settings(mut settings: TaskSettings) -> Result<TaskSettings, String> {
    for bucket in &mut settings.blessed_buckets {
        let name = bucket.trim();
        let name = name
            .strip_prefix("s3://")
            .unwrap_or(name)
            .trim_end_matches('/');
        if name.is_empty() || name.contains('/') {
            return Err(format!("Not a bucket name: {bucket}"));
        }
        *bucket = format!("s3://{name}");
    }
    for (name, agent) in &settings.agents {
        if name.trim().is_empty() {
            return Err("An agent has no name".to_string());
        }
        if agent
            .command
            .first()
            .is_none_or(|program| program.trim().is_empty())
        {
            return Err(format!("Agent {name} has no command"));
        }
    }
    Ok(settings)
}

/// Each built-in ignore rule as Settings shows it: its group, and a few
/// words on what it keeps out. In the order the popup lists them; one row
/// for each of `quiltignore::DEFAULT_RULES`.
const RULE_WORDS: &[(&str, &str, &str)] = &[
    (
        ".*/",
        "Hidden folders",
        "Any folder whose name starts with a dot, such as .git or .venv",
    ),
    (".DS_Store", "macOS", "Finder's folder settings"),
    (
        ".DS_Store?",
        "macOS",
        "Finder's folder settings, renamed on another system",
    ),
    (
        "._*",
        "macOS",
        "File details macOS keeps on drives it doesn't own",
    ),
    (".LSOverride", "macOS", "A folder's custom app choice"),
    (".VolumeIcon.icns", "macOS", "A drive's custom icon"),
    ("Icon[\r]", "macOS", "A folder's custom icon"),
    ("Thumbs.db", "Windows", "Picture thumbnails"),
    ("ehthumbs.db", "Windows", "Media Center thumbnails"),
    ("desktop.ini", "Windows", "A folder's view settings"),
    ("$RECYCLE.BIN/", "Windows", "A drive's Recycle Bin"),
    (
        ".directory",
        "Linux and network drives",
        "A KDE folder's view settings",
    ),
    (
        ".nfs*",
        "Linux and network drives",
        "Files a network drive keeps while they are open",
    ),
    ("~$*", "Editors and office lock files", "Office lock files"),
    (
        ".~lock.*#",
        "Editors and office lock files",
        "LibreOffice lock files",
    ),
    (".#*", "Editors and office lock files", "Emacs lock files"),
    (
        "\\#*\\#",
        "Editors and office lock files",
        "Emacs auto-saves",
    ),
    ("*.swp", "Editors and office lock files", "Vim swap files"),
    (
        "*.swo",
        "Editors and office lock files",
        "More Vim swap files",
    ),
    ("*~", "Editors and office lock files", "Editor backups"),
    (
        "*.code-workspace",
        "Editors and office lock files",
        "VS Code workspace settings",
    ),
    (".env", "Secrets", "Environment secrets"),
    (".env.*", "Secrets", "Their variants, such as .env.local"),
    (".netrc", "Secrets", "Saved logins"),
    ("*.pem", "Secrets", "Keys and certificates"),
    ("__pycache__/", "Python and Node", "Compiled Python"),
    ("*.py[cod]", "Python and Node", "Compiled Python files"),
    (
        "node_modules/",
        "Python and Node",
        "Installed Node packages",
    ),
];

/// One built-in ignore rule, its place in the popup, and whether it is on.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DefaultRuleData {
    pub rule: String,
    pub group: String,
    pub about: String,
    pub on: bool,
}

/// The built-in ignore rules, and the user-wide file that turns them off.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct IgnoreRulesData {
    pub file: String,
    /// `None` when the file could not be read.
    pub rules: Option<Vec<DefaultRuleData>>,
}

impl IgnoreRulesData {
    fn read(file: &std::path::Path) -> Self {
        let rules = quilt::quiltignore::default_rules(file)
            .inspect_err(|err| tracing::warn!("Could not read {}: {err}", file.display()))
            .ok()
            .map(|rules| {
                RULE_WORDS
                    .iter()
                    .filter_map(|(rule, group, about)| {
                        let on = rules.iter().find(|d| d.rule == *rule)?.on;
                        Some(DefaultRuleData {
                            rule: (*rule).to_string(),
                            group: (*group).to_string(),
                            about: (*about).to_string(),
                            on,
                        })
                    })
                    .collect()
            });
        Self {
            file: file.display().to_string(),
            rules,
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
    /// The saved choice, which applies after a restart.
    pub log_level: LogLevel,
    pub log_env: LogEnv,
    pub logs_dir: String,
    pub logs_dir_is_temporary: bool,
    pub os: String,
    pub changelog: Vec<changelog::ChangelogEntry>,
    pub publish: PublishSettingsData,
    pub autosync: AutosyncSettingsData,
    pub fswatcher: FsWatcherSettingsData,
    pub experimental: ExperimentalSettingsData,
    pub design: Design,
    pub ignore_rules: IgnoreRulesData,
    pub tasks: TaskSettingsData,
}

#[tauri::command]
pub async fn get_settings_data(
    m: tauri::State<'_, model::Model>,
    app: tauri::State<'_, app::App>,
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    publish: tauri::State<'_, SharedPublishSettings>,
    autosync_settings: tauri::State<'_, SharedAutosyncSettings>,
    fswatcher_settings: tauri::State<'_, SharedFsWatcherSettings>,
    experimental: tauri::State<'_, SharedExperimentalSettings>,
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
    // A broken file costs the dropdown its value, not the whole page; startup
    // already logged why.
    let log_level = LogSettings::load(&data_dir)
        .map(|s| s.level)
        .unwrap_or_default();
    let publish_data = PublishSettingsData::from(publish.read().await.clone());
    let autosync_data = AutosyncSettingsData::from(autosync_settings.read().await.clone());
    let fswatcher_data = FsWatcherSettingsData::from(fswatcher_settings.read().await.clone());
    let experimental_data = ExperimentalSettingsData::from(experimental.read().await.clone());
    // A broken file shows v2, the default, as a failed read in the UI does.
    let design = DesignSettings::load(&data_dir)
        .await
        .map(|s| s.design)
        .unwrap_or_default();
    let user_rules = m.get_quilt().lock().await.user_quiltignore();
    let tasks = TaskSettingsData::read(&data_dir).await;

    Ok(SettingsData {
        version: app.version.to_string(),
        home_dir,
        data_dir: data_dir.display().to_string(),
        auth_hosts,
        log_level,
        log_env: LogEnv::from_env(),
        logs_dir: app.logging.dir.path().display().to_string(),
        logs_dir_is_temporary: matches!(app.logging.dir, crate::telemetry::LogsDir::Temporary(_)),
        os: std::env::consts::OS.to_string(),
        changelog: changelog::latest_entries(),
        publish: publish_data,
        autosync: autosync_data,
        fswatcher: fswatcher_data,
        experimental: experimental_data,
        design,
        ignore_rules: IgnoreRulesData::read(&user_rules),
        tasks,
    })
}

/// Replace `task_settings.json`. The task runner reads it again on its next
/// check, so this needs no restart.
#[tauri::command]
pub async fn update_task_settings(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    settings: TaskSettings,
) -> Result<(), String> {
    let app_handle = app_handle.lock().await;
    let data_dir = app_handle
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?;
    save_task_settings(&data_dir, settings).await
}

/// Write `settings`, unless the file there now cannot be read: it may have
/// been broken by hand since the page read it, and is the user's to fix.
async fn save_task_settings(
    data_dir: &std::path::Path,
    settings: TaskSettings,
) -> Result<(), String> {
    let settings = checked_task_settings(settings)?;
    TaskSettings::read(data_dir)
        .await
        .map_err(|e| format!("Can't read task_settings.json: {e}"))?;
    settings.save(data_dir).await.map_err(|e| e.to_string())
}

/// Turn off exactly the built-in ignore rules in `off`, for every package.
/// The file watcher sees the user-wide file change and checks every package
/// again.
#[tauri::command]
pub async fn set_default_ignore_rules(
    m: tauri::State<'_, model::Model>,
    off: Vec<String>,
) -> Result<(), String> {
    // A clone, so the domain's mutex is not held while the file's lock waits.
    let domain = m.get_quilt().lock().await.clone();
    let off: Vec<&str> = off.iter().map(String::as_str).collect();
    domain
        .set_default_rules(&off)
        .await
        .map_err(|e| e.to_string())
}

/// The first line of a user-wide rules file *Open file* creates.
const USER_RULES_HEADER: &str =
    "# Your ignore rules for every package, one gitignore pattern per line.";

/// Open the user-wide rules file in the default application, creating it
/// first if missing.
#[tauri::command]
pub async fn open_user_ignore_rules(m: tauri::State<'_, model::Model>) -> Result<(), String> {
    let file = m.get_quilt().lock().await.user_quiltignore();
    if !file.exists() {
        quilt::quiltignore::append_lines(&file, &[USER_RULES_HEADER.to_string()])
            .map_err(|e| e.to_string())?;
    }
    open_in_text_editor(&file)
}

/// The file has no extension, so the system may have no app for it: macOS
/// asks for its text editor, and Windows falls back to Notepad.
fn open_in_text_editor(file: &std::path::Path) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        std::process::Command::new("open")
            .arg("-t")
            .arg(file)
            .status()
            .map_err(|e| e.to_string())
            .and_then(|status| {
                status
                    .success()
                    .then_some(())
                    .ok_or_else(|| format!("Could not open {}", file.display()))
            })
    }
    #[cfg(target_os = "windows")]
    {
        opener::open(file).or_else(|_| {
            std::process::Command::new("notepad.exe")
                .arg(file)
                .spawn()
                .map(drop)
                .map_err(|e| e.to_string())
        })
    }
    #[cfg(not(any(target_os = "macos", target_os = "windows")))]
    {
        opener::open(file).map_err(|e| e.to_string())
    }
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

    let clear = {
        let mut current = autosync_settings.write().await;
        let merged = merge_autosync_settings_data(&current, &settings);
        merged.save(&data_dir).await.map_err(|e| e.to_string())?;
        let clear = clears_pauses(&current, &merged);
        // Under the settings lock: a check that sees publishing on must also
        // see the new times.
        if turns_publishing_on(&current, &merged) {
            watcher.note_publishing_on().await;
        }
        *current = merged;
        clear
    };
    if clear {
        watcher.clear_all_paused().await;
    }
    watcher.settings_changed();
    Ok(())
}

/// Flip one direction of autosync from the v2 main page.
///
/// v2's own command rather than a call into `update_autosync_settings`, for the
/// reason §8 decision 2 gives: the main page knows two booleans and that command
/// takes all five settings fields, so calling it would mean the page first
/// fetching a settings payload it has no other use for. The two rules that
/// matter — merge, and clear on the off->on edge — are shared functions, not
/// duplicated logic.
#[tauri::command]
pub async fn set_autosync_direction(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    autosync_settings: tauri::State<'_, SharedAutosyncSettings>,
    watcher: tauri::State<'_, Watcher>,
    pull: Option<bool>,
    push: Option<bool>,
) -> Result<(), String> {
    let patch = AutosyncPatch {
        pull_enabled: pull,
        push_enabled: push,
        ..AutosyncPatch::default()
    };
    write_autosync_patch(&app_handle, &autosync_settings, &watcher, &patch).await
}

/// Change some of autosync's settings and leave the rest as stored: the v2
/// settings page saves each control as it changes.
#[tauri::command]
pub async fn patch_autosync_settings(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    autosync_settings: tauri::State<'_, SharedAutosyncSettings>,
    watcher: tauri::State<'_, Watcher>,
    patch: AutosyncPatch,
) -> Result<(), String> {
    write_autosync_patch(&app_handle, &autosync_settings, &watcher, &patch).await
}

async fn write_autosync_patch(
    app_handle: &sync::Mutex<tauri::AppHandle>,
    autosync_settings: &SharedAutosyncSettings,
    watcher: &Watcher,
    patch: &AutosyncPatch,
) -> Result<(), String> {
    let app_handle = app_handle.lock().await;
    let data_dir = app_handle
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?;

    let clear = {
        let mut current = autosync_settings.write().await;
        let next = patch.apply(&current)?;
        next.save(&data_dir).await.map_err(|e| e.to_string())?;
        let clear = clears_pauses(&current, &next);
        // Under the settings lock: a check that sees publishing on must also
        // see the new times.
        if turns_publishing_on(&current, &next) {
            watcher.note_publishing_on().await;
        }
        *current = next;
        clear
    };
    if clear {
        watcher.clear_all_paused().await;
    }
    watcher.settings_changed();
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

/// The transfer autopull is running now, for a window that mounts
/// mid-transfer; it then follows the `autopull-activity` event.
#[tauri::command]
pub async fn get_autopull_activity(
    watcher: tauri::State<'_, Watcher>,
) -> Result<Option<crate::autopull::activity::AutopullActivity>, String> {
    Ok(watcher.activity())
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

/// Save the log level. Logging reads it once at startup, so it applies after a
/// restart.
#[tauri::command]
pub async fn update_log_settings(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    level: LogLevel,
) -> Result<(), String> {
    let app_handle = app_handle.lock().await;
    let data_dir = app_handle
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?;
    LogSettings { level }
        .save(&data_dir)
        .await
        .map_err(|e| e.to_string())
}

/// Turn an experiment on or off.
///
/// A gate on a *control*, not on behaviour: enabling this downloads nothing by
/// itself, and disabling it leaves every per-package choice written where it
/// is, so re-enabling resumes them. Clearing those would destroy a decision the
/// user made in order to undo a setting that only ever hid a control.
#[tauri::command]
pub async fn update_experimental_settings(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    experimental: tauri::State<'_, SharedExperimentalSettings>,
    entire_package_sync: Option<bool>,
    agent_tasks: Option<bool>,
) -> Result<(), String> {
    let app_handle = app_handle.lock().await;
    let data_dir = app_handle
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?;

    // Read guard dropped before the write guard is taken.
    let mut new = { experimental.read().await.clone() };
    new.patch(entire_package_sync, agent_tasks);
    new.save(&data_dir).await.map_err(|e| e.to_string())?;
    *experimental.write().await = new;
    Ok(())
}

/// Choose the design generation. The next page that asks draws it.
#[tauri::command]
pub async fn update_design_settings(
    app_handle: tauri::State<'_, sync::Mutex<tauri::AppHandle>>,
    design: Design,
) -> Result<(), String> {
    let app_handle = app_handle.lock().await;
    let data_dir = app_handle
        .path()
        .app_local_data_dir()
        .map_err(|e| e.to_string())?;
    DesignSettings { design }
        .save(&data_dir)
        .await
        .map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    use crate::Error;

    fn tasks(buckets: &[&str], agents: &[(&str, &[&str])]) -> TaskSettings {
        TaskSettings {
            blessed_buckets: buckets.iter().map(ToString::to_string).collect(),
            agents: agents
                .iter()
                .map(|(name, command)| {
                    (
                        (*name).to_string(),
                        crate::tasks::settings::AgentSettings {
                            command: command.iter().map(ToString::to_string).collect(),
                            trust_all: false,
                        },
                    )
                })
                .collect(),
        }
    }

    #[test]
    fn task_buckets_are_stored_as_s3_uris() {
        let checked = checked_task_settings(tasks(&["quilt-dev", "s3://quilt-prod/ "], &[]));
        assert_eq!(
            checked.map(|s| s.blessed_buckets),
            Ok(vec![
                "s3://quilt-dev".to_string(),
                "s3://quilt-prod".to_string()
            ])
        );
    }

    #[test]
    fn task_settings_that_cannot_run_are_refused() {
        assert!(checked_task_settings(tasks(&["s3://"], &[])).is_err());
        assert!(checked_task_settings(tasks(&["quilt-dev/data"], &[])).is_err());
        assert!(checked_task_settings(tasks(&[], &[(" ", &["agent"])])).is_err());
        assert!(checked_task_settings(tasks(&[], &[("kiro", &[])])).is_err());
        assert!(checked_task_settings(tasks(&[], &[("kiro", &["kiro-cli", "acp"])])).is_ok());
    }

    /// A file the app cannot read says why, and shows as empty.
    #[tokio::test]
    async fn a_malformed_task_file_says_why() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        assert_eq!(TaskSettingsData::read(dir.path()).await.error, None);
        tokio::fs::write(dir.path().join("task_settings.json"), b"{\"agents\": [").await?;
        let data = TaskSettingsData::read(dir.path()).await;
        assert!(data.error.is_some());
        assert_eq!(data.settings, TaskSettings::default());
        Ok(())
    }

    /// A save writes a missing file, and leaves one it cannot read as it is.
    #[tokio::test]
    async fn a_save_never_replaces_a_file_it_cannot_read() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let settings = tasks(&["quilt-dev"], &[]);
        assert_eq!(
            save_task_settings(dir.path(), settings.clone()).await,
            Ok(())
        );
        let path = dir.path().join("task_settings.json");
        tokio::fs::write(&path, b"{\"agents\": [").await?;
        assert!(save_task_settings(dir.path(), settings).await.is_err());
        assert_eq!(tokio::fs::read(&path).await?, b"{\"agents\": [");
        Ok(())
    }

    /// A built-in rule added or removed in quilt-rs needs its words here.
    #[test]
    fn every_built_in_rule_has_one_row_of_words() {
        let mut rows: Vec<&str> = RULE_WORDS.iter().map(|(rule, ..)| *rule).collect();
        let mut rules = quilt::quiltignore::DEFAULT_RULES.to_vec();
        rows.sort_unstable();
        rules.sort_unstable();
        assert_eq!(rows, rules);
    }

    #[test]
    fn data_from_settings_projects_focused_onto_pull_interval() {
        let s = AutosyncSettings {
            pull: PullSettings {
                enabled: true,
                focused_secs: 180,
                unfocused_secs: 999, // retired, never shown
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
        assert_eq!(data.pull_interval_secs, 180, "the one interval");
        assert_eq!(data.idle_timeout_secs, 22);
    }

    #[tokio::test]
    async fn merge_preserves_the_retired_intervals() -> Result<(), Error> {
        // Save a settings file with non-default `unfocused_secs` and
        // `closed_secs`, then run the merge logic with arbitrary UI-owned
        // values, and verify both survive.
        let dir = TempDir::new().unwrap();
        let initial = AutosyncSettings {
            pull: PullSettings {
                enabled: true,
                focused_secs: 60,
                unfocused_secs: 77,
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
        assert_eq!(merged.pull.focused_secs, 60, "7 s runs as one minute");
        assert_eq!(
            merged.pull.unfocused_secs, 77,
            "unfocused_secs must flow through untouched"
        );
        assert_eq!(
            merged.pull.closed_secs, 999,
            "closed_secs must flow through untouched"
        );
        assert_eq!(merged.push.idle_timeout_secs, 9);
        Ok(())
    }

    /// The v1 popup's seconds field: what it saves is what runs, and what
    /// its next load shows.
    #[test]
    fn a_v1_save_runs_in_whole_minutes() {
        for (typed, runs) in [
            (0, 60),
            (30, 60),
            (45, 60),
            (90, 120),
            (150, 180),
            (300, 300),
        ] {
            let incoming = AutosyncSettingsData {
                pull_interval_secs: typed,
                ..AutosyncSettingsData::from(AutosyncSettings::default())
            };
            assert!(validate_autosync_settings_data(&incoming).is_ok());
            let merged = merge_autosync_settings_data(&AutosyncSettings::default(), &incoming);
            assert_eq!(merged.pull.focused_secs, runs, "{typed} s");
            assert_eq!(
                AutosyncSettingsData::from(merged).pull_interval_secs,
                runs,
                "and shows it"
            );
        }
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
    fn setting_one_direction_leaves_the_other_and_the_intervals_alone() {
        // The main page knows two booleans; `AutosyncSettingsData` has five fields.
        // A write that sent defaults for the other three would silently reset the
        // user's pull interval and quiet window from a card that never showed them.
        let current = AutosyncSettings {
            pull: PullSettings {
                enabled: false,
                focused_secs: 300,
                unfocused_secs: 60,
                closed_secs: 900,
            },
            push: PushSettings {
                enabled: true,
                idle_timeout_secs: 45,
            },
            close_to_tray: true,
        };
        let next = AutosyncPatch {
            pull_enabled: Some(true),
            ..AutosyncPatch::default()
        }
        .apply(&current)
        .unwrap();
        assert!(next.pull.enabled);
        assert!(next.push.enabled, "the untouched direction is untouched");
        assert_eq!(next.pull.focused_secs, 300);
        assert_eq!(next.pull.unfocused_secs, 60);
        assert_eq!(next.pull.closed_secs, 900);
        assert_eq!(next.push.idle_timeout_secs, 45);
        assert!(next.close_to_tray);
    }

    /// The settings page's minute field writes the one interval only: the
    /// retired unfocused and closed ones are not its to change.
    #[test]
    fn a_pull_interval_patch_leaves_unfocused_and_closed_alone() {
        let current = AutosyncSettings {
            pull: PullSettings {
                enabled: true,
                focused_secs: 60,
                unfocused_secs: 120,
                closed_secs: 600,
            },
            push: PushSettings {
                enabled: false,
                idle_timeout_secs: 300,
            },
            close_to_tray: false,
        };
        let next = AutosyncPatch {
            pull_interval_secs: Some(180),
            ..AutosyncPatch::default()
        }
        .apply(&current)
        .unwrap();
        assert_eq!(next.pull.focused_secs, 180);
        assert_eq!(next.pull.unfocused_secs, 120);
        assert_eq!(next.pull.closed_secs, 600);
        assert_eq!(
            next,
            AutosyncSettings {
                pull: PullSettings {
                    focused_secs: 180,
                    ..current.pull.clone()
                },
                ..current
            },
            "and nothing else moves"
        );
    }

    #[test]
    fn a_patch_writes_each_field_it_names() {
        let next = AutosyncPatch {
            pull_enabled: Some(true),
            push_enabled: Some(true),
            pull_interval_secs: Some(60),
            idle_timeout_secs: Some(120),
            close_to_tray: Some(true),
        }
        .apply(&AutosyncSettings::default())
        .unwrap();
        assert!(next.pull.enabled && next.push.enabled && next.close_to_tray);
        assert_eq!(next.pull.focused_secs, 60);
        assert_eq!(next.push.idle_timeout_secs, 120);
    }

    #[test]
    fn a_patch_refuses_a_zero_quiet_window() {
        let patch = AutosyncPatch {
            idle_timeout_secs: Some(0),
            ..AutosyncPatch::default()
        };
        assert!(patch.apply(&AutosyncSettings::default()).is_err());
    }

    #[test]
    fn a_patch_runs_the_interval_in_whole_minutes() {
        for (typed, runs) in [
            (0, 60),
            (30, 60),
            (45, 60),
            (90, 120),
            (150, 180),
            (300, 300),
        ] {
            let next = AutosyncPatch {
                pull_interval_secs: Some(typed),
                ..AutosyncPatch::default()
            }
            .apply(&AutosyncSettings::default())
            .unwrap();
            assert_eq!(next.pull.focused_secs, runs, "{typed} s");
        }
    }

    /// A toggle writes the interval as it is; only the minutes are tidied.
    #[test]
    fn a_toggle_keeps_a_stored_interval() {
        let current = AutosyncSettings {
            pull: PullSettings {
                focused_secs: 600,
                ..PullSettings::default()
            },
            ..AutosyncSettings::default()
        };
        let next = AutosyncPatch {
            push_enabled: Some(true),
            ..AutosyncPatch::default()
        }
        .apply(&current)
        .unwrap();
        assert_eq!(next.pull, current.pull);
    }

    #[test]
    fn turning_everything_off_and_one_thing_on_is_the_edge_that_clears_pauses() {
        // The rule `update_autosync_settings` already applies: a re-enable in either
        // direction means the user wants every namespace retried, not just the ones
        // they touched. A second write path that skipped it would leave the user
        // flipping a switch that used to clear pauses and now does not.

        /// One moment's pair of direction flags. Nothing else matters to the rule.
        fn enabled(pull: bool, push: bool) -> AutosyncSettings {
            AutosyncSettings {
                pull: PullSettings {
                    enabled: pull,
                    ..Default::default()
                },
                push: PushSettings {
                    enabled: push,
                    ..Default::default()
                },
                close_to_tray: false,
            }
        }
        assert!(clears_pauses(&enabled(false, false), &enabled(true, false)));
        assert!(clears_pauses(&enabled(false, false), &enabled(false, true)));
        assert!(!clears_pauses(&enabled(true, false), &enabled(true, true)));
        assert!(!clears_pauses(&enabled(true, true), &enabled(false, false)));
        // Publishing on is its own edge: it restarts every waiting package's
        // countdown, whatever the pull side does.
        assert!(turns_publishing_on(
            &enabled(true, false),
            &enabled(true, true)
        ));
        assert!(turns_publishing_on(
            &enabled(false, false),
            &enabled(false, true)
        ));
        assert!(!turns_publishing_on(
            &enabled(true, true),
            &enabled(false, true)
        ));
        assert!(!turns_publishing_on(
            &enabled(false, true),
            &enabled(false, false)
        ));
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
