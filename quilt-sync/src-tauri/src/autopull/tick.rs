use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::time::Duration;
use std::time::Instant;
use std::time::SystemTime;

use quilt_uri::Host;
use quilt_uri::Namespace;

use crate::Error;
use crate::autopull::PausedReason;
use crate::autopull::WatcherInner;
use crate::autopull::reporter::PackageStatusEvent;
use crate::model;
use crate::model::QuiltModel;
use crate::publish_settings::PublishSettings;
use crate::quilt;
use crate::quilt::flow::PullOutcome;
use crate::telemetry::prelude::*;

#[derive(Debug)]
pub(crate) struct RefreshOutcome {
    pub upstream: quilt::lineage::UpstreamState,
    pub has_changes: bool,
    /// `Some(message)` only on the publish success path; `None` on
    /// pull success, on quiet-window deferral, and on any no-action
    /// tick. `run_once` reads this to call `report_published`.
    pub published: Option<String>,
}

#[derive(Debug)]
pub(crate) enum WatchError {
    Conflict(PausedReason),
    LoginRequired(Option<Host>),
    Transient(Error),
}

// String-matches the guard messages in `quilt-rs/src/flow/pull.rs` and
// `quilt-rs/src/flow/push.rs`. Open question in the plan: replace with
// typed `PullRefusal` / `PushRefusal` enums upstream.
//
// Policy:
// - Known pull-side refusals (`PackageOpError::Package`) keep their
//   specific `PausedReason`.
// - `Push` / `Commit` / `Publish` variants almost always reflect
//   user-actionable trouble (workflow rejected, hash mismatch, ...),
//   so we pause with `Other(_)` carrying the message.
// - HTTP / IO / S3 — including the AWS SDK `S3Error` family —
//   are `Transient` (retry with backoff). S3 is a peer variant of
//   `PackageOp` on `quilt::Error`, not nested inside it: `PutObject`,
//   `UploadFile`, throttling, 5xx, and the like all propagate as
//   `Error::S3(_)` straight through the publish flow. Treating them
//   as `Other(_)` would permanently pause the namespace on a single
//   network blip — exactly the wrong shape for autopush. Truly
//   permanent S3 sub-kinds (`NotFound`, `PermissionDenied`-like) are
//   either caught upstream (`Error::is_not_found` in `flow::push`) or
//   accepted as "retry every 64 s until the user fixes it" via the
//   capped backoff — annoying but not catastrophic, and far better
//   than silently pausing on every transient blip.
// - Everything else lands in `Other(_)` — the new default arm flips the
//   bias from "keep trying quietly" to "stop and surface."
pub(crate) fn classify_sync_err(err: Error) -> Result<(), WatchError> {
    match &err {
        Error::Quilt(quilt::Error::PackageOp(quilt::PackageOpError::Package(msg))) => {
            if msg == "package has pending changes" {
                Err(WatchError::Conflict(PausedReason::PendingChanges))
            } else if msg == "package has pending commits" {
                Err(WatchError::Conflict(PausedReason::PendingCommit))
            } else if msg == "package has diverged" {
                Err(WatchError::Conflict(PausedReason::Diverged))
            } else {
                Err(WatchError::Conflict(PausedReason::Other(msg.clone())))
            }
        }
        // A pull that raced another pull and found nothing to do. Benign:
        // the namespace is already at `latest`, so keep syncing quietly.
        Error::Quilt(quilt::Error::PackageOp(quilt::PackageOpError::AlreadyUpToDate)) => Ok(()),
        Error::Quilt(quilt::Error::PackageOp(
            quilt::PackageOpError::Push(msg)
            | quilt::PackageOpError::Commit(msg)
            | quilt::PackageOpError::Publish(msg),
        )) => Err(WatchError::Conflict(PausedReason::Other(msg.clone()))),
        // The engine's defensive refusal when a `pull` hits a conflict the
        // dry-run classifier didn't foresee (a race between classify and
        // apply). Map it to the same `PullConflict` pause the dry-run path
        // produces so both routes agree on the reason the UI renders.
        Error::Quilt(quilt::Error::PackageOp(quilt::PackageOpError::PullConflict(conflicts))) => {
            let files = conflicts.iter().map(|p| p.display().to_string()).collect();
            Err(WatchError::Conflict(PausedReason::PullConflict(files)))
        }
        // Workflow rejection (commit- or push-side): user-actionable, so
        // pause rather than retry. Bind the inner `WorkflowValidationError`
        // so the reason text is the validator's own message (which names the
        // failing rule and fields) without the `Quilt error:` wrapper prefix
        // `Error::Quilt`'s Display would add — cleaner in the tray/tooltip.
        // Kept as `PausedReason::Other` (not a dedicated variant): the UI
        // already renders the free-form message for `Other`, so a new variant
        // would add wire churn for no user-visible gain.
        Error::Quilt(quilt::Error::WorkflowValidation(inner)) => {
            Err(WatchError::Conflict(PausedReason::Other(inner.to_string())))
        }
        // A malformed `.quilt/workflows/config.yml` is a user-actionable
        // misconfiguration: pause (Conflict), never retry as a transient. Bind
        // the inner error so the tray/tooltip reason is the focused
        // "Invalid workflows config: …" message rather than the outer
        // "Quilt error:" wrapper the default arm's `err.to_string()` would add.
        Error::Quilt(quilt::Error::RemoteCatalog(
            inner @ quilt::RemoteCatalogError::InvalidWorkflowsConfig(_),
        )) => Err(WatchError::Conflict(PausedReason::Other(inner.to_string()))),
        Error::Quilt(quilt::Error::Reqwest(_) | quilt::Error::Io(_) | quilt::Error::S3(_)) => {
            Err(WatchError::Transient(err))
        }
        Error::Quilt(quilt::Error::Login(quilt::LoginError::Required(host))) => {
            Err(WatchError::LoginRequired(host.clone()))
        }
        _ => Err(WatchError::Conflict(PausedReason::Other(err.to_string()))),
    }
}

