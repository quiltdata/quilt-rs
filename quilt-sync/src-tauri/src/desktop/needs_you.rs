//! What needs the reader, on the desktop: an OS notification for each entry
//! that enters while the window is not in front.
//!
//! Only on entering: an entry that entered while the window was in front
//! posts nothing later, and one that leaves and comes back posts again.

use std::sync::Arc;

use tauri::AppHandle;
use tauri::Manager;
use tauri::async_runtime;
use tauri_plugin_notification::NotificationExt;

use super::MAIN;
use super::show_window;
use crate::engine::navigation;
use crate::engine::needs_you::Entry;
use crate::engine::routes;
use crate::engine::services::Services;
use crate::engine::telemetry::prelude::*;

/// The notice's extra field naming the page a click opens.
const PAGE: &str = "page";

/// Post a notice for each entry that enters, and route a click on one to the
/// page that fixes it.
pub fn spawn(app: &AppHandle, services: &Arc<Services>) -> async_runtime::JoinHandle<()> {
    let clicked = app.clone();
    let _ = app.notification().on_action(move |performed| {
        if performed.action_id() != "tap" {
            return;
        }
        open(
            &clicked,
            performed.notification().and_then(|n| n.extra().get(PAGE)),
        );
    });

    let app = app.clone();
    let services = Arc::clone(services);
    let mut rx = services.watcher.needs_you().subscribe();
    async_runtime::spawn(async move {
        let mut known = rx.borrow_and_update().clone();
        while rx.changed().await.is_ok() {
            let now = rx.borrow_and_update().clone();
            let notify = services.autosync_settings.read().await.notify_needs_you;
            if notify && !in_front(&app) {
                for entry in entered(&known, &now) {
                    post(&app, entry);
                }
            }
            known = now;
        }
    })
}

/// The entries in `now` that `before` did not hold.
fn entered<'a>(before: &[Entry], now: &'a [Entry]) -> impl Iterator<Item = &'a Entry> {
    now.iter().filter(|entry| {
        !before
            .iter()
            .any(|was| was.subject == entry.subject && was.cause.kind() == entry.cause.kind())
    })
}

/// Whether the reader is looking at the window.
fn in_front(app: &AppHandle) -> bool {
    app.get_webview_window(MAIN).is_some_and(|window| {
        window.is_visible().unwrap_or(false)
            && !window.is_minimized().unwrap_or(false)
            && window.is_focused().unwrap_or(false)
    })
}

fn post(app: &AppHandle, entry: &Entry) {
    let (title, body) = entry.notice();
    if let Err(err) = app
        .notification()
        .builder()
        .title(title)
        .body(body)
        .extra(PAGE, routes::page(entry.page()))
        .show()
    {
        warn!("could not post a notification: {err}");
    }
}

/// Show the window on `page`, or as it was when the notice names none.
fn open(app: &AppHandle, page: Option<&serde_json::Value>) {
    show_window(app);
    let Some(services) = app.try_state::<Arc<Services>>() else {
        return;
    };
    let Some(path) = page
        .and_then(serde_json::Value::as_str)
        .and_then(|page| page.parse::<routes::Paths>().ok())
    else {
        return;
    };
    services
        .navigations
        .send(&services.events, navigation::WINDOW, path);
}

#[cfg(test)]
mod tests {
    use chrono::Utc;

    use super::*;
    use crate::engine::needs_you::Cause;
    use crate::engine::needs_you::Subject;

    fn entry(subject: Subject, cause: Cause) -> Entry {
        Entry {
            subject,
            cause,
            since: Utc::now(),
        }
    }

    #[test]
    fn only_an_entry_that_was_not_there_enters() {
        let watcher = entry(
            Subject::App,
            Cause::Watcher {
                message: "a".into(),
            },
        );
        let reworded = entry(
            Subject::App,
            Cause::Watcher {
                message: "b".into(),
            },
        );
        let credentials = entry(Subject::App, Cause::Credentials);
        let before = [watcher];
        let now = [reworded, credentials.clone()];
        assert_eq!(entered(&before, &now).collect::<Vec<_>>(), [&credentials]);
    }

    #[test]
    fn a_notice_page_reads_back_as_its_route() {
        let ns = quilt_uri::Namespace::try_from("a/b").unwrap();
        let page =
            routes::page(entry(Subject::package(&ns), Cause::NoAccess { role: None }).page());
        assert_eq!(
            page.parse::<routes::Paths>().unwrap(),
            routes::Paths::InstalledPackage(ns, routes::EntriesFilter::default())
        );
    }
}
