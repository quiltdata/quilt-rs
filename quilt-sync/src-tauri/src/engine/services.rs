//! Everything the commands and the sync engine share, built without Tauri.
//!
//! The desktop app builds one `Arc<Services>`, manages it, and runs every
//! command through [`crate::engine::dispatch`].

use std::path::PathBuf;
use std::sync::Arc;

use semver::Version;
use tokio::sync::RwLock;
use tokio::sync::watch;

use crate::engine::Result;
use crate::engine::action_needed;
use crate::engine::action_needed::ActionNeeded;
use crate::engine::app::App;
use crate::engine::autopull;
use crate::engine::autopull::StatusReporter;
use crate::engine::autopull::SyncTrayStatus;
use crate::engine::autopull::Watcher;
use crate::engine::autopull::reporter::SinkEventReporter;
use crate::engine::autopull::reporter::TelemetryReporter;
use crate::engine::commands::RoleCache;
use crate::engine::commands::WorkflowRulesCache;
use crate::engine::dataspace::Dataspace;
use crate::engine::events::EventSink;
use crate::engine::experimental_settings;
use crate::engine::experimental_settings::SharedExperimentalSettings;
use crate::engine::fswatcher;
use crate::engine::fswatcher::SharedFsWatcherSettings;
use crate::engine::model::Model;
use crate::engine::navigation::Navigations;
use crate::engine::oauth::OAuthState;
use crate::engine::publish_settings;
use crate::engine::publish_settings::SharedPublishSettings;
use crate::engine::quilt;
use crate::engine::quit::QuitGate;
use crate::engine::record::Outside;
use crate::engine::record::Record;
use crate::engine::telemetry;
use crate::engine::telemetry::Telemetry;
use crate::engine::telemetry::prelude::*;
use crate::engine::updates::Updates;

pub struct Services {
    pub data_dir: PathBuf,
    pub model: Arc<Model>,
    /// The engine on the dataspace API, where the switch turned it on and
    /// it opened; `None` runs every command on the ref API, as before.
    pub dataspace: Option<Arc<Dataspace>>,
    /// The dataspace engine's folder watch, held for as long as the app runs.
    dataspace_watch: std::sync::Mutex<Option<DataspaceWatch>>,
    pub workflow_rules: WorkflowRulesCache,
    pub roles: Arc<RoleCache>,
    pub app: App,
    pub telemetry: Arc<Telemetry>,
    pub oauth: OAuthState,
    /// What happened, for the inbox.
    pub record: Arc<Record>,
    pub publish_settings: SharedPublishSettings,
    pub autosync_settings: autopull::SharedAutosyncSettings,
    pub experimental_settings: SharedExperimentalSettings,
    pub fswatcher_settings: SharedFsWatcherSettings,
    pub watcher: Watcher,
    pub quit_gate: QuitGate,
    /// Held by every settings write, so two saves of one file never overlap.
    pub settings_writes: tokio::sync::Mutex<()>,
    pub events: EventSink,
    pub navigations: Navigations,
    pub updates: Updates,
    /// The tray icon's state; a desktop tray clones it.
    pub tray_status: watch::Receiver<SyncTrayStatus>,
}

impl Services {
    /// Build the services and start the sync engine on the current runtime.
    ///
    /// # Errors
    ///
    /// When file logging cannot start.
    #[allow(
        clippy::too_many_lines,
        reason = "one step per service, in the order they depend on each other"
    )]
    pub async fn init(data_dir: PathBuf, version: Version) -> Result<Arc<Self>> {
        let app_version = version.clone();
        crate::engine::events::set_runtime(tokio::runtime::Handle::current());

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
        let paths = quilt::paths::DomainPaths::new(data_dir.clone());
        let state_store = Arc::new(quilt::state_store::PackageStateStore::new(paths.state_db()));
        let last_change = autopull::last_change::LastChange::shared(Arc::clone(&state_store));
        let events = EventSink::new();
        let record = Arc::new(Record::open(&data_dir, events.clone()).await);
        debug!("Root directory is {data_dir:?}");
        let domain = quilt::LocalDomain::new(&data_dir).with_state_store(state_store);
        if let Err(err) = domain.sweep_leftovers().await {
            error!("Failed to sweep what killed lineage writes left: {err}");
        }
        // Every lineage write of this process goes through its hook, so a
        // revision that moved without it was moved by another tool.
        let outside =
            Outside::open(&data_dir, domain.lineage_io().clone(), Arc::clone(&record)).await;
        let dataspace = open_dataspace(&experimental_settings, &data_dir, &domain).await;
        let model = Arc::new(Model::from_domain(domain.with_lineage_hook(outside.hook())));
        let roles = Arc::new(RoleCache::default());

        let action_needed = Arc::new(ActionNeeded::default());
        let reporter = reporter(&events, &record, &action_needed, &telemetry);
        let (watcher, tray_status) = Watcher::spawn(
            data_dir.clone(),
            Arc::clone(&model),
            Arc::clone(&roles),
            autosync_settings.clone(),
            publish_settings.clone(),
            reporter.clone(),
            last_change,
            Arc::clone(&action_needed),
            outside,
            dataspace.clone(),
        );
        autopull::activity::spawn_forwarder(events.clone(), watcher.subscribe_activity());
        action_needed::spawn_forwarder(events.clone(), action_needed.subscribe());
        let dataspace_watch = watch(
            dataspace.as_ref(),
            &model,
            &watcher,
            &fswatcher_settings,
            &reporter,
        )
        .await;

        Ok(Arc::new(Self {
            data_dir,
            model,
            dataspace,
            dataspace_watch: std::sync::Mutex::new(dataspace_watch),
            workflow_rules: WorkflowRulesCache::default(),
            roles,
            app: App::new(version, logging),
            telemetry,
            oauth: OAuthState::default(),
            record,
            publish_settings,
            autosync_settings,
            experimental_settings,
            fswatcher_settings,
            watcher,
            quit_gate: QuitGate::default(),
            settings_writes: tokio::sync::Mutex::new(()),
            events,
            navigations: Navigations::default(),
            updates: Updates::new(app_version),
            tray_status,
        }))
    }

    /// Moves the dataspace engine's autosync and folder watch to the default
    /// dataspace, after a setup changed it.
    pub async fn rewatch_dataspace(&self) {
        let Some(ds) = &self.dataspace else {
            return;
        };
        let next = watch_dataspace(ds, &self.fswatcher_settings).await;
        let mut held = self
            .dataspace_watch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        *held = next;
    }
}