pub(crate) async fn refresh_then_maybe_sync(
    model: &impl QuiltModel,
    namespace: &Namespace,
    lineage: &quilt::lineage::PackageLineage,
    publish: &PublishSettings,
    quiet_window: Duration,
    pull_enabled: bool,
    push_enabled: bool,
) -> Result<RefreshOutcome, WatchError> {
    let installed = model
        .get_installed_package(namespace)
        .await
        .map_err(WatchError::Transient)?
        .ok_or_else(|| {
            WatchError::Transient(Error::from(quilt::InstallPackageError::NotInstalled(
                namespace.clone(),
            )))
        })?;

    // `status` does the cheap tag refresh; an expired token surfaces here.
    let status = model
        .get_installed_package_status(&installed, None)
        .await
        .map_err(|err| match &err {
            Error::Quilt(quilt::Error::Login(quilt::LoginError::Required(host))) => {
                WatchError::LoginRequired(host.clone())
            }
            _ => WatchError::Transient(err),
        })?;
    let upstream = status.upstream_state;
    let has_changes = !status.changes.is_empty();
    // `lineage` comes from `run_once`'s skip-filter read — re-reading it
    // here would cost an extra trait call per tick and open a narrow
    // race window where a commit landing between the two reads would
    // make `has_pending_commit` stale relative to `upstream`.
    let has_pending_commit = lineage.commit.is_some();

    // A `Diverged` state needs explicit user action (Certify Latest or
    // Reset Local). Neither the pull nor the publish branch would touch
    // it, but leaving it as an `Ok(_)` outcome means we'd re-emit
    // `diverged` every tick without pausing — the UI then looks healthy
    // even though no progress can be made. Surface as a Conflict so the
    // namespace lands in the paused set on the first observation.
    if upstream == quilt::lineage::UpstreamState::Diverged {
        return Err(WatchError::Conflict(PausedReason::Diverged));
    }

    // Pull branch — route on the `PullOutcome`, not clean-vs-dirty. A
    // `Behind` tree with non-conflicting local work now pulls (preserving
    // it) rather than falling through to the publish arm and diverging; a
    // real conflict pauses. Pull runs before publish so additive local work
    // reconciles to `latest` rather than committing into divergence.
    //
    // The `!has_pending_commit` clause is **defensive**, not load-bearing:
    // under the new `From<PackageLineage> for UpstreamState`, a package
    // with a pending commit and a stale `latest_hash` lands in `Diverged`,
    // not `Behind`, so this code path is unreachable in practice. We keep
    // the clause to make mutual exclusivity with the publish branch
    // explicit at the call site — if the `From` conversion ever changes
    // shape, this gate is what stops a pull and a publish from racing on
    // the same package in the same tick.
    if pull_enabled && upstream == quilt::lineage::UpstreamState::Behind && !has_pending_commit {
        // TODO: this dry-run and the `package_pull` below each build their own
        // snapshot (one tag resolution + one status walk apiece — the per-call
        // cost is now a single tag read), so `classify_pull` still runs twice
        // per Behind tick with a race window between them. Have `flow::pull`
        // return the `PullOutcome` it already computes and route on the pull
        // result alone — same outcome-based routing, half the work.
        //
        // TODO: the blanket `Transient` mapping below bypasses the
        // `LoginRequired` classification the status call gets, so an expired
        // session surfaces the login affordance one backoff (~2 s) later than
        // it could. Route this error through the same classification.
        let outcome = model
            .package_pull_outcome(&installed)
            .await
            .map_err(WatchError::Transient)?;
        match outcome {
            PullOutcome::Blocked { conflicts } => {
                let files = conflicts.iter().map(|p| p.display().to_string()).collect();
                return Err(WatchError::Conflict(PausedReason::PullConflict(files)));
            }
            PullOutcome::CleanUpdate | PullOutcome::KeepsLocalChanges { .. } => {
                return match model.package_pull(&installed, None).await {
                    Ok(_) => {
                        info!("autosync: pulled namespace={namespace}");
                        // Kept work leaves a dirty tree: `UpToDate` +
                        // `has_changes` is the intended post-pull state, so
                        // carry `has_changes` through rather than forcing it
                        // to `false`.
                        // TODO: when the pull trivially resolved every local
                        // change, the pre-pull `has_changes` is stale-true and
                        // the UI counts phantom pending changes for one tick
                        // interval. Recompute (or refresh) after the pull.
                        Ok(RefreshOutcome {
                            upstream: quilt::lineage::UpstreamState::UpToDate,
                            has_changes,
                            published: None,
                        })
                    }
                    Err(err) => classify_sync_err(err).map(|()| RefreshOutcome {
                        upstream,
                        has_changes,
                        published: None,
                    }),
                };
            }
            // Race: tip moved back to up-to-date between status and classify.
            PullOutcome::UpToDate => {}
        }
    }

    // Publish branch.
    let publish_eligible = push_enabled
        && matches!(
            upstream,
            quilt::lineage::UpstreamState::UpToDate
                | quilt::lineage::UpstreamState::Ahead
                | quilt::lineage::UpstreamState::Local,
        )
        && (has_changes || has_pending_commit);
    if publish_eligible {
        let now = SystemTime::now();
        if !status.working_tree_quiet(now, quiet_window) {
            info!("autosync: namespace={namespace} working tree not quiet, deferring");
            return Ok(RefreshOutcome {
                upstream,
                has_changes,
                published: None,
            });
        }
        // `publish_with_settings` is shared with the manual one-click
        // Publish command in `commands.rs`, so a change to publish
        // settings (new placeholder, new field) applies identically
        // regardless of who triggered the publish.
        return match model::publish_with_settings(model, namespace, publish, status).await {
            Ok((_, message)) => {
                info!("autosync: published namespace={namespace}");
                Ok(RefreshOutcome {
                    upstream: quilt::lineage::UpstreamState::UpToDate,
                    has_changes: false,
                    published: Some(message),
                })
            }
            Err(err) => classify_sync_err(err).map(|()| RefreshOutcome {
                upstream,
                has_changes,
                published: None,
            }),
        };
    }

    Ok(RefreshOutcome {
        upstream,
        has_changes,
        published: None,
    })
}

