//! Everything the commands and the sync engine share, built without Tauri.
//!
//! The desktop app manages one `Arc<Services>` and its commands are thin
//! wrappers over the plain functions in [`crate::commands`]; a headless run
//! builds the same `Services` and reaches the commands through
//! [`crate::dispatch::dispatch`].

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::OnceLock;
use std::sync::atomic::AtomicUsize;

use semver::Version;
use tokio::sync::RwLock;
use tokio::sync::watch;

use crate::Result;
use crate::app::App;
use crate::autopull;
use crate::autopull::StatusReporter;
use crate::autopull::SyncTrayStatus;
use crate::autopull::Watcher;
use crate::autopull::reporter::SinkEventReporter;
use crate::autopull::reporter::TelemetryReporter;
use crate::commands::RoleCache;
use crate::commands::WorkflowRulesCache;
use crate::events::EventSink;
use crate::experimental_settings;
use crate::experimental_settings::SharedExperimentalSettings;
use crate::fswatcher;
use crate::fswatcher::SharedFsWatcherSettings;
use crate::model::Model;
use crate::oauth::OAuthState;
use crate::publish_settings;
use crate::publish_settings::SharedPublishSettings;
use crate::quilt;
use crate::quit::QuitGate;
use crate::routes;
use crate::telemetry;
use crate::telemetry::Telemetry;
use crate::telemetry::prelude::*;
use crate::toast::SinkToastEmitter;
use crate::toast::ToastCenter;

/// The identifier in `tauri.conf.json`, which names the data folder.
pub const APP_IDENTIFIER: &str = "com.quiltdata.quilt-sync";

/// The folder Tauri's `app_local_data_dir()` resolves to: the platform's
/// local data folder joined with the app identifier. On macOS that is
/// `~/Library/Application Support/<id>`, on Linux `$XDG_DATA_HOME/<id>` or
/// `~/.local/share/<id>`, on Windows `%LOCALAPPDATA%\<id>`.
#[must_use]
pub fn default_data_dir() -> PathBuf {
    if let Some(dir) = data_dir_override() {
        return dir;
    }
    dirs::data_local_dir()
        .expect("no local data folder on this platform")
        .join(APP_IDENTIFIER)
}

/// `QUILTSYNC_DATA_DIR`, a stand-in for a `--data-dir` flag: run against a
/// throwaway folder instead of the real one.
#[must_use]
pub fn data_dir_override() -> Option<PathBuf> {
    std::env::var_os("QUILTSYNC_DATA_DIR")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
}

pub struct Services {
    pub data_dir: PathBuf,
    pub model: Arc<Model>,
    pub workflow_rules: WorkflowRulesCache,
    pub roles: Arc<RoleCache>,
    pub app: App,
    pub telemetry: Arc<Telemetry>,
    pub oauth: OAuthState,
    pub toasts: Arc<ToastCenter>,
    pub publish_settings: SharedPublishSettings,
    pub autosync_settings: autopull::SharedAutosyncSettings,
    pub experimental_settings: SharedExperimentalSettings,
    pub fswatcher_settings: SharedFsWatcherSettings,
    pub watcher: Watcher,
    pub quit_gate: QuitGate,
    pub events: EventSink,
    /// The tray icon's state; a desktop tray clones it.
    pub tray_status: watch::Receiver<SyncTrayStatus>,
    /// The desktop app, set once its setup ran. Desktop-only commands work
    /// through [`crate::dispatch::dispatch`] when it is there; a headless run
    /// never sets it.
    pub desktop: OnceLock<tauri::AppHandle>,
    /// How many WebSocket front ends are connected right now.
    pub web_clients: AtomicUsize,
}

