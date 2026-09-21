//! Refetch decisions for the watcher's events, shared by the pages that follow
//! them.
//!
//! **Decisions only, never rendering** — so nothing drawn can go stale from what
//! is remembered here. v1 makes the same split for the same reason
//! (`installed_packages_list.rs:170-175`).
//!
//! Shared rather than copied. The main page and the v2 package page both gate a
//! whole-page read on this stream, and the fingerprint rule is the part worth
//! getting right once: the producer is a firehose by design — the watcher
//! re-reports whether or not anything moved — so each consumer decides for
//! itself what counts as news.

use std::collections::HashMap;

use leptos::leptos_dom::helpers::TimeoutHandle;
use leptos::prelude::*;
use quilt_uri::Namespace;

use crate::commands;

/// How long news is allowed to gather before the page asks the backend again.
///
/// A watcher tick reports every package, so news about several arrives as several
/// events a few milliseconds apart. Without a window, each would restart the read
/// the last one began.
pub(super) const STATUS_BURST: std::time::Duration = std::time::Duration::from_millis(250);

#[derive(Clone, Copy)]
pub(super) struct StatusWatch {
    /// The last fingerprint acted on, per namespace.
    seen: StoredValue<HashMap<Namespace, String>>,
    timer: StoredValue<Option<TimeoutHandle>>,
    reload: Trigger,
}

impl StatusWatch {
    pub(super) fn new(reload: Trigger) -> Self {
        let watch = Self {
            seen: StoredValue::new(HashMap::new()),
            timer: StoredValue::new(None),
            reload,
        };
        // The pending window must not outlive the page —
        // `components/set_remote_popup.rs`'s shape for the same hazard.
        on_cleanup(move || {
            if let Some(Some(handle)) = watch.timer.try_get_value() {
                handle.clear();
            }
        });
        watch
    }

    /// Act on one event, if it reports anything the last one for its namespace
    /// did not.
    ///
    /// A namespace never seen counts as news. There is nothing to seed from — the
    /// package payload carries no fingerprint — and a swallowed first sighting
    /// would be exactly the pull that completed while the page was open.
    pub(super) fn observe(self, event: &commands::PackageStatusEvent) {
        let known = self
            .seen
            .with_value(|seen| seen.get(&event.namespace) == Some(&event.fingerprint));
        if known {
            return;
        }
        self.seen.update_value(|seen| {
            seen.insert(event.namespace.clone(), event.fingerprint.clone());
        });
        self.schedule();
    }

    /// Act on something that carries no fingerprint to compare.
    ///
    /// A pause is the case: `status_fingerprint` digests the upstream state and
    /// the changed paths, and a package can pause without either moving — a
    /// workflow rejection, a refused role. So the status stream would report the
    /// same observation, this would call it old news, and the pause would never
    /// reach the page. Coalesced through the same window, because a pause and
    /// the tick that noticed it arrive together.
    pub(super) fn nudge(self) {
        self.schedule();
    }

    fn schedule(self) {
        if let Some(handle) = self.timer.get_value() {
            handle.clear();
        }
        let reload = self.reload;
        if let Ok(handle) = set_timeout_with_handle(move || reload.notify(), STATUS_BURST) {
            self.timer.set_value(Some(handle));
        }
    }
}
