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
use crate::engine::model::QuiltModel;
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
    /// What the cutover's import did on this start, or why it fell back to
    /// the ref engine, for the desktop to show once; `None` where nothing
    /// happened.
    pub cutover_notice: Option<String>,
    /// The dataspace engine's folder watch and task dispatch, held for as
    /// long as the app runs.
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
        // Built on the app's folder apart from the ref path, and shared by
        // both engines: signing in reaches each through one cache.
        let remote = Arc::new(quilt::io::remote::RemoteS3::new(
            paths.clone(),
            quilt::io::storage::LocalStorage::new(),
        ));
        let domain = quilt::LocalDomain::new(&data_dir)
            .with_remote(Arc::clone(&remote))
            .with_state_store(state_store);
        if let Err(err) = domain.sweep_leftovers().await {
            error!("Failed to sweep what killed lineage writes left: {err}");
        }
        // Every lineage write of this process goes through its hook, so a
        // revision that moved without it was moved by another tool.
        let outside =
            Outside::open(&data_dir, domain.lineage_io().clone(), Arc::clone(&record)).await;
        let (dataspace, cutover_notice) =
            open_dataspace(&experimental_settings, &data_dir, &domain, &remote).await;
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
            &data_dir,
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
            cutover_notice,
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
        let next = watch_dataspace(ds, &self.data_dir, &self.fswatcher_settings).await;
        let mut held = self
            .dataspace_watch
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        // The old dispatch goes first, so it never runs beside the new one.
        *held = None;
        *held = Some(next);
    }

    /// The home folder the Settings page and the diagnostics show: the
    /// default dataspace with the dataspace engine on, else the ref engine's.
    pub async fn home_dir(&self) -> Option<PathBuf> {
        match &self.dataspace {
            Some(ds) => Some(ds.dataspace()),
            None => self
                .model
                .get_quilt()
                .lock()
                .await
                .get_home()
                .await
                .ok()
                .map(|home| home.as_ref().clone()),
        }
    }

    /// The user-wide ignore rules file, in the app's folder.
    pub fn user_rules(&self) -> PathBuf {
        user_rules(&self.data_dir)
    }
}

/// The user-wide ignore rules file of the app whose folder is `data_dir`.
fn user_rules(data_dir: &std::path::Path) -> PathBuf {
    quilt::paths::DomainPaths::new(data_dir.to_path_buf()).user_quiltignore()
}

/// The Domain's folder: the one [`quilt::domain::locate`] names.
#[cfg(not(test))]
fn domain_dir(_data_dir: &std::path::Path) -> Option<PathBuf> {
    quilt::domain::locate(None)
}

/// The Domain's folder under test: inside the test's own app folder, never
/// the user's.
#[cfg(test)]
#[allow(clippy::unnecessary_wraps, reason = "the signature of the real lookup")]
fn domain_dir(data_dir: &std::path::Path) -> Option<PathBuf> {
    Some(data_dir.join(quilt::domain::DOMAIN_DIR_NAME))
}

/// The dataspace engine, where the switch in `settings` is on: on the Domain
/// [`quilt::domain::locate`] names and the default dataspace, else the ref
/// path's home folder. On the first start with the switch on, the packages
/// the ref engine kept are imported first, once; with them the notice to
/// show. `None`, logged, where the import falls back or the engine cannot
/// open, as before a home folder is chosen: the app then runs on the ref
/// API.
async fn open_dataspace(
    settings: &SharedExperimentalSettings,
    data_dir: &std::path::Path,
    domain: &quilt::LocalDomain,
    remote: &Arc<quilt::io::remote::RemoteS3>,
) -> (Option<Arc<Dataspace>>, Option<String>) {
    if !settings.read().await.dataspace() {
        return (None, None);
    }
    let Some(domain_dir) = domain_dir(data_dir) else {
        error!("The dataspace engine is on, but the OS has no local data folder for the Domain");
        return (None, None);
    };
    let found = Box::pin(quilt::domain::import::cutover(
        &quilt::io::storage::LocalStorage::new(),
        data_dir,
        &quilt::domain::DomainRoot::new(&domain_dir),
    ))
    .await;
    let notice = found.report();
    if let Some(report) = &notice {
        if found.on() {
            info!("{report}");
        } else {
            error!("{report}");
        }
    }
    if !found.on() {
        return (None, notice);
    }
    let home = match domain.get_home().await {
        Ok(home) => home.as_ref().clone(),
        Err(err) => {
            warn!("The dataspace engine waits for a home folder: {err}");
            return (None, notice);
        }
    };
    let opened = Dataspace::open(
        Arc::clone(remote),
        domain_dir,
        data_dir.to_path_buf(),
        &home,
        user_rules(data_dir),
    )
    .await;
    match opened {
        Ok(dataspace) => (Some(Arc::new(dataspace)), notice),
        Err(err) => {
            error!("The dataspace engine could not open, so the ref engine runs: {err}");
            (None, notice)
        }
    }
}

type FolderWatch = notify_debouncer_full::Debouncer<
    notify::RecommendedWatcher,
    notify_debouncer_full::RecommendedCache,
>;

/// What the dataspace engine holds for its default dataspace while it runs.
struct DataspaceWatch {
    /// `None` where watching is off or could not start.
    _folders: Option<FolderWatch>,
    /// `agent_exec`'s dispatch from the `post-read` hook.
    _tasks: quilt::dataspace::hooks::Registration,
}