impl Services {
    /// Build the services and start the sync engine on the current runtime.
    ///
    /// # Errors
    ///
    /// When file logging cannot start.
    pub async fn init(data_dir: PathBuf, version: Version) -> Result<Arc<Self>> {
        crate::events::set_runtime(tokio::runtime::Handle::current());

        // Resolved before the sinks are built: the crash client's event hook
        // captures the identity when the client is constructed, so it has to
        // be known first or crashes would go out unattributed.
        let sinks = telemetry::Sinks::resolve();
        let telemetry = Arc::new(Telemetry::new(
            &version,
            sinks,
            telemetry::InstallId::load(&data_dir),
            Some(telemetry::Buffer::new(&data_dir)),
        ));

        let logging = Telemetry::init_file_logging(&data_dir)?;
        telemetry.init();

        let publish_settings = publish_settings::init(&data_dir)
            .await
            .unwrap_or_else(|err| {
                error!("Failed to load publish settings, using defaults: {err}");
                Arc::new(RwLock::new(publish_settings::PublishSettings::default()))
            });
        let autosync_settings = autopull::init_settings(&data_dir)
            .await
            .unwrap_or_else(|err| {
                error!("Failed to load autosync settings, using defaults: {err}");
                Arc::new(RwLock::new(autopull::AutosyncSettings::default()))
            });
        let experimental_settings =
            experimental_settings::init(&data_dir)
                .await
                .unwrap_or_else(|err| {
                    error!("Failed to load experimental settings, using defaults: {err}");
                    Arc::new(RwLock::new(
                        experimental_settings::ExperimentalSettings::default(),
                    ))
                });
        let fswatcher_settings = fswatcher::init_settings(&data_dir)
            .await
            .unwrap_or_else(|err| {
                error!("Failed to load fswatcher settings, using defaults: {err}");
                Arc::new(RwLock::new(fswatcher::FsWatcherSettings::default()))
            });

        // Only the app opens the package state store; the CLI never does.
        // One store for the last-change times and the walks' stat cache.
        let state_store = Arc::new(quilt::state_store::PackageStateStore::new(
            quilt::paths::DomainPaths::new(data_dir.clone()).state_db(),
        ));
        let last_change = autopull::last_change::LastChange::shared(Arc::clone(&state_store));
        let model = Arc::new(Model::create_with_state_store(
            data_dir.clone(),
            state_store,
        ));
        let roles = Arc::new(RoleCache::default());
        let events = EventSink::new();
        let toasts = Arc::new(ToastCenter::new(Box::new(SinkToastEmitter::new(
            events.clone(),
        ))));

        // Telemetry wraps the UI reporter rather than replacing it: the
        // engine's outcomes reach the window exactly as before, and telemetry
        // observes them on the way past.
        let reporter: Arc<dyn StatusReporter> = Arc::new(TelemetryReporter::wrapping(
            Arc::new(SinkEventReporter::new(events.clone(), Arc::clone(&toasts))),
            Arc::clone(&telemetry),
        ));
        let (watcher, tray_status) = Watcher::spawn(
            data_dir.clone(),
            Arc::clone(&model),
            Arc::clone(&roles),
            autosync_settings.clone(),
            publish_settings.clone(),
            reporter.clone(),
            last_change,
        );
        autopull::activity::spawn_forwarder(events.clone(), watcher.subscribe_activity());
        fswatcher::spawn(
            Arc::clone(&model),
            watcher.shared_arc(),
            fswatcher_settings.clone(),
            &reporter,
        )
        .await;

        Ok(Arc::new(Self {
            data_dir,
            model,
            workflow_rules: WorkflowRulesCache::default(),
            roles,
            app: App::new(version, logging),
            telemetry,
            oauth: OAuthState::default(),
            toasts,
            publish_settings,
            autosync_settings,
            experimental_settings,
            fswatcher_settings,
            watcher,
            quit_gate: QuitGate::default(),
            events,
            tray_status,
            desktop: OnceLock::new(),
            web_clients: AtomicUsize::new(0),
        }))
    }

    /// Tell a window-less front end to show `path`: a `navigate` event with
    /// the path and query, e.g. `{"url": "/settings"}`.
    pub fn navigate(&self, path: routes::Paths) {
        let base = url::Url::parse("http://localhost/").expect("static URL");
        let url = routes::from_url(path, base);
        let target = match url.query() {
            Some(query) => format!("{}?{query}", url.path()),
            None => url.path().to_owned(),
        };
        self.events.emit(
            crate::events::NAVIGATE_EVENT,
            serde_json::json!({ "url": target }),
        );
    }
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;
    use crate::dispatch::dispatch;

    #[test]
    fn the_data_folder_is_named_by_the_tauri_identifier() {
        let conf: serde_json::Value =
            serde_json::from_str(include_str!("../tauri.conf.json")).expect("tauri.conf.json");
        assert_eq!(conf["identifier"], APP_IDENTIFIER);
        assert!(default_data_dir().ends_with(APP_IDENTIFIER));
    }

    /// The services start with no Tauri app, answer commands through
    /// `dispatch` and announce on the event sink.
    #[tokio::test(flavor = "multi_thread")]
    async fn services_run_without_tauri() {
        let data_dir = tempfile::tempdir().expect("temp data dir");
        let s = Services::init(data_dir.path().to_path_buf(), Version::new(0, 0, 0))
            .await
            .expect("services");
        let mut events = s.events.subscribe();

        assert_eq!(dispatch(&s, "get_toasts", json!({})).await, Ok(json!([])));
        assert_eq!(
            dispatch(&s, "dismiss_toast", json!({"id": 7})).await,
            Ok(serde_json::Value::Null)
        );
        assert_eq!(
            dispatch(&s, "login_oauth", json!({"host": "a.test"})).await,
            Err(json!(crate::dispatch::NOT_IN_BROWSER))
        );
        assert!(dispatch(&s, "get_settings_data", json!({})).await.is_ok());

        s.toasts
            .post(crate::toast::ToastDraft {
                body: "hello".into(),
                ..Default::default()
            })
            .await;
        s.navigate(routes::Paths::Settings);
        let mut seen = Vec::new();
        while seen.len() < 2 {
            let (name, payload) =
                tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
                    .await
                    .expect("an event in time")
                    .expect("the sink is open");
            if name == "toast" || name == "navigate" {
                seen.push((name, payload));
            }
        }
        assert_eq!(seen[0].0, "toast");
        assert_eq!(seen[0].1["body"], "hello");
        assert_eq!(
            seen[1],
            ("navigate".to_owned(), json!({"url": "/settings"}))
        );
    }
}