#[derive(Debug, Clone, Copy)]
pub(crate) struct BackoffState {
    pub next_attempt: Instant,
    pub consecutive_failures: u32,
}

// 2, 4, 8, 16, 32, 64 s, then capped.
pub(crate) fn backoff_duration(failures: u32) -> Duration {
    let exp = failures.min(6);
    Duration::from_secs(1u64 << exp)
}

fn is_backoff_due(
    backoff: &BTreeMap<Namespace, BackoffState>,
    namespace: &Namespace,
    now: Instant,
) -> bool {
    backoff.get(namespace).is_none_or(|b| now >= b.next_attempt)
}

fn bump_backoff(
    backoff: &mut BTreeMap<Namespace, BackoffState>,
    namespace: &Namespace,
    now: Instant,
) {
    let entry = backoff.entry(namespace.clone()).or_insert(BackoffState {
        next_attempt: now,
        consecutive_failures: 0,
    });
    entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
    entry.next_attempt = now + backoff_duration(entry.consecutive_failures);
}

#[allow(clippy::too_many_lines, reason = "cohesive autosync tick sequence")]
pub(crate) async fn run_once(model: &impl QuiltModel, inner: &WatcherInner) -> Result<(), Error> {
    // Cheap pre-check: if both directions are off we have nothing to
    // do. Per-direction gating lives inside `refresh_then_maybe_sync`
    // so a single-direction config (pull only / push only) still
    // exercises the cheap status refresh and the skip rules.
    let (pull_enabled, push_enabled) = {
        let settings = inner.settings.read().await;
        (settings.pull.enabled, settings.push.enabled)
    };
    if !pull_enabled && !push_enabled {
        return Ok(());
    }

    let packages = model.get_installed_packages_list().await?;
    let current: BTreeSet<Namespace> = packages.iter().map(|p| p.namespace.clone()).collect();
    inner
        .paused
        .write()
        .await
        .retain(|ns, _| current.contains(ns));
    inner
        .backoff
        .write()
        .await
        .retain(|ns, _| current.contains(ns));
    inner
        .login_blocked
        .write()
        .await
        .retain(|ns, _| current.contains(ns));
    inner.aggregator.retain_namespaces(&current);

    // Snapshot publish settings once per tick so we don't reacquire the
    // RwLock per package. Same lifetime for `quiet_window`.
    //
    // `quiet_window` is the constant `push.idle_timeout_secs`. It does
    // not depend on window mode anymore — that coupling was the bug.
    // The sleep loop in `Watcher::spawn` still reads
    // `cadence_for_mode(&settings.pull, mode)`, so pull frequency and
    // push quiet window can be tuned independently.
    let publish = inner.publish_settings.read().await.clone();
    let quiet_window = {
        let settings = inner.settings.read().await;
        Duration::from_secs(settings.push.idle_timeout_secs)
    };

    let now = Instant::now();
    for pkg in packages {
        let namespace = pkg.namespace.clone();

        // Skip Local / misconfigured packages without a network round-trip.
        let lineage = match model.get_installed_package_lineage(&pkg).await {
            Ok(l) => l,
            Err(err) => {
                warn!("autosync: lineage read failed for {namespace}: {err}");
                continue;
            }
        };
        let Some(remote) = lineage.remote_uri.as_ref() else {
            continue;
        };
        if remote.origin.is_none() || remote.bucket.is_empty() {
            continue;
        }

        if inner.paused.read().await.contains_key(&namespace) {
            continue;
        }
        if !is_backoff_due(&*inner.backoff.read().await, &namespace, now) {
            continue;
        }

        match refresh_then_maybe_sync(
            model,
            &namespace,
            &lineage,
            &publish,
            quiet_window,
            pull_enabled,
            push_enabled,
        )
        .await
        {
            Ok(outcome) => {
                inner.backoff.write().await.remove(&namespace);
                inner.login_blocked.write().await.remove(&namespace);
                if let Some(message) = outcome.published.as_deref() {
                    inner.reporter.report_published(&namespace, message);
                }
                inner.reporter.report_status(
                    &namespace,
                    PackageStatusEvent {
                        namespace: namespace.to_string(),
                        status: outcome.upstream.to_string(),
                        has_changes: outcome.has_changes,
                    },
                );
                inner.aggregator.clear_error(&namespace);
                inner
                    .aggregator
                    .note_status(&namespace, outcome.has_changes);
            }
            Err(WatchError::LoginRequired(host)) => {
                // Backoff until the user re-auths; the Ok arm clears it.
                bump_backoff(&mut *inner.backoff.write().await, &namespace, now);
                inner
                    .login_blocked
                    .write()
                    .await
                    .insert(namespace.clone(), host.clone());
                inner.reporter.report_login_required(host.as_ref());
                inner.aggregator.note_login_required(&namespace, host);
            }
            Err(WatchError::Conflict(reason)) => {
                inner
                    .paused
                    .write()
                    .await
                    .insert(namespace.clone(), reason.clone());
                inner.reporter.report_paused(&namespace, reason.clone());
                // Heuristic status from the refusal reason — flow::pull /
                // flow::publish don't expose the post-attempt state
                // directly. The string `"error"` is **reserved** for "we
                // couldn't talk to the remote" — the UI renders a Login
                // affordance on that one. Surface autosync refusals as
                // `"paused"` so the row banner is neutral, and let the UI
                // pull the message out of the `autosync-paused` event the
                // reporter emits.
                let (status, has_changes) = match reason {
                    PausedReason::PendingChanges => ("behind", true),
                    PausedReason::PendingCommit => ("ahead", false),
                    PausedReason::Diverged => ("diverged", false),
                    PausedReason::PullConflict(ref files) => {
                        warn!("autosync: paused namespace={namespace} pull conflict={files:?}");
                        ("paused", true)
                    }
                    PausedReason::Other(ref msg) => {
                        warn!("autosync: paused namespace={namespace} error={msg}");
                        ("paused", false)
                    }
                };
                inner.reporter.report_status(
                    &namespace,
                    PackageStatusEvent {
                        namespace: namespace.to_string(),
                        status: status.to_string(),
                        has_changes,
                    },
                );
                let aggregator_message = match &reason {
                    PausedReason::PendingChanges => "pending changes",
                    PausedReason::PendingCommit => "pending commits",
                    PausedReason::Diverged => "diverged",
                    PausedReason::PullConflict(_) => "pull conflict",
                    PausedReason::Other(msg) => msg.as_str(),
                };
                inner.aggregator.note_paused(&namespace, aggregator_message);
                inner.aggregator.note_status(&namespace, has_changes);
            }
            Err(WatchError::Transient(err)) => {
                bump_backoff(&mut *inner.backoff.write().await, &namespace, now);
                warn!("autosync: transient error for {namespace}: {err}");
                // Transient: don't touch the aggregator's error map — a
                // network blip should not flip the tray to Error. The
                // next tick either clears or escalates.
            }
        }
    }

    Ok(())
}

#[cfg(test)]
mod tests;
