use std::sync::Arc;
use std::time::Duration;

use tokio::sync::mpsc;

use crate::engine::autopull::StatusReporter;
use crate::engine::model::Model;
use crate::engine::telemetry::prelude::*;

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

/// Spawn the reactor and seed it with the current installed-packages list.
/// The reactor then polls the model on a fixed interval (see
/// `RECONCILE_INTERVAL` in `reactor.rs`) to pick up installs / uninstalls
/// without a dedicated mutation channel — accepted-cost is a few-second
/// delay before a freshly-installed package starts watching.
pub async fn spawn(
    model: Arc<Model>,
    watcher: Arc<crate::engine::autopull::WatcherInner>,
    settings: SharedFsWatcherSettings,
    reporter: &Arc<dyn StatusReporter>,
) {
    let (signal_tx, signal_rx) = mpsc::unbounded_channel::<MappingSignal>();

    // Use `try_read` so a poisoned/contended lock on settings doesn't
    // block startup; fall back to the type default.
    let (debounce_ms, enabled) = settings.try_read().map_or_else(
        |_| {
            let default = FsWatcherSettings::default();
            (default.debounce_ms, default.enabled)
        },
        |s| (s.debounce_ms, s.enabled),
    );
    let subscription =
        match Subscription::new(Duration::from_millis(debounce_ms), signal_tx, reporter) {
            Ok(sub) => sub,
            Err(err) => {
                // We can't build a debouncer — most likely a fatal platform
                // issue. Report and return; the reactor never starts. With
                // watching off, nothing the reader turned on has failed.
                if enabled {
                    emit_subscriber_error(reporter.as_ref(), &err);
                } else {
                    warn!("fswatcher: cannot start, and watching is off: {err}");
                }
                return;
            }
        };

    let mut state = ReactorState {
        settings,
        reporter: reporter.clone(),
        signal_rx,
        subscription,
        trailing: reactor::TrailingWalks::default(),
        local_volume: std::collections::BTreeMap::new(),
        last_reconcile_error_kind: None,
    };

    // Initial reconcile up front so existing packages are watched
    // immediately, instead of waiting for the first periodic tick.
    let initial = snapshot_mappings(&*model).await;
    if let Some(initial) = initial {
        reactor::note_volumes(&mut state, &initial.packages);
        if let Err(err) = state
            .subscription
            .reconcile(initial.packages, initial.user_rules)
        {
            emit_subscriber_error(reporter.as_ref(), &err);
        }
    }

    crate::engine::events::spawn(async move {
        run_reactor(state, model, watcher).await;
    });

    info!("fswatcher: spawned (debounce={debounce_ms}ms)");
}
