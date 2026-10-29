use std::fmt::Write;
use std::sync::Arc;

use serde::Serialize;

use quilt_uri::Host;
use quilt_uri::Namespace;
use tauri::{Emitter, Manager};

use crate::engine::autopull::PausedReason;
use crate::engine::autopull::pull_toast::ReportToast;
use crate::engine::quilt;
use crate::engine::telemetry::Telemetry;
use crate::engine::telemetry::event::{
    AutosyncAuthEvent, AutosyncEvent, AutosyncPausedEvent, MixpanelEvent, PausedKind,
};
use crate::engine::telemetry::prelude::*;
use crate::engine::toast::ToastCenter;

/// Whether a deployment's session has *just* become unusable, or was already.
///
/// The distinction exists for telemetry: the loop rediscovers an expired session
/// on every backoff-due tick, for every package on that deployment, so reporting
/// each discovery would count one expiry many times over. The UI wants the
/// opposite — it re-renders the same affordance idempotently and does not care —
/// so this informs telemetry without changing when anyone is told.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LoginBlock {
    /// No package on this deployment was blocked before. The episode starts here.
    Began,
    /// Another package on the same deployment is already blocked, or this one was.
    Continues,
}

/// Event names. Kept in lockstep with the UI's `listen(...)` calls.
pub const STATUS_EVENT: &str = "package-status-changed";
pub const LOGIN_REQUIRED_EVENT: &str = "autosync-login-required";
pub const SUBSCRIBER_ERROR_EVENT: &str = "fswatcher-subscriber-error";
pub const PUBLISHED_EVENT: &str = "autosync-published";
pub const PAUSED_EVENT: &str = "autosync-paused";
/// Tells the window a package's autosync state may have changed, so it reads
/// the main page's watcher payload again. No payload.
pub const AUTOSYNC_CHANGED_EVENT: &str = "autosync-changed";

/// Payload emitted to the UI when a package's upstream state changes after
/// a watcher tick. Mirrors the camelCase shape of `RefreshedPackageStatus`
/// so the UI can reuse its existing per-package signal.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackageStatusEvent {
    pub namespace: quilt_uri::Namespace,
    pub status: String,
    pub has_changes: bool,
    /// Digest of the observation this event reports — the upstream state,
    /// the newer revision it was computed against, per changed path its kind
    /// and content hash, and per ignored path the rule that ignored it. Two events with
    /// equal fingerprints describe the same working tree and remote, so a
    /// consumer that already acted on one can ignore the other. Assembled
    /// here with `status`/`has_changes` from a single `status` so the three
    /// cannot disagree.
    pub fingerprint: String,
}

impl PackageStatusEvent {
    /// The single assembler: `status`, `has_changes`, and `fingerprint` all
    /// come from one `InstalledPackageStatus` observation.
    pub fn from_status(
        namespace: &Namespace,
        status: &quilt::lineage::InstalledPackageStatus,
    ) -> Self {
        Self {
            namespace: namespace.clone(),
            status: status.upstream_state.to_string(),
            has_changes: !status.changes.is_empty(),
            fingerprint: status_fingerprint(status),
        }
    }
}

/// Stable digest of the parts of `InstalledPackageStatus` a consumer renders.
/// Two recompute results with the same fingerprint produce the same view, so
/// the second need not be acted on.
///
/// Format: `<upstream>;<hex(latest)>;<changes>|<ignored>`, where `<changes>`
/// is `<hex(path)>:<kind>:<hash>;` per changed path and `<ignored>` is
/// `file:<hex(path)>:<source>:<hex(pattern)>:<size>;` per ignored file, then
/// `dir:<hex(path)>:<source>:<hex(pattern)>;` per ignored folder. `<source>`
/// is `default`, `user:<hex(rules path)>:<line>` or `package:<line>`.
/// `latest` is the newer revision the status was computed against, so a
/// remote that moves from one newer revision to the next, leaving the package
/// behind with the same changes, is a new observation. The ignored rows are
/// the page's Ignored facet, so a new rule for an ignored file, or a newly
/// ignored file, is a new observation too.
/// Changed paths are walked in `BTreeMap` order (sorted by `PathBuf`); ignored
/// rows arrive in walk order and are sorted by path here, so the output is
/// deterministic. A path can hold arbitrary bytes
/// — non-UTF-8 names on Linux, or the `:`/`;`/`|` delimiters (kind is A/M/D, the
/// hash is hex/base64) — so it is **hex-encoded from its lossless bytes**, and
/// so are `latest`, the patterns and the user rules path, which can hold
/// delimiters too. Hex is
/// `0-9a-f` only, so it is both lossless and delimiter-free, keeping the
/// digest **injective**: different inputs give different digests, so a
/// consumer never skips a real refetch, and the same status always gives the
/// same digest, so a re-report stays inert. That holds for the digest, not for
/// every event: the announced pull-conflict clear prefixes it with `unpaused;`
/// on purpose, so its event differs from a plain report of the same status. (A lossy path string would fold two
/// distinct non-UTF-8 names to the same `�`; a raw path could serialize a
/// crafted `a:M:<hash>;b` like two entries.)
pub(crate) fn status_fingerprint(status: &quilt::lineage::InstalledPackageStatus) -> String {
    let mut out = String::new();
    let _ = write!(out, "{};", status.upstream_state);
    push_hex(&mut out, status.latest_hash.as_bytes());
    out.push(';');
    out.push_str(&changes_fingerprint(&status.changes));
    out.push('|');
    out.push_str(&ignored_fingerprint(status));
    out
}

