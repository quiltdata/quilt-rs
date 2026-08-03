use std::fmt::Write;

use serde::Serialize;

use quilt_uri::Host;
use quilt_uri::Namespace;
use tauri::Emitter;

use crate::autopull::PausedReason;
use crate::quilt;
use crate::telemetry::prelude::*;

/// Event names. Kept in lockstep with the UI's `listen(...)` calls.
pub const STATUS_EVENT: &str = "package-status-changed";
pub const LOGIN_REQUIRED_EVENT: &str = "autosync-login-required";
pub const SUBSCRIBER_ERROR_EVENT: &str = "fswatcher-subscriber-error";
pub const PUBLISHED_EVENT: &str = "autosync-published";
pub const PAUSED_EVENT: &str = "autosync-paused";

/// Payload emitted to the UI when a package's upstream state changes after
/// a watcher tick. Mirrors the camelCase shape of `RefreshedPackageStatus`
/// so the UI can reuse its existing per-package signal.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PackageStatusEvent {
    pub namespace: String,
    pub status: String,
    pub has_changes: bool,
    /// Digest of the observation this event reports — the upstream state
    /// and, per changed path, its kind and content hash. Two events with
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
            namespace: namespace.to_string(),
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
/// Format: `<upstream>;<hex(path)>:<kind>:<hash>;<hex(path)>:<kind>:<hash>;...`
/// Paths are walked in `BTreeMap` order (sorted by `PathBuf`), so the output
/// is deterministic without an explicit sort. The path is the only field that
/// can hold arbitrary bytes — non-UTF-8 names on Linux, or the `:`/`;`
/// delimiters (kind is A/M/D, the hash is hex/base64) — so it is **hex-encoded
/// from its lossless bytes**. Hex is `0-9a-f` only, so it is both lossless and
/// delimiter-free, keeping the digest **injective**: no two distinct
/// observations share a fingerprint, so a consumer never skips a real refetch.
/// (A lossy path string would fold two distinct non-UTF-8 names to the same
/// `�`; a raw path could serialize a crafted `a:M:<hash>;b` like two entries.)
pub(crate) fn status_fingerprint(status: &quilt::lineage::InstalledPackageStatus) -> String {
    let mut out = String::new();
    let _ = write!(out, "{};", status.upstream_state);
    for (path, change) in &status.changes {
        let (kind, row) = match change {
            quilt::lineage::Change::Added(r) => ("A", r),
            quilt::lineage::Change::Modified(r) => ("M", r),
            quilt::lineage::Change::Removed(r) => ("D", r),
        };
        for b in path.as_os_str().as_encoded_bytes() {
            let _ = write!(out, "{b:02x}");
        }
        let _ = write!(out, ":{kind}:{};", row.hash);
    }
    out
}

/// Fingerprint of a clean, up-to-date working tree — the state a package
/// reaches right after a successful publish, and after a pull that kept no
/// local work. The mutation paths in the tick don't hold a post-mutation
/// `InstalledPackageStatus` to fingerprint directly, but the observation they
/// represent is a settled `UpToDate` tree, and this names it. (A pull that
/// *kept* local changes reads this too; it understates by one changed-path
/// digest, which the next tick's real observation corrects — a redundant
/// refetch at worst, never a missed one.)
pub(crate) fn clean_uptodate_fingerprint() -> String {
    status_fingerprint(&quilt::lineage::InstalledPackageStatus::new(
        quilt::lineage::UpstreamState::UpToDate,
        std::collections::BTreeMap::new(),
    ))
}

/// Payload emitted to the UI after autosync publishes a package. The UI
/// renders this as the same toast surface manual Commit & Push uses.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct PublishedEvent {
    pub namespace: String,
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
    pub namespace: String,
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
            namespace: namespace.to_string(),
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
    fn report_paused(&self, namespace: &Namespace, reason: PausedReason);
    fn report_login_required(&self, host: Option<&Host>);
    fn report_subscriber_error(&self, event: SubscriberErrorEvent) {
        warn!(
            "fswatcher: kind={} namespace={:?} message={}",
            event.kind, event.namespace, event.message
        );
    }
    /// Surface a successful autosync publish. Default implementation
    /// logs only; `TauriEventReporter` also emits `PUBLISHED_EVENT`.
    fn report_published(&self, namespace: &Namespace, message: &str) {
        info!("autosync: published namespace={namespace} message={message}");
    }
}

