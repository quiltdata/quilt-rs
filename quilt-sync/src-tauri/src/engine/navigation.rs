//! Sending a page somewhere: after a login, or a deep link.
//!
//! One page goes, not every page: the page a link lands on may run its
//! operation as it opens, as `/remote-package` installs. So the engine keeps
//! the address for the page it is meant for and nudges every page with the
//! `navigate` event. That page takes the address with `take_navigation`, at the
//! nudge or the next time it connects, so a window still loading still goes.

use std::collections::HashMap;
use std::sync::Mutex;

use crate::engine::events::EventSink;
use crate::engine::routes;

/// The nudge every page gets when one of them has somewhere to go.
pub const NAVIGATE_EVENT: &str = "navigate";

/// The desktop window's page id. A browser tab makes up its own.
pub const WINDOW: &str = "window";

/// Where each page is to go next, by page id.
#[derive(Default)]
pub struct Navigations {
    pending: Mutex<HashMap<String, String>>,
}

impl Navigations {
    /// Send page `to` to `path`, replacing anywhere it was to go before.
    pub fn send(&self, events: &EventSink, to: &str, path: routes::Paths) {
        let page = routes::page(path);
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(to.to_owned(), page);
        }
        events.emit(NAVIGATE_EVENT, ());
    }

    /// Where page `id` is to go, once.
    pub fn take(&self, id: &str) -> Option<String> {
        self.pending.lock().ok()?.remove(id)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_the_page_it_was_sent_to_takes_it_and_only_once() {
        let navigations = Navigations::default();
        let events = EventSink::new();
        let mut nudges = events.subscribe();

        navigations.send(&events, WINDOW, routes::Paths::Settings);
        assert_eq!(
            nudges.recv().await.unwrap(),
            (NAVIGATE_EVENT.to_owned(), serde_json::Value::Null)
        );
        assert_eq!(navigations.take("a-tab"), None);
        assert_eq!(navigations.take(WINDOW).as_deref(), Some("/settings"));
        assert_eq!(navigations.take(WINDOW), None);
    }
}