/// The dataspace engine, where the switch in `settings` is on: on the Domain
/// [`quilt::domain::locate`] names and the default dataspace, else the ref
/// path's home folder. `None`, logged, where it cannot open, as before a
/// home folder is chosen: the app then runs on the ref API.
async fn open_dataspace(
    settings: &SharedExperimentalSettings,
    data_dir: &std::path::Path,
    domain: &quilt::LocalDomain,
) -> Option<Arc<Dataspace>> {
    if !settings.read().await.dataspace {
        return None;
    }
    let Some(domain_dir) = quilt::domain::locate(None) else {
        error!("The dataspace engine is on, but the OS has no local data folder for the Domain");
        return None;
    };
    let home = match domain.get_home().await {
        Ok(home) => home.as_ref().clone(),
        Err(err) => {
            warn!("The dataspace engine waits for a home folder: {err}");
            return None;
        }
    };
    match Dataspace::open(
        domain.remote_handle(),
        domain_dir,
        data_dir.to_path_buf(),
        &home,
        domain.user_quiltignore(),
    )
    .await
    {
        Ok(dataspace) => Some(Arc::new(dataspace)),
        Err(err) => {
            error!("The dataspace engine could not open, so the ref engine runs: {err}");
            None
        }
    }
}

type DataspaceWatch = notify_debouncer_full::Debouncer<
    notify::RecommendedWatcher,
    notify_debouncer_full::RecommendedCache,
>;

/// Starts watching folders: the dataspace engine's dataspace for its
/// runner, or the ref engine's installed packages.
async fn watch(
    dataspace: Option<&Arc<Dataspace>>,
    model: &Arc<Model>,
    watcher: &Watcher,
    settings: &SharedFsWatcherSettings,
    reporter: &Arc<dyn StatusReporter>,
) -> Option<DataspaceWatch> {
    if let Some(ds) = dataspace {
        return watch_dataspace(ds, settings).await;
    }
    fswatcher::spawn(
        Arc::clone(model),
        watcher.shared_arc(),
        settings.clone(),
        reporter,
    )
    .await;
    None
}

/// Puts the default dataspace on autosync, so the runner ticks it, and
/// watches its folder where watching is on. `None`, logged, where the
/// watch cannot start: the tick still finds what `latest` moved.
async fn watch_dataspace(
    ds: &Arc<Dataspace>,
    settings: &SharedFsWatcherSettings,
) -> Option<DataspaceWatch> {
    if let Err(err) = ds.autosync_on().await {
        error!("The default dataspace could not be put on autosync: {err}");
    }
    let (enabled, debounce_ms) = {
        let settings = settings.read().await;
        (settings.enabled, settings.debounce_ms)
    };
    if !enabled {
        return None;
    }
    let debounce = std::time::Duration::from_millis(debounce_ms);
    match crate::engine::dataspace::autosync::watch(Arc::clone(ds), debounce) {
        Ok(watch) => Some(watch),
        Err(err) => {
            error!("The dataspace watcher could not start: {err}");
            None
        }
    }
}

/// The watcher's reporter. Telemetry wraps the UI reporter rather than
/// replacing it: the engine's outcomes reach the window exactly as before, and
/// telemetry observes them on the way past.
fn reporter(
    events: &EventSink,
    record: &Arc<Record>,
    action_needed: &Arc<ActionNeeded>,
    telemetry: &Arc<Telemetry>,
) -> Arc<dyn StatusReporter> {
    Arc::new(TelemetryReporter::wrapping(
        Arc::new(SinkEventReporter::new(
            events.clone(),
            Arc::clone(record),
            Arc::clone(action_needed),
        )),
        Arc::clone(telemetry),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::commands;

    /// The services start with no Tauri app, answer commands as plain
    /// functions and announce on the event sink.
    #[tokio::test(flavor = "multi_thread")]
    async fn services_run_without_tauri() {
        let data_dir = tempfile::tempdir().expect("temp data dir");
        let s = Services::init(data_dir.path().to_path_buf(), Version::new(0, 0, 0))
            .await
            .expect("services");
        let mut events = s.events.subscribe();

        assert_eq!(commands::get_record(&s).await, Ok(vec![]));
        assert!(commands::get_settings_data(&s).await.is_ok());

        let id = s
            .record
            .add(crate::engine::record::Draft::uninstalled(
                &("acme", "demo").into(),
                None,
            ))
            .await;
        loop {
            let (name, payload) =
                tokio::time::timeout(std::time::Duration::from_secs(5), events.recv())
                    .await
                    .expect("an event in time")
                    .expect("the sink is open");
            if name == crate::engine::record::RECORD_EVENT {
                assert_eq!(payload, serde_json::Value::Null);
                break;
            }
        }
        assert_eq!(commands::get_record(&s).await.unwrap()[0].entry.id, id);
    }
}
