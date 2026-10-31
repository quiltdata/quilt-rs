//! Sending a page somewhere: after a login, or a deep link.
//!
//! One page goes, not every page: the page a link lands on may run its
//! operation as it opens, as `/remote-package` installs. So the engine keeps
//! the address for the page it is meant for and nudges every page with the
//! `navigate` event. That page asks with `pending_navigation`, at the nudge or
//! the next time it connects, so a window still loading still goes. The engine
//! keeps the address until the page says it went, with `navigation_done`, so a
//! reply lost to a dropped socket is asked for again; the page tells a repeat
//! by its id and doesn't go twice.

use std::collections::HashMap;
use std::sync::Mutex;

use serde::Serialize;

use crate::engine::events::EventSink;
use crate::engine::routes;

/// The nudge every page gets when one of them has somewhere to go.
pub const NAVIGATE_EVENT: &str = "navigate";

/// The desktop window's page id. A browser tab makes up its own.
pub const WINDOW: &str = "window";

/// Somewhere one page is to go.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct Navigation {
    /// Unique across engine restarts, so a page never mistakes a new one for
    /// one it already followed.
    pub id: String,
    /// The address, without the host.
    pub page: String,
}

/// Where each page is to go next, by page id.
#[derive(Default)]
pub struct Navigations {
    pending: Mutex<HashMap<String, Navigation>>,
}

impl Navigations {
    /// Send page `to` to `path`, replacing anywhere it was to go before.
    pub fn send(&self, events: &EventSink, to: &str, path: routes::Paths) {
        let navigation = Navigation {
            id: uuid::Uuid::new_v4().simple().to_string(),
            page: routes::page(path),
        };
        if let Ok(mut pending) = self.pending.lock() {
            pending.insert(to.to_owned(), navigation);
        }
        events.emit(NAVIGATE_EVENT, ());
    }

    /// Where page `page` is to go, until it says it went.
    pub fn pending(&self, page: &str) -> Option<Navigation> {
        self.pending.lock().ok()?.get(page).cloned()
    }

    /// Page `page` followed navigation `id`. A newer one stays.
    pub fn done(&self, page: &str, id: &str) {
        if let Ok(mut pending) = self.pending.lock()
            && pending.get(page).is_some_and(|n| n.id == id)
        {
            pending.remove(page);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn only_the_page_it_was_sent_to_gets_it_until_it_says_it_went() {
        let navigations = Navigations::default();
        let events = EventSink::new();
        let mut nudges = events.subscribe();

        navigations.send(&events, WINDOW, routes::Paths::Settings);
        assert_eq!(
            nudges.recv().await.unwrap(),
            (NAVIGATE_EVENT.to_owned(), serde_json::Value::Null)
        );
        assert_eq!(navigations.pending("a-tab"), None);
        let sent = navigations.pending(WINDOW).unwrap();
        assert_eq!(sent.page, "/settings");
        assert_eq!(
            navigations.pending(WINDOW),
            Some(sent.clone()),
            "asked again, as after a lost reply"
        );
        navigations.done(WINDOW, &sent.id);
        assert_eq!(navigations.pending(WINDOW), None);
    }

    #[test]
    fn done_with_an_old_one_keeps_a_newer_one() {
        let navigations = Navigations::default();
        let events = EventSink::new();
        navigations.send(&events, WINDOW, routes::Paths::Settings);
        let old = navigations.pending(WINDOW).unwrap();
        navigations.send(&events, WINDOW, routes::Paths::Home);
        navigations.done(WINDOW, &old.id);
        assert_eq!(navigations.pending(WINDOW).unwrap().page, "/");
    }
}