/// Per changed path, its kind and content hash: what the working tree holds,
/// apart from the remote. Ignored rows stay out: they are never published, so
/// they must not move the last-change time that paces autopublish.
pub(crate) fn changes_fingerprint(changes: &quilt::lineage::ChangeSet) -> String {
    let mut out = String::new();
    for (path, change) in changes {
        let (kind, row) = match change {
            quilt::lineage::Change::Added(r) => ("A", r),
            quilt::lineage::Change::Modified(r) => ("M", r),
            quilt::lineage::Change::Removed(r) => ("D", r),
        };
        push_hex(&mut out, path.as_os_str().as_encoded_bytes());
        let _ = write!(out, ":{kind}:{};", row.hash);
    }
    out
}

/// The `<ignored>` part of [`status_fingerprint`]: per ignored file and
/// folder, the rule that ignored it, and the size of each ignored file.
pub(crate) fn ignored_fingerprint(status: &quilt::lineage::InstalledPackageStatus) -> String {
    let mut out = String::new();
    let mut files: Vec<_> = status.ignored_files.iter().collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, ignored_by, size) in files {
        out.push_str("file:");
        push_ignored_by(&mut out, path, ignored_by);
        let _ = write!(out, ":{size};");
    }
    let mut dirs: Vec<_> = status.ignored_dirs.iter().collect();
    dirs.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, ignored_by) in dirs {
        out.push_str("dir:");
        push_ignored_by(&mut out, path, ignored_by);
        out.push(';');
    }
    out
}

/// `<hex(path)>:<source>:<hex(pattern)>`
fn push_ignored_by(
    out: &mut String,
    path: &std::path::Path,
    ignored_by: &quilt::quiltignore::IgnoredBy,
) {
    push_hex(out, path.as_os_str().as_encoded_bytes());
    out.push(':');
    match &ignored_by.source {
        quilt::quiltignore::RuleSource::Default => out.push_str("default"),
        quilt::quiltignore::RuleSource::User { path, line } => {
            out.push_str("user:");
            push_hex(out, path.as_os_str().as_encoded_bytes());
            let _ = write!(out, ":{line}");
        }
        quilt::quiltignore::RuleSource::Package { line } => {
            let _ = write!(out, "package:{line}");
        }
    }
    out.push(':');
    push_hex(out, ignored_by.pattern.as_bytes());
}

fn push_hex(out: &mut String, bytes: &[u8]) {
    for b in bytes {
        let _ = write!(out, "{b:02x}");
    }
}

/// Fingerprint of a clean, up-to-date working tree whose `latest` is
/// `latest` and whose ignored rows digest to `ignored` — the state a package
/// reaches right after a successful publish, and after a pull that kept no
/// local work, with the revision it reached as `latest`. The mutation paths in
/// the tick don't hold a post-mutation `InstalledPackageStatus` to fingerprint
/// directly, but the observation they represent is a settled `UpToDate` tree,
/// and this names it, so the next tick's observation of that tree matches it.
/// Neither a publish nor a pull touches an ignored file, so `ignored` is the
/// [`ignored_fingerprint`] of the status read before the mutation. (A pull
/// that *kept* local changes reads this too; it understates by one
/// changed-path digest, which the next tick's real observation corrects — a
/// redundant refetch at worst, never a missed one. So does an empty `latest`,
/// for a publish that did not become `latest`, and a pull that brings a
/// `.quiltignore` changing which rule ignores a file. Only the open page reads
/// this digest, never the autosync decisions.)
pub(crate) fn settled_fingerprint(latest: &str, ignored: &str) -> String {
    let mut out = status_fingerprint(&quilt::lineage::InstalledPackageStatus {
        latest_hash: latest.to_string(),
        ..quilt::lineage::InstalledPackageStatus::new(
            quilt::lineage::UpstreamState::UpToDate,
            std::collections::BTreeMap::new(),
        )
    });
    out.push_str(ignored);
    out
}

/// Payload emitted to the UI after autosync publishes a package. The UI
/// renders this as the same toast surface manual Commit & Push uses.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishedEvent {
    pub namespace: quilt_uri::Namespace,
    pub message: String,
}

/// Payload emitted to the UI when autosync pauses a namespace. Carries
/// both a stable `reason` discriminant (so the UI can branch by category)
/// and an optional free-form `message` populated for
/// [`PausedReason::Other`] — workflow validation failures, hash mismatches,
/// JSON parse errors, etc.
///
/// This is distinct from the `package-status-changed` event because the
/// status string alone cannot disambiguate "remote unreachable" (which
/// surfaces as `status = "error"` and a Login affordance in the UI) from
/// "remote refused this push" (which is `status = "paused"` and renders a
/// neutral banner showing `message`).
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PausedEvent {
    pub namespace: quilt_uri::Namespace,
    /// Stable category: `"pendingChanges"`, `"pendingCommit"`,
    /// `"diverged"`, `"pullConflict"`, `"roleDenied"`, or `"other"`. Kept as
    /// a string so the wire format is independent of the Rust enum's variant
    /// layout.
    pub reason: String,
    /// Free-form description: the raw refusal reason for `reason = "other"`,
    /// the comma-joined conflicting file names for `reason =
    /// "pullConflict"`, or the active role's name for `reason =
    /// "roleDenied"` (absent when the name could not be resolved).
    pub message: Option<String>,
}

