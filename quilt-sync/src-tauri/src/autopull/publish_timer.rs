//! Publishes each package at its own deadline, apart from the check.
//!
//! A deadline is the package's last-change time plus the wait, read at each
//! wake. Whatever can make one earlier calls
//! [`WatcherInner::reschedule_publish`]; a deadline that moved later is found
//! when the timer wakes, and it sleeps again.

use std::collections::BTreeMap;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

use quilt_uri::Namespace;

use crate::autopull::WatcherInner;
use crate::autopull::publish_deadline;
use crate::autopull::tick::Pass;
use crate::autopull::tick::PassSettings;
use crate::autopull::tick::sync_package;
use crate::commands::RoleCache;
use crate::model::QuiltModel;
use crate::telemetry::prelude::*;

/// How soon a package another writer holds is tried again.
const BUSY_RETRY: Duration = Duration::from_secs(5);

/// The longest sleep. `Instant` stops while the computer sleeps, and deadlines
/// are wall-clock times.
const MAX_SLEEP: Duration = Duration::from_mins(1);

/// The timer's loop: publish what is due, then sleep until the next deadline
/// or a reschedule.
pub(crate) async fn run(model: &impl QuiltModel, roles: &RoleCache, inner: &WatcherInner) {
    let mut held = BTreeMap::new();
    loop {
        let next = Box::pin(publish_due(model, roles, inner, &mut held)).await;
        let woken = inner.clocks.publish_timer.notified();
        match next {
            Some(at) => {
                let at = at.min(Instant::now() + MAX_SLEEP);
                tokio::select! {
                    () = tokio::time::sleep_until(at.into()) => {}
                    () = woken => {}
                }
            }
            None => woken.await,
        }
    }
}

/// Run the pass over every package whose deadline has come, and return when
/// the next one is due. `None` when nothing counts down or publishing is off.
///
/// `held` keeps the packages another writer held, until their retry.
pub(crate) async fn publish_due(
    model: &impl QuiltModel,
    roles: &RoleCache,
    inner: &WatcherInner,
    held: &mut BTreeMap<Namespace, Instant>,
) -> Option<Instant> {
    // Before the settings and the deadlines are read: a check can change
    // either while it runs.
    let _passes = inner.clocks.passes.lock().await;
    let settings = inner.settings.read().await.clone();
    if !settings.push.enabled {
        return None;
    }
    let run = PassSettings {
        publish: inner.publish_settings.read().await.clone(),
        wait: Duration::from_secs(settings.push.idle_timeout_secs),
        pull_enabled: settings.pull.enabled,
        push_enabled: true,
    };
    let now = Instant::now();
    let wall = SystemTime::now();
    held.retain(|_, until| *until > now);

    let mut due = Vec::new();
    for namespace in waiting(inner).await {
        if due_at(inner, &namespace, run.wait, held, (now, wall))
            .await
            .is_some_and(|at| at <= now)
        {
            due.push(namespace);
        }
    }
    if !due.is_empty() {
        inner.aggregator.note_tick_started();
        run_passes(model, roles, inner, &run, due, held).await;
        inner.aggregator.note_tick_ended_ok();
    }

    let clock = (Instant::now(), SystemTime::now());
    let mut next: Option<Instant> = None;
    for namespace in waiting(inner).await {
        if let Some(at) = due_at(inner, &namespace, run.wait, held, clock).await {
            next = Some(next.map_or(at, |cur| cur.min(at)));
        }
    }
    next
}

async fn run_passes(
    model: &impl QuiltModel,
    roles: &RoleCache,
    inner: &WatcherInner,
    run: &PassSettings,
    due: Vec<Namespace>,
    held: &mut BTreeMap<Namespace, Instant>,
) {
    for namespace in due {
        let pass = match model.get_installed_package(&namespace).await {
            Ok(Some(pkg)) => sync_package(model, roles, inner, &pkg, run, Instant::now()).await,
            Ok(None) => Pass::Skipped,
            Err(err) => {
                warn!("autosync: publish timer cannot read {namespace}: {err}");
                Pass::Skipped
            }
        };
        match pass {
            Pass::Busy => {
                held.insert(namespace, Instant::now() + BUSY_RETRY);
            }
            Pass::Skipped => {
                inner
                    .clocks
                    .publish_waiting
                    .write()
                    .await
                    .remove(&namespace);
                inner.reporter.report_autosync_changed();
            }
            Pass::Done => {}
        }
    }
}

async fn waiting(inner: &WatcherInner) -> Vec<Namespace> {
    inner
        .clocks
        .publish_waiting
        .read()
        .await
        .iter()
        .cloned()
        .collect()
}

/// When the timer may run `namespace`: its deadline, and no earlier than its
/// backoff or its retry after a busy lock. `None` for a paused package and one
/// with no last-change time. `clock` is one reading of both clocks.
async fn due_at(
    inner: &WatcherInner,
    namespace: &Namespace,
    wait: Duration,
    held: &BTreeMap<Namespace, Instant>,
    (now, wall): (Instant, SystemTime),
) -> Option<Instant> {
    if inner.paused.read().await.contains_key(namespace) {
        return None;
    }
    let deadline = publish_deadline(inner.last_change.get(namespace).await, wait)?;
    let mut at = now + deadline.duration_since(wall).unwrap_or_default();
    if let Some(backoff) = inner.backoff.read().await.get(namespace) {
        at = at.max(backoff.next_attempt);
    }
    if let Some(until) = held.get(namespace) {
        at = at.max(*until);
    }
    Some(at)
}

#[cfg(test)]
mod tests;