/// Stderr/log-only reporter. Used in tests where no Tauri runtime is
/// available, and reserved for a future headless CLI daemon (see
/// `plans/autosync/01-autopull/approach.md` "Follow-ups").
#[cfg_attr(not(test), allow(dead_code))]
pub struct LogReporter;

impl StatusReporter for LogReporter {
    fn report_status(&self, namespace: &Namespace, event: PackageStatusEvent) {
        info!(
            "autosync: namespace={namespace} status={} has_changes={}",
            event.status, event.has_changes,
        );
    }

    fn report_paused(&self, namespace: &Namespace, reason: PausedReason) {
        info!("autosync: paused namespace={namespace} reason={reason:?}");
    }

    fn report_login_required(&self, host: Option<&Host>) {
        if let Some(h) = host {
            warn!("autosync: login required for {h}");
        } else {
            warn!("autosync: login required");
        }
    }
}

/// Production reporter: emits typed events on the Tauri event bus and
/// also logs so file-tail-style debugging still works.
pub struct TauriEventReporter {
    handle: tauri::AppHandle,
}

impl TauriEventReporter {
    pub fn new(handle: tauri::AppHandle) -> Self {
        Self { handle }
    }
}

impl StatusReporter for TauriEventReporter {
    fn report_status(&self, namespace: &Namespace, event: PackageStatusEvent) {
        info!(
            "autosync: namespace={namespace} status={} has_changes={}",
            event.status, event.has_changes,
        );
        if let Err(err) = self.handle.emit(STATUS_EVENT, &event) {
            warn!("autosync: failed to emit {STATUS_EVENT}: {err}");
        }
    }

    fn report_paused(&self, namespace: &Namespace, reason: PausedReason) {
        info!("autosync: paused namespace={namespace} reason={reason:?}");
        let payload = PausedEvent::from_reason(namespace, &reason);
        if let Err(err) = self.handle.emit(PAUSED_EVENT, &payload) {
            warn!("autosync: failed to emit {PAUSED_EVENT}: {err}");
        }
    }

    fn report_login_required(&self, host: Option<&Host>) {
        if let Some(h) = host {
            warn!("autosync: login required for {h}");
        } else {
            warn!("autosync: login required");
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

    fn report_published(&self, namespace: &Namespace, message: &str) {
        info!("autosync: published namespace={namespace} message={message}");
        let payload = PublishedEvent {
            namespace: namespace.to_string(),
            message: message.to_string(),
        };
        if let Err(err) = self.handle.emit(PUBLISHED_EVENT, &payload) {
            warn!("autosync: failed to emit {PUBLISHED_EVENT}: {err}");
        }
    }
}

#[derive(Serialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
struct LoginRequiredEvent {
    host: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

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

    #[test]
    fn package_status_event_serializes_camel_case() {
        let event = PackageStatusEvent {
            namespace: "acme/demo".to_string(),
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
                namespace: "acme/demo".to_string(),
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
        pub subscriber_errors: Mutex<Vec<SubscriberErrorEvent>>,
        pub published: Mutex<Vec<(Namespace, String)>>,
    }

    impl StatusReporter for RecordingReporter {
        fn report_status(&self, namespace: &Namespace, event: PackageStatusEvent) {
            self.statuses
                .lock()
                .unwrap()
                .push((namespace.clone(), event));
        }

        fn report_paused(&self, namespace: &Namespace, reason: PausedReason) {
            self.paused
                .lock()
                .unwrap()
                .push((namespace.clone(), reason));
        }

        fn report_login_required(&self, host: Option<&Host>) {
            self.logins.lock().unwrap().push(host.cloned());
        }

        fn report_subscriber_error(&self, event: SubscriberErrorEvent) {
            self.subscriber_errors.lock().unwrap().push(event);
        }

        fn report_published(&self, namespace: &Namespace, message: &str) {
            self.published
                .lock()
                .unwrap()
                .push((namespace.clone(), message.to_string()));
        }
    }
}