impl PausedEvent {
    /// `message` carries only the raw refusal reason. The "resolve, then
    /// push manually to resume" guidance is presentation and lives in the
    /// UI (it once was appended here, which mixed data with guidance and
    /// made the banner unreadable).
    pub fn from_reason(namespace: &Namespace, reason: &PausedReason) -> Self {
        let (reason_str, message) = match reason {
            PausedReason::PendingChanges => ("pendingChanges", None),
            PausedReason::PendingCommit => ("pendingCommit", None),
            PausedReason::Diverged => ("diverged", None),
            PausedReason::PullConflict(files) => ("pullConflict", Some(files.join(", "))),
            // The message is the role *name*, not a sentence: the wording
            // ("switch role to resume") is presentation and lives in the UI,
            // the same split `pullConflict` uses for its file list. An
            // unresolved name sends no message, so the UI falls back to the
            // role-less phrasing.
            PausedReason::RoleDenied { role } => {
                ("roleDenied", (!role.is_empty()).then(|| role.clone()))
            }
            PausedReason::Other(msg) => ("other", Some(msg.clone())),
        };
        Self {
            namespace: namespace.clone(),
            reason: reason_str.to_string(),
            message,
        }
    }
}

/// Point-in-time view of the autosync watcher's per-namespace state.
///
/// Returned by the `get_autosync_snapshot` Tauri command so the UI can
/// re-hydrate per-page banners on navigation — listening for the
/// `autosync-paused` event only catches pauses that fire while the page
/// is mounted, but the watcher's `paused` map persists across page
/// loads. Each entry in `paused` has the same wire shape as the
/// `autosync-paused` event payload.
#[derive(Serialize, Clone, Debug, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub struct WatcherSnapshot {
    pub paused: Vec<PausedEvent>,
}

/// Payload emitted when the filesystem watcher hits an OS-level error
/// the user might want to react to (e.g. the inotify limit).
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct SubscriberErrorEvent {
    pub kind: String,
    pub message: String,
    pub namespace: Option<String>,
}

/// Backend → frontend / log surface for watcher results.
///
/// The trait keeps the watcher portable: production wires a Tauri emitter,
/// tests and a hypothetical headless daemon wire a logger.
pub trait StatusReporter: Send + Sync + 'static {
    fn report_status(&self, namespace: &Namespace, event: PackageStatusEvent);
    /// `host` is the package's catalog, `None` for a package with a bucket and
    /// no catalog.
    fn report_paused(&self, namespace: &Namespace, host: Option<&Host>, reason: PausedReason);
    /// `block` distinguishes the deployment's session *becoming* unusable from
    /// the loop retrying while it stays that way. Only the transition is worth
    /// counting; the retries are a log line.
    fn report_login_required(&self, host: Option<&Host>, block: LoginBlock);
    fn report_subscriber_error(&self, event: SubscriberErrorEvent) {
        warn!(
            "fswatcher: kind={} namespace={:?} message={}",
            event.kind, event.namespace, event.message
        );
    }
    /// Surface a successful autosync publish. Default implementation
    /// logs only; `TauriEventReporter` also emits `PUBLISHED_EVENT`.
    fn report_published(&self, namespace: &Namespace, host: Option<&Host>, message: &str) {
        let host = log_host(host);
        info!("autosync: published namespace={namespace} host={host} message={message}");
    }

    /// Surface what a pull brought — the counterpart of
    /// [`Self::report_published`] for the other direction, and the one the
    /// watcher had no way to make: a pull's only outward sign was a status
    /// string.
    ///
    /// **Deliberately without a default.** [`TelemetryReporter`] decorates
    /// another reporter and forwards each call explicitly, so a defaulted
    /// method would be answered by the default there and never reach
    /// [`TauriEventReporter`] — the toast would silently never appear.
    /// Requiring an implementation makes that a compile error instead.
    fn report_pulled(&self, namespace: &Namespace, toast: ReportToast);

    /// A package's autosync state may have changed: a walk, a pass, a pause,
    /// a commit or a settings change. Without a default, for the same reason
    /// as [`Self::report_pulled`].
    fn report_autosync_changed(&self);
}

/// Stderr/log-only reporter. Used in tests where no Tauri runtime is
/// available, and reserved for a future headless CLI daemon (see
/// `plans/autosync/01-autopull/approach.md` "Follow-ups").
#[cfg_attr(not(test), allow(dead_code))]
pub struct LogReporter;

impl StatusReporter for LogReporter {
    fn report_autosync_changed(&self) {}

    fn report_pulled(&self, namespace: &Namespace, toast: ReportToast) {
        info!(
            "autosync: pulled namespace={namespace} kind={:?} body={}",
            toast.kind, toast.body
        );
    }

    fn report_status(&self, namespace: &Namespace, event: PackageStatusEvent) {
        // `debug`, not `info`: one line per package per tick. It is a progress
        // signal — the same reason it is not a countable event — and at info it
        // filled the log with hundreds of "up_to_date, no changes" a minute.
        debug!(
            "autosync: namespace={namespace} status={} has_changes={}",
            event.status, event.has_changes,
        );
    }

    fn report_paused(&self, namespace: &Namespace, host: Option<&Host>, reason: PausedReason) {
        let host = log_host(host);
        info!("autosync: paused namespace={namespace} host={host} reason={reason:?}");
    }

    fn report_login_required(&self, host: Option<&Host>, block: LoginBlock) {
        if let Some(h) = host {
            warn!("autosync: login required for {h} ({block:?})");
        } else {
            warn!("autosync: login required ({block:?})");
        }
    }
}

