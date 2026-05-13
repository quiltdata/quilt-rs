use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use tauri::Manager;
use tokio::sync::mpsc;

use crate::autopull::StatusReporter;
use crate::model::Model;
use crate::telemetry::prelude::*;

pub mod filter;
pub mod reactor;
pub mod settings;
pub mod subscriber;

pub use settings::FsWatcherSettings;
pub use settings::SharedFsWatcherSettings;
pub use settings::init as init_settings;

use reactor::ReactorState;
use reactor::emit_subscriber_error;
use reactor::run as run_reactor;
use reactor::snapshot_mappings;
use subscriber::MappingSignal;
use subscriber::Subscription;

const SIGNAL_CHANNEL_CAPACITY: usize = 128;

/// Spawn the reactor and seed it with the current installed-packages list.
/// The reactor then polls the model on a fixed interval (see
/// `RECONCILE_INTERVAL` in `reactor.rs`) to pick up installs / uninstalls
/// without a dedicated mutation channel — accepted-cost is a few-second
/// delay before a freshly-installed package starts watching.
pub fn spawn(
    app_handle: &tauri::AppHandle,
    settings: SharedFsWatcherSettings,
    reporter: &Arc<dyn StatusReporter>,
) {
    let (signal_tx, signal_rx) = mpsc::channel::<MappingSignal>(SIGNAL_CHANNEL_CAPACITY);

    // Use `try_read` so a poisoned/contended lock on settings doesn't
    // block startup; fall back to the type default.
    let debounce_ms = settings.try_read().map_or_else(
        |_| FsWatcherSettings::default().debounce_ms,
        |s| s.debounce_ms,
    );
    let subscription =
        match Subscription::new(Duration::from_millis(debounce_ms), signal_tx, reporter) {
            Ok(sub) => sub,
            Err(err) => {
                // We can't build a debouncer — most likely a fatal platform
                // issue. Report and return; the reactor never starts.
                emit_subscriber_error(reporter.as_ref(), &err);
                return;
            }
        };

    let mut state = ReactorState {
        settings,
        reporter: reporter.clone(),
        signal_rx,
        subscription,
        previous_fingerprints: BTreeMap::new(),
    };

    // Initial reconcile up front so existing packages are watched
    // immediately, instead of waiting for the first periodic tick.
    let initial = {
        let model = app_handle.state::<Model>();
        let model_ref: &Model = &model;
        tauri::async_runtime::block_on(async { snapshot_mappings(model_ref).await })
    };
    if let Some(initial) = initial
        && let Err(err) = state.subscription.reconcile(initial)
    {
        emit_subscriber_error(reporter.as_ref(), &err);
    }

    let task_handle = app_handle.clone();
    tauri::async_runtime::spawn(async move {
        run_reactor(state, task_handle).await;
    });

    info!("fswatcher: spawned (debounce={debounce_ms}ms)");
}