/// Starts watching folders: the dataspace engine's dataspace for its
/// runner, or the ref engine's installed packages.
async fn watch(
    dataspace: Option<&Arc<Dataspace>>,
    data_dir: &std::path::Path,
    model: &Arc<Model>,
    watcher: &Watcher,
    settings: &SharedFsWatcherSettings,
    reporter: &Arc<dyn StatusReporter>,
) -> Option<DataspaceWatch> {
    if let Some(ds) = dataspace {
        return Some(watch_dataspace(ds, data_dir, settings).await);
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

/// Puts the default dataspace on autosync, so the runner ticks it, watches
/// its folder where watching is on, and dispatches `agent_exec` from its
/// `post-read` hook. The folder watch is `None`, logged, where it cannot
/// start: the tick still finds what `latest` moved.
async fn watch_dataspace(
    ds: &Arc<Dataspace>,
    data_dir: &std::path::Path,
    settings: &SharedFsWatcherSettings,
) -> DataspaceWatch {
    if let Err(err) = ds.autosync_on().await {
        error!("The default dataspace could not be put on autosync: {err}");
    }
    let tasks = crate::engine::tasks::dataspace::register(
        ds,
        data_dir.to_path_buf(),
        Arc::new(crate::engine::tasks::dataspace::log_task),
    );
    let (enabled, debounce_ms) = {
        let settings = settings.read().await;
        (settings.enabled, settings.debounce_ms)
    };
    let folders = if enabled {
        let debounce = std::time::Duration::from_millis(debounce_ms);
        crate::engine::dataspace::autosync::watch(Arc::clone(ds), debounce)
            .inspect_err(|err| error!("The dataspace watcher could not start: {err}"))
            .ok()
    } else {
        None
    };
    DataspaceWatch {
        _folders: folders,
        _tasks: tasks,
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

    /// An app folder whose ref engine has a home, as an older `QuiltSync`
    /// left it: `home/` beside it.
    async fn older_app() -> tempfile::TempDir {
        let dir = tempfile::tempdir().expect("temp dir");
        let data_dir = dir.path().join("app");
        quilt::LocalDomain::new(&data_dir)
            .set_home(dir.path().join("home"))
            .await
            .expect("a home");
        dir
    }

    async fn start(dir: &tempfile::TempDir) -> Arc<Services> {
        Services::init(dir.path().join("app"), Version::new(0, 0, 0))
            .await
            .expect("services")
    }

    /// The dataspace engine is on by default: the first start imports what
    /// the ref engine kept and says so, and the next imports nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_dataspace_engine_is_on_by_default_and_imports_once() {
        let dir = older_app().await;

        let first = start(&dir).await;
        let notice = first.cutover_notice.clone();
        let opened = first.dataspace.as_ref().map(|ds| ds.dataspace());
        drop(first);
        let second = start(&dir).await;

        assert!(notice.is_some_and(|notice| notice.starts_with("Imported")));
        assert_eq!(opened, Some(dir.path().join("home")));
        assert!(second.dataspace.is_some());
        assert_eq!(second.cutover_notice, None);
    }

    /// `"dataspace_opt_out": true` keeps the ref engine, and imports nothing.
    #[tokio::test(flavor = "multi_thread")]
    async fn the_opt_out_keeps_the_ref_engine() {
        let dir = older_app().await;
        let data_dir = dir.path().join("app");
        std::fs::write(
            data_dir.join("experimental_settings.json"),
            br#"{"dataspace_opt_out":true}"#,
        )
        .expect("settings");

        let s = start(&dir).await;

        assert!(s.dataspace.is_none());
        assert_eq!(s.cutover_notice, None);
        assert!(!data_dir.join(quilt::domain::DOMAIN_DIR_NAME).exists());
        assert_eq!(s.home_dir().await, Some(dir.path().join("home")));
    }

    /// An import that is refused falls back to the ref engine, and says why.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_refused_import_falls_back_to_the_ref_engine() {
        let dir = older_app().await;
        let domain = quilt::domain::DomainRoot::new(
            dir.path().join("app").join(quilt::domain::DOMAIN_DIR_NAME),
        );
        domain
            .set_autosync(
                &quilt::io::storage::LocalStorage::new(),
                &dir.path().join("elsewhere"),
                true,
            )
            .await
            .expect("a Domain in use");

        let s = start(&dir).await;

        assert!(s.dataspace.is_none());
        assert!(
            s.cutover_notice
                .as_deref()
                .is_some_and(|notice| notice.contains("ref path")),
            "{:?}",
            s.cutover_notice
        );
    }

    /// With the dataspace engine on, signing in goes through the app's own
    /// remote, which the dataspace engine shares: no domain is locked, and a
    /// logout reaches both engines.
    #[tokio::test(flavor = "multi_thread")]
    async fn auth_works_with_the_dataspace_engine_on() {
        let dir = older_app().await;
        let data_dir = dir.path().join("app");
        let auth = quilt::paths::DomainPaths::new(data_dir.clone()).auth_dir();
        std::fs::create_dir_all(auth.join("one.quilt.test")).expect("an account");

        let s = start(&dir).await;
        let ds = s.dataspace.as_ref().expect("the dataspace engine");
        let _held = s.model.get_quilt().lock().await;
        let rows = quilt::paths::list_auth_hosts(&data_dir);
        let erased = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            commands::erase_auth(&s, "one.quilt.test".to_string()),
        )
        .await
        .expect("a logout that waits on no domain");

        let shared = s.model.remote();
        assert!(std::ptr::eq(
            std::ptr::from_ref(&*shared),
            std::ptr::from_ref(ds.remote())
        ));
        assert_eq!(rows, ["one.quilt.test"]);
        assert!(erased.is_ok(), "{erased:?}");
        assert!(!auth.join("one.quilt.test").exists());
    }

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