/// Production reporter: emits typed events on the Tauri event bus and
/// also logs so file-tail-style debugging still works.
/// Wraps another reporter and, in passing, tells telemetry what the engine just
/// did.
///
/// A decorator rather than calls threaded into the tick: the engine already
/// funnels every outcome through [`StatusReporter`], so that trait *is* the seam,
/// and adding a second one inside the loop would mean two places to keep in step.
/// The inner reporter still does its job — this only observes.
///
/// **Not every method reports.** `report_status` fires per package per tick and
/// carries a fingerprint precisely so consumers can discard repeats; it is a
/// progress signal, not a countable act, and sending it would swamp the
/// vocabulary. A subscriber error is reported as a *fault* rather than an event,
/// because the analytics vocabulary carries no error events by design.
///
/// Emission is spawned rather than awaited, because the trait is synchronous and
/// telemetry is not. That is the right shape regardless: the engine must not wait
/// on a network call to finish its tick.
pub struct TelemetryReporter {
    handle: tauri::AppHandle,
    inner: Arc<dyn StatusReporter>,
}

impl TelemetryReporter {
    pub fn wrapping(inner: Arc<dyn StatusReporter>, handle: tauri::AppHandle) -> Self {
        Self { handle, inner }
    }

    /// Hand `event` to telemetry without blocking the caller.
    ///
    /// The `Telemetry` lives in Tauri state rather than being held here, so a
    /// reporter constructed before it is managed still works — the same reason
    /// [`TauriEventReporter`] holds a handle instead of a window.
    fn emit(&self, event: MixpanelEvent) {
        let handle = self.handle.clone();
        tauri::async_runtime::spawn(async move {
            handle.state::<Telemetry>().track(event);
        });
    }
}

impl StatusReporter for TelemetryReporter {
    fn report_status(&self, namespace: &Namespace, event: PackageStatusEvent) {
        // Deliberately not reported — see the type's note on volume.
        self.inner.report_status(namespace, event);
    }

    fn report_paused(&self, namespace: &Namespace, host: Option<&Host>, reason: PausedReason) {
        self.emit(MixpanelEvent::AutosyncPaused(AutosyncPausedEvent {
            host: host.cloned(),
            reason: PausedKind::from(&reason),
        }));
        self.inner.report_paused(namespace, host, reason);
    }

    fn report_login_required(&self, host: Option<&Host>, block: LoginBlock) {
        // Once per deployment per episode. The loop rediscovers the same expired
        // session on every backoff-due tick and for every package on that host, so
        // counting discoveries would report one expiry as many.
        if block == LoginBlock::Began {
            self.emit(MixpanelEvent::AutosyncLoginRequired(AutosyncAuthEvent {
                host: host.cloned(),
            }));
        }
        self.inner.report_login_required(host, block);
    }

    fn report_subscriber_error(&self, event: SubscriberErrorEvent) {
        // A fault, not an event: the vocabulary has no error events, and the
        // filesystem watcher concerns no deployment, so there would be nothing to
        // attribute one to. The message is a constant so the reporter groups it as
        // one issue rather than one per path.
        let handle = self.handle.clone();
        tauri::async_runtime::spawn(async move {
            handle
                .state::<Telemetry>()
                .report_anomaly("Autosync filesystem watcher reported an error");
        });
        self.inner.report_subscriber_error(event);
    }

    fn report_published(&self, namespace: &Namespace, host: Option<&Host>, message: &str) {
        self.emit(MixpanelEvent::AutosyncPublished(AutosyncEvent {
            host: host.cloned(),
        }));
        self.inner.report_published(namespace, host, message);
    }

    /// Forwarded, not counted. What a pull brought is a report to the user, not
    /// an analytics outcome — and the autopull tick already emits its own
    /// events for what it did.
    fn report_pulled(&self, namespace: &Namespace, toast: ReportToast) {
        self.inner.report_pulled(namespace, toast);
    }

    fn report_autosync_changed(&self) {
        self.inner.report_autosync_changed();
    }
}

pub struct TauriEventReporter {
    handle: tauri::AppHandle,
    logged: SeenFingerprints,
}

/// Remembers the last status fingerprint *logged* per package.
///
/// A tick recomputes status whether or not anything moved, so the log filled with
/// the same answer several times a second — near-zero information at real cost. The
/// fingerprint already exists to tell "same observation" from "different one", for
/// the UI's benefit; a log reader wants the same distinction.
///
/// Logging only, never emission: the UI dedupes for itself and must keep receiving
/// every event, or a consumer that missed one would wait forever for a repeat that
/// never comes.
#[derive(Default)]
struct SeenFingerprints(std::sync::Mutex<std::collections::BTreeMap<Namespace, String>>);

impl SeenFingerprints {
    /// Whether this observation differs from the last one seen for `namespace`.
    ///
    /// A poisoned lock answers "yes": a duplicate line is a better failure than a
    /// silence that hides a real change.
    fn is_news(&self, namespace: &Namespace, fingerprint: &str) -> bool {
        let Ok(mut seen) = self.0.lock() else {
            return true;
        };
        seen.insert(namespace.clone(), fingerprint.to_owned())
            .is_none_or(|previous| previous != fingerprint)
    }
}

impl TauriEventReporter {
    pub fn new(handle: tauri::AppHandle) -> Self {
        Self {
            handle,
            logged: SeenFingerprints::default(),
        }
    }
}

impl StatusReporter for TauriEventReporter {
    fn report_status(&self, namespace: &Namespace, event: PackageStatusEvent) {
        // Only when the observation actually changed. A tick recomputes status
        // several times a second and almost always finds the same thing, so logging
        // each one buys nothing and costs a line — this is a progress signal, which
        // is also why it is not a countable event.
        if self.logged.is_news(namespace, &event.fingerprint) {
            debug!(
                "autosync: namespace={namespace} status={} has_changes={}",
                event.status, event.has_changes,
            );
        } else {
            trace!(
                "autosync: namespace={namespace} unchanged ({})",
                event.status
            );
        }
        if let Err(err) = self.handle.emit(STATUS_EVENT, &event) {
            warn!("autosync: failed to emit {STATUS_EVENT}: {err}");
        }
    }

