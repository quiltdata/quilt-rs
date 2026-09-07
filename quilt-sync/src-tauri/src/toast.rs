//! Server-emitted notifications the client stacks as toasts.
//!
//! Distinct from [`notify`](crate::notify), which is the log-and-telemetry
//! helper around a command's result — this is a user-facing surface.
//!
//! **Additive by construction.** The existing per-page notification slot
//! (`Layout`'s single `Option<Notification>`) is untouched; nothing that
//! reports through it changes. This is where *new* notifications go, and the
//! difference that earns a second mechanism is the direction: a page toast is
//! raised by something the user just did and dies with the page, while these
//! are raised by the backend — including the autosync tick, which runs when no
//! page is mounted at all.
//!
//! Which is why the centre **retains** what it emits. An event alone reaches
//! only a mounted window, so a toast raised while the app was in the tray
//! would be lost; the client hydrates from [`ToastCenter::live`] on mount and
//! then follows [`TOAST_EVENT`]. Retention is in memory only: a restart is a
//! clean slate.

use std::collections::VecDeque;
use std::sync::atomic::AtomicU64;
use std::sync::atomic::Ordering;

use serde::Serialize;
use tauri::Emitter;
use tokio::sync::RwLock;

use crate::telemetry::prelude::*;

/// Event name. Kept in lockstep with the UI's `listen(...)` call.
#[cfg_attr(not(test), allow(dead_code))] // no producer until the revision report lands
pub const TOAST_EVENT: &str = "toast";

/// How many undismissed toasts the centre holds before the oldest is dropped.
///
/// A bound rather than a queue that grows forever: an app left in the tray over
/// a weekend of agent pushes would otherwise accumulate without limit, and a
/// user who has not read the oldest of fifty is not served by the fifty-first.
#[cfg_attr(not(test), allow(dead_code))] // no producer until the revision report lands
const CAPACITY: usize = 50;

/// One notification, as the client renders it.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Toast {
    /// Assigned by the centre. The client dismisses by id, so it must survive
    /// the round trip.
    pub id: u64,
    pub kind: ToastKind,
    /// A short heading. `None` renders the body alone.
    pub title: Option<String>,
    pub body: String,
    /// Auto-dismiss delay. `None` stands until the user closes it — which is
    /// the right default for anything reporting an unattended change, since a
    /// timer would race the user's absence.
    pub timeout_ms: Option<u32>,
}

#[cfg_attr(not(test), allow(dead_code))] // no producer until the revision report lands
#[derive(Serialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub enum ToastKind {
    Info,
    Success,
    Warning,
    Error,
}

/// Backend → frontend sink. A trait so the centre stays testable without a
/// Tauri runtime, the same reason [`StatusReporter`](crate::autopull::StatusReporter)
/// is one.
pub trait ToastEmitter: Send + Sync + 'static {
    #[cfg_attr(not(test), allow(dead_code))] // no producer until the revision report lands
    fn emit(&self, toast: &Toast);
}

/// Production sink: emits on the Tauri event bus.
pub struct TauriToastEmitter {
    handle: tauri::AppHandle,
}

impl TauriToastEmitter {
    #[must_use]
    pub fn new(handle: tauri::AppHandle) -> Self {
        Self { handle }
    }
}

impl ToastEmitter for TauriToastEmitter {
    fn emit(&self, toast: &Toast) {
        if let Err(err) = self.handle.emit(TOAST_EVENT, toast) {
            warn!("toast: failed to emit {TOAST_EVENT}: {err}");
        }
    }
}

/// The retained set of undismissed toasts, plus the ids to hand out.
pub struct ToastCenter {
    live: RwLock<VecDeque<Toast>>,
    next_id: AtomicU64,
    emitter: Box<dyn ToastEmitter>,
}

impl ToastCenter {
    #[must_use]
    pub fn new(emitter: Box<dyn ToastEmitter>) -> Self {
        Self {
            live: RwLock::new(VecDeque::new()),
            next_id: AtomicU64::new(1),
            emitter,
        }
    }

    /// Post a toast: retain it, then emit it. Returns its id.
    ///
    /// Retain-then-emit, not the reverse, so a client that hydrates in the
    /// window between the two sees the toast once rather than not at all —
    /// a duplicate is a render concern the client already de-duplicates by id,
    /// while a miss is unrecoverable.
    #[cfg_attr(not(test), allow(dead_code))] // no producer until the revision report lands
    pub async fn post(
        &self,
        kind: ToastKind,
        title: Option<String>,
        body: String,
        timeout_ms: Option<u32>,
    ) -> u64 {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let toast = Toast {
            id,
            kind,
            title,
            body,
            timeout_ms,
        };
        {
            let mut live = self.live.write().await;
            if live.len() >= CAPACITY {
                live.pop_front();
            }
            live.push_back(toast.clone());
        }
        info!("toast: posted id={id} kind={kind:?}");
        self.emitter.emit(&toast);
        id
    }

    /// Every undismissed toast, oldest first — the client's hydration read.
    pub async fn live(&self) -> Vec<Toast> {
        self.live.read().await.iter().cloned().collect()
    }

    /// Drop one toast. `false` when the id is unknown, which is the ordinary
    /// outcome of two windows dismissing the same toast, not an error.
    pub async fn dismiss(&self, id: u64) -> bool {
        let mut live = self.live.write().await;
        let before = live.len();
        live.retain(|t| t.id != id);
        before != live.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::Mutex;

    #[derive(Default)]
    struct Collector(Arc<Mutex<Vec<Toast>>>);

    impl ToastEmitter for Collector {
        fn emit(&self, toast: &Toast) {
            self.0.lock().unwrap().push(toast.clone());
        }
    }

    fn center() -> (ToastCenter, Arc<Mutex<Vec<Toast>>>) {
        let seen = Arc::new(Mutex::new(Vec::new()));
        let center = ToastCenter::new(Box::new(Collector(Arc::clone(&seen))));
        (center, seen)
    }

    #[tokio::test]
    async fn post_retains_and_emits() {
        let (center, seen) = center();
        let id = center
            .post(ToastKind::Info, None, "one".to_owned(), None)
            .await;
        assert_eq!(center.live().await.len(), 1);
        assert_eq!(seen.lock().unwrap().len(), 1);
        assert_eq!(seen.lock().unwrap()[0].id, id);
    }

    #[tokio::test]
    async fn ids_are_unique_and_live_is_oldest_first() {
        let (center, _) = center();
        for body in ["one", "two", "three"] {
            center
                .post(ToastKind::Info, None, body.to_owned(), None)
                .await;
        }
        let live = center.live().await;
        let bodies: Vec<_> = live.iter().map(|t| t.body.as_str()).collect();
        assert_eq!(bodies, vec!["one", "two", "three"]);
        let ids: Vec<_> = live.iter().map(|t| t.id).collect();
        assert_eq!(ids, vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn dismiss_removes_once() {
        let (center, _) = center();
        let id = center
            .post(ToastKind::Success, None, "gone".to_owned(), None)
            .await;
        assert!(center.dismiss(id).await);
        assert!(center.live().await.is_empty());
        // A second dismissal of the same id is not an error.
        assert!(!center.dismiss(id).await);
    }

    /// The client matches on these strings to pick a toast's colour, and it
    /// lives in another crate — so a rename here is a silent break there.
    #[test]
    fn kinds_serialize_to_the_names_the_client_matches() {
        for (kind, expected) in [
            (ToastKind::Info, "\"info\""),
            (ToastKind::Success, "\"success\""),
            (ToastKind::Warning, "\"warning\""),
            (ToastKind::Error, "\"error\""),
        ] {
            assert_eq!(serde_json::to_string(&kind).unwrap(), expected);
        }
    }

    #[tokio::test]
    async fn capacity_drops_the_oldest() {
        let (center, _) = center();
        for n in 0..=CAPACITY {
            center
                .post(ToastKind::Info, None, n.to_string(), None)
                .await;
        }
        let live = center.live().await;
        assert_eq!(live.len(), CAPACITY);
        // The first one posted is the one that went.
        assert_eq!(live.first().unwrap().body, "1");
        assert_eq!(live.last().unwrap().body, CAPACITY.to_string());
    }
}