    fn report_paused(&self, namespace: &Namespace, host: Option<&Host>, reason: PausedReason) {
        let host = log_host(host);
        info!("autosync: paused namespace={namespace} host={host} reason={reason:?}");
        let payload = PausedEvent::from_reason(namespace, &reason);
        if let Err(err) = self.handle.emit(PAUSED_EVENT, &payload) {
            warn!("autosync: failed to emit {PAUSED_EVENT}: {err}");
        }
    }

    fn report_login_required(&self, host: Option<&Host>, block: LoginBlock) {
        if let Some(h) = host {
            warn!("autosync: login required for {h} ({block:?})");
        } else {
            warn!("autosync: login required ({block:?})");
        }
        // TODO(autosync/03-merge-conflicts.md): no UI listener yet.
        let payload = LoginRequiredEvent {
            host: host.map(ToString::to_string),
        };
        if let Err(err) = self.handle.emit(LOGIN_REQUIRED_EVENT, &payload) {
            warn!("autosync: failed to emit {LOGIN_REQUIRED_EVENT}: {err}");
        }
    }

    fn report_subscriber_error(&self, event: SubscriberErrorEvent) {
        warn!(
            "fswatcher: kind={} namespace={:?} message={}",
            event.kind, event.namespace, event.message
        );
        if let Err(err) = self.handle.emit(SUBSCRIBER_ERROR_EVENT, &event) {
            warn!("fswatcher: failed to emit {SUBSCRIBER_ERROR_EVENT}: {err}");
        }
    }

    fn report_pulled(&self, namespace: &Namespace, toast: ReportToast) {
        info!(
            "autosync: pulled namespace={namespace} kind={:?}",
            toast.kind
        );
        // The centre is Tauri-managed state and posting is async, while every
        // other method here is a synchronous fire-and-forget emit. Spawning
        // keeps that shape rather than making the whole trait async for one
        // method.
        let handle = self.handle.clone();
        tauri::async_runtime::spawn(async move {
            handle
                .state::<ToastCenter>()
                .post(crate::engine::toast::ToastDraft {
                    kind: toast.kind,
                    title: Some(toast.title),
                    body: toast.body,
                    groups: toast.groups,
                    timeout_ms: None,
                })
                .await;
        });
    }

    fn report_published(&self, namespace: &Namespace, host: Option<&Host>, message: &str) {
        let host = log_host(host);
        info!("autosync: published namespace={namespace} host={host} message={message}");
        let payload = PublishedEvent {
            namespace: namespace.clone(),
            message: message.to_string(),
        };
        if let Err(err) = self.handle.emit(PUBLISHED_EVENT, &payload) {
            warn!("autosync: failed to emit {PUBLISHED_EVENT}: {err}");
        }
    }

    fn report_autosync_changed(&self) {
        if let Err(err) = self.handle.emit(AUTOSYNC_CHANGED_EVENT, ()) {
            warn!("autosync: failed to emit {AUTOSYNC_CHANGED_EVENT}: {err}");
        }
    }
}

/// A host for a log line: a package with no catalog has none.
fn log_host(host: Option<&Host>) -> String {
    host.map_or_else(|| "none".to_string(), ToString::to_string)
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct LoginRequiredEvent {
    host: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The dedup that keeps an idle app quiet: the first observation is news, an
    /// identical repeat is not, and a change is news again.
    ///
    /// Without this the log carried the same `up_to_date, no changes` several times
    /// a second — the whole answer to "why is this file always growing".
    #[test]
    fn an_unchanged_observation_is_not_news_twice() {
        let seen = SeenFingerprints::default();
        let ns: Namespace = ("acme", "demo").into();

        assert!(seen.is_news(&ns, "fp-1"), "the first is always news");
        assert!(!seen.is_news(&ns, "fp-1"), "an identical repeat is not");
        assert!(!seen.is_news(&ns, "fp-1"), "and stays not, however often");
        assert!(seen.is_news(&ns, "fp-2"), "a change is news again");
        assert!(!seen.is_news(&ns, "fp-2"), "then settles");
    }

    /// Per package, not global — two packages reporting the same state must both be
    /// heard, or one would silence the other.
    #[test]
    fn packages_are_tracked_apart() {
        let seen = SeenFingerprints::default();
        let one: Namespace = ("acme", "one").into();
        let other: Namespace = ("acme", "other").into();

        assert!(seen.is_news(&one, "same"));
        assert!(
            seen.is_news(&other, "same"),
            "a second package's first observation is its own news"
        );
        assert!(!seen.is_news(&one, "same"));
    }

    /// Hex of an ASCII string — mirrors the per-byte encoding the fingerprint
    /// applies to a path, for asserting a path appears hex-encoded in a digest.
    fn hex(s: &str) -> String {
        s.bytes().fold(String::new(), |mut out, b| {
            let _ = write!(out, "{b:02x}");
            out
        })
    }

    #[test]
    fn from_status_assembles_status_has_changes_and_fingerprint_from_one_observation() {
        use std::collections::BTreeMap;
        use std::path::PathBuf;

        let ns: Namespace = ("acme", "demo").into();
        let mut changes = BTreeMap::new();
        changes.insert(
            PathBuf::from("a.txt"),
            quilt::lineage::Change::Modified(quilt::manifest::ManifestRow::default()),
        );
        let status = quilt::lineage::InstalledPackageStatus::new(
            quilt::lineage::UpstreamState::UpToDate,
            changes,
        );

        let event = PackageStatusEvent::from_status(&ns, &status);

        // status and has_changes are read from the same status...
        assert_eq!(event.status, "up_to_date");
        assert!(event.has_changes);
        // ...and the fingerprint carries the upstream state plus the changed
        // path (hex-encoded) and its kind, so the three describe one observation.
        assert!(
            event.fingerprint.starts_with("up_to_date;"),
            "fingerprint should lead with the upstream state, got: {}",
            event.fingerprint
        );
        let hex_path = hex("a.txt");
        assert!(
            event.fingerprint.contains(&format!("{hex_path}:M:")),
            "fingerprint should carry the hex-encoded path and kind, got: {}",
            event.fingerprint
        );
    }

    fn fingerprint_of_one_modified(path: std::path::PathBuf) -> String {
        use std::collections::BTreeMap;
        let mut changes = BTreeMap::new();
        changes.insert(
            path,
            quilt::lineage::Change::Modified(quilt::manifest::ManifestRow::default()),
        );
        PackageStatusEvent::from_status(
            &("acme", "demo").into(),
            &quilt::lineage::InstalledPackageStatus::new(
                quilt::lineage::UpstreamState::UpToDate,
                changes,
            ),
        )
        .fingerprint
    }

    #[test]
    fn fingerprint_hex_encodes_paths_so_delimiters_cannot_collide() {
        use std::path::PathBuf;

        // The path is the only field that can hold the `:`/`;` delimiters (kind
        // is A/M/D, the hash is hex/base64). Hex-encoded, a crafted path like
        // `a:M:<hash>;b` can't serialize like two separate entries, so a genuine
        // change is never read as a duplicate.
        let fp = fingerprint_of_one_modified(PathBuf::from("weird;name:with:delims"));
        let expected = hex("weird;name:with:delims");
        assert!(
            fp.contains(&expected),
            "path should be hex-encoded, got: {fp}"
        );
        assert!(
            !fp.contains("name:with"),
            "raw path delimiters must not survive to be read as separators, got: {fp}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn fingerprint_distinguishes_non_utf8_paths() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::path::PathBuf;

        // Two paths that differ only in invalid-UTF-8 bytes fold to the same
        // `to_string_lossy()` output (both `foo` + the replacement char), which
        // would collide their fingerprints. Hex-encoding the lossless bytes
        // keeps them distinct.
        let a = fingerprint_of_one_modified(PathBuf::from(OsStr::from_bytes(b"foo\xff")));
        let b = fingerprint_of_one_modified(PathBuf::from(OsStr::from_bytes(b"foo\xfe")));
        assert_ne!(a, b, "non-UTF-8 paths must not collide in the fingerprint");
    }

    /// A behind package with no local changes whose `latest` is `latest`.
    fn behind_with_latest(latest: &str) -> quilt::lineage::InstalledPackageStatus {
        quilt::lineage::InstalledPackageStatus {
            latest_hash: latest.to_string(),
            ..quilt::lineage::InstalledPackageStatus::new(
                quilt::lineage::UpstreamState::Behind,
                std::collections::BTreeMap::new(),
            )
        }
    }

    #[test]
    fn fingerprint_names_the_newer_revision() {
        // The remote moved from one newer revision to the next: still behind,
        // the same local changes. Without the hash these two collide, and an
        // open page drops the tick that read the move as old news.
        let first = status_fingerprint(&behind_with_latest("aaaa"));
        let next = status_fingerprint(&behind_with_latest("bbbb"));
        assert_ne!(first, next, "a moved `latest` is a different observation");
        assert!(
            first.starts_with("behind;"),
            "the upstream state still leads, got: {first}"
        );
    }

    #[test]
    fn fingerprint_hex_encodes_the_newer_revision() {
        // A hash is hex today, but the digest does not lean on that: encoded,
        // no `latest` can carry a delimiter into the entries after it.
        let fp = status_fingerprint(&behind_with_latest("feed;beef"));
        assert!(fp.contains(&hex("feed;beef")), "got: {fp}");
        assert!(!fp.contains("feed;beef"), "got: {fp}");
    }

    #[test]
    fn settled_fingerprint_is_the_settled_observation() {
        // After its own pull or publish the tick fingerprints the tree it
        // reached. The next tick observes that tree for real; the two must
        // match, or every sync would cost an open page one more re-read.
        let observed = quilt::lineage::InstalledPackageStatus {
            latest_hash: "cafe".to_string(),
            ..quilt::lineage::InstalledPackageStatus::new(
                quilt::lineage::UpstreamState::UpToDate,
                std::collections::BTreeMap::new(),
            )
        };
        assert_eq!(
            settled_fingerprint("cafe", ""),
            status_fingerprint(&observed)
        );
        assert_ne!(settled_fingerprint("cafe", ""), settled_fingerprint("", ""));
    }

    fn ignored_by(
        pattern: &str,
        source: quilt::quiltignore::RuleSource,
    ) -> quilt::quiltignore::IgnoredBy {
        quilt::quiltignore::IgnoredBy {
            pattern: pattern.to_string(),
            source,
        }
    }

    /// An up-to-date package with no changes and `.env` ignored by `by`.
    fn with_ignored_env(
        by: quilt::quiltignore::IgnoredBy,
    ) -> quilt::lineage::InstalledPackageStatus {
        let mut status = behind_with_latest("cafe");
        status.upstream_state = quilt::lineage::UpstreamState::UpToDate;
        status.ignored_files = vec![(std::path::PathBuf::from(".env"), by, 12)];
        status
    }

    #[test]
    fn fingerprint_names_the_rule_that_ignored_a_file() {
        // A user-wide rule now ignores `.env` instead of the default one: the
        // page shows a different rule, so it must refetch.
        use quilt::quiltignore::RuleSource;
        let by_default =
            status_fingerprint(&with_ignored_env(ignored_by(".env", RuleSource::Default)));
        let by_user = status_fingerprint(&with_ignored_env(ignored_by(
            ".env",
            RuleSource::User {
                path: std::path::PathBuf::from("/home/u/.quiltignore"),
                line: 1,
            },
        )));
        let by_other_pattern =
            status_fingerprint(&with_ignored_env(ignored_by(".env*", RuleSource::Default)));
        let by_package = status_fingerprint(&with_ignored_env(ignored_by(
            ".env",
            RuleSource::Package { line: 1 },
        )));
        assert_ne!(by_default, by_user);
        assert_ne!(by_default, by_other_pattern);
        assert_ne!(by_default, by_package);
        assert_ne!(by_user, by_package);
        assert_ne!(by_default, status_fingerprint(&behind_with_latest("cafe")));
    }

    #[test]
    fn fingerprint_names_the_size_of_an_ignored_file() {
        let by = ignored_by(".env", quilt::quiltignore::RuleSource::Default);
        let mut grown = with_ignored_env(by.clone());
        grown.ignored_files[0].2 = 13;
        assert_ne!(
            status_fingerprint(&with_ignored_env(by)),
            status_fingerprint(&grown)
        );
    }

    #[test]
    fn fingerprint_names_ignored_folders_apart_from_files() {
        use quilt::quiltignore::RuleSource;
        let mut plain = behind_with_latest("cafe");
        plain.upstream_state = quilt::lineage::UpstreamState::UpToDate;
        let folder = |pattern: &str| {
            let mut status = behind_with_latest("cafe");
            status.upstream_state = quilt::lineage::UpstreamState::UpToDate;
            status.ignored_dirs = vec![(
                std::path::PathBuf::from("node_modules"),
                ignored_by(pattern, RuleSource::Default),
            )];
            status
        };
        let a = status_fingerprint(&folder("node_modules/"));
        let b = status_fingerprint(&folder("node_*/"));
        assert_ne!(a, b, "a different rule for the folder is a new observation");
        assert_ne!(a, status_fingerprint(&plain));

        // The same row as a file is a different observation from a folder.
        let mut as_file = plain;
        as_file.ignored_files = vec![(
            std::path::PathBuf::from("node_modules"),
            ignored_by("node_modules/", RuleSource::Default),
            0,
        )];
        assert_ne!(a, status_fingerprint(&as_file));
    }

    #[test]
    fn fingerprint_of_ignored_rows_ignores_walk_order() {
        // The walk reads folders in whatever order the filesystem gives, so
        // the same rows must digest the same in any order.
        use quilt::quiltignore::RuleSource;
        let row = |name: &str| {
            (
                std::path::PathBuf::from(name),
                ignored_by(name, RuleSource::Package { line: 2 }),
                1,
            )
        };
        let mut one = behind_with_latest("cafe");
        one.ignored_files = vec![row(".env"), row(".netrc")];
        let mut other = behind_with_latest("cafe");
        other.ignored_files = vec![row(".netrc"), row(".env")];
        assert_eq!(status_fingerprint(&one), status_fingerprint(&one));
        assert_eq!(status_fingerprint(&one), status_fingerprint(&other));
    }

    #[test]
    fn fingerprint_hex_encodes_patterns_and_the_user_rules_path() {
        use quilt::quiltignore::RuleSource;
        let fp = status_fingerprint(&with_ignored_env(ignored_by(
            "a;b|c",
            RuleSource::User {
                path: std::path::PathBuf::from("/x;y|z"),
                line: 3,
            },
        )));
        assert!(fp.contains(&hex("a;b|c")), "got: {fp}");
        assert!(fp.contains(&hex("/x;y|z")), "got: {fp}");
        assert!(!fp.contains("a;b|c") && !fp.contains("/x;y|z"), "got: {fp}");
    }

    #[test]
    fn changes_fingerprint_leaves_ignored_rows_out() {
        // Ignored files are never published, so they must not move the
        // last-change time that paces autopublish.
        let plain = behind_with_latest("cafe");
        let ignored = with_ignored_env(ignored_by(".env", quilt::quiltignore::RuleSource::Default));
        assert_eq!(
            changes_fingerprint(&plain.changes),
            changes_fingerprint(&ignored.changes)
        );
    }

    #[test]
    fn settled_fingerprint_keeps_the_ignored_rows() {
        // A publish leaves ignored files alone, so the tree it settles on
        // still shows them, and the next tick must find that old news.
        let observed =
            with_ignored_env(ignored_by(".env", quilt::quiltignore::RuleSource::Default));
        assert_eq!(
            settled_fingerprint("cafe", &ignored_fingerprint(&observed)),
            status_fingerprint(&observed)
        );
    }

    #[test]
    fn package_status_event_serializes_camel_case() {
        let event = PackageStatusEvent {
            namespace: quilt_uri::Namespace::try_from("acme/demo")
                .expect("a valid fixture namespace"),
            status: "up_to_date".to_string(),
            has_changes: false,
            fingerprint: "up_to_date;".to_string(),
        };
        let json = serde_json::to_string(&event).unwrap();
        assert!(
            json.contains(r#""hasChanges":false"#),
            "expected camelCase `hasChanges`, got: {json}"
        );
        assert!(json.contains(r#""status":"up_to_date""#));
        assert!(json.contains(r#""namespace":"acme/demo""#));
        assert!(
            json.contains(r#""fingerprint":"up_to_date;""#),
            "expected the fingerprint on the wire, got: {json}"
        );
    }

    #[test]
    fn paused_event_from_reason_known_variants_have_no_message() {
        let ns = quilt_uri::Namespace::from(("acme", "demo"));
        assert_eq!(
            PausedEvent::from_reason(&ns, &PausedReason::PendingChanges),
            PausedEvent {
                namespace: quilt_uri::Namespace::try_from("acme/demo")
                    .expect("a valid fixture namespace"),
                reason: "pendingChanges".to_string(),
                message: None,
            }
        );
        assert_eq!(
            PausedEvent::from_reason(&ns, &PausedReason::PendingCommit).reason,
            "pendingCommit"
        );
        assert_eq!(
            PausedEvent::from_reason(&ns, &PausedReason::Diverged).reason,
            "diverged"
        );
    }

    /// The banner needs the role name to say which role to switch away from,
    /// so it rides in `message` — the same data-not-copy split `pullConflict`
    /// uses for its file list.
    #[test]
    fn paused_event_role_denied_carries_the_role_name() {
        let ns = quilt_uri::Namespace::from(("acme", "demo"));
        let ev = PausedEvent::from_reason(
            &ns,
            &PausedReason::RoleDenied {
                role: "ReadOnly".to_string(),
            },
        );
        assert_eq!(ev.reason, "roleDenied");
        assert_eq!(ev.message.as_deref(), Some("ReadOnly"));
    }

    /// An unresolved role name sends no message at all, so the UI picks its
    /// role-less phrasing instead of rendering an empty name.
    #[test]
    fn paused_event_role_denied_without_a_name_sends_no_message() {
        let ns = quilt_uri::Namespace::from(("acme", "demo"));
        let ev = PausedEvent::from_reason(
            &ns,
            &PausedReason::RoleDenied {
                role: String::new(),
            },
        );
        assert_eq!(ev.reason, "roleDenied");
        assert!(ev.message.is_none(), "got: {:?}", ev.message);
    }

    #[test]
    fn paused_event_from_reason_other_carries_raw_reason_only() {
        let ns = quilt_uri::Namespace::from(("acme", "demo"));
        let ev = PausedEvent::from_reason(
            &ns,
            &PausedReason::Other("workflow rejected metadata".to_string()),
        );
        assert_eq!(ev.reason, "other");

        // `message` is exactly the raw refusal reason — no appended
        // guidance. The "resolve, then push manually" line is presentation
        // and is added by each UI surface, not baked into the data.
        assert_eq!(
            ev.message.as_deref(),
            Some("workflow rejected metadata"),
            "Other should carry the raw reason with no appended hint"
        );

        // Serializes as camelCase with the raw reason as the message.
        let json = serde_json::to_string(&ev).unwrap();
        assert!(json.contains(r#""reason":"other""#), "got: {json}");
        assert!(json.contains("workflow rejected metadata"), "got: {json}");
        assert!(json.contains(r#""namespace":"acme/demo""#), "got: {json}");
    }
}

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Mutex;

    use super::*;

    #[derive(Default)]
    pub struct RecordingReporter {
        pub statuses: Mutex<Vec<(Namespace, PackageStatusEvent)>>,
        pub paused: Mutex<Vec<(Namespace, PausedReason)>>,
        pub logins: Mutex<Vec<Option<Host>>>,
        /// Whether each login report was the start of an episode or a repeat, so a
        /// test can assert one expiry is counted once.
        pub login_blocks: Mutex<Vec<LoginBlock>>,
        pub subscriber_errors: Mutex<Vec<SubscriberErrorEvent>>,
        pub published: Mutex<Vec<(Namespace, String)>>,
        /// Hosts seen on paused and published reports, so a test can assert the
        /// engine attributed a report to the package's own catalog, or to none.
        pub hosts: Mutex<Vec<Option<Host>>>,
        /// What each pull was reported as bringing, so a test can assert the
        /// wording rather than merely that something was posted.
        pub pulled: Mutex<Vec<(Namespace, ReportToast)>>,
        /// How many times the window was told a state may have changed.
        pub autosync_changed: std::sync::atomic::AtomicUsize,
    }

    impl StatusReporter for RecordingReporter {
        fn report_autosync_changed(&self) {
            self.autosync_changed
                .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }

        fn report_pulled(&self, namespace: &Namespace, toast: ReportToast) {
            self.pulled.lock().unwrap().push((namespace.clone(), toast));
        }

        fn report_status(&self, namespace: &Namespace, event: PackageStatusEvent) {
            self.statuses
                .lock()
                .unwrap()
                .push((namespace.clone(), event));
        }

        fn report_paused(&self, namespace: &Namespace, host: Option<&Host>, reason: PausedReason) {
            self.hosts.lock().unwrap().push(host.cloned());
            self.paused
                .lock()
                .unwrap()
                .push((namespace.clone(), reason));
        }

        fn report_login_required(&self, host: Option<&Host>, block: LoginBlock) {
            self.login_blocks.lock().unwrap().push(block);
            self.logins.lock().unwrap().push(host.cloned());
        }

        fn report_subscriber_error(&self, event: SubscriberErrorEvent) {
            self.subscriber_errors.lock().unwrap().push(event);
        }

        fn report_published(&self, namespace: &Namespace, host: Option<&Host>, message: &str) {
            self.hosts.lock().unwrap().push(host.cloned());
            self.published
                .lock()
                .unwrap()
                .push((namespace.clone(), message.to_string()));
        }
    }
}
