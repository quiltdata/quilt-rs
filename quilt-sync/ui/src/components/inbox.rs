//! The appbar's inbox, fed from the backend: the record of what happened, and
//! the action-needed set the badge counts.
//!
//! [`provide_inbox`] fills the kit's [`InboxFeed`] once, above every page, and
//! the kit's appbar draws the bell from it. Reading takes a dwell: an entry is
//! read once the open panel, or its package's page, has stayed in a visible
//! window for [`DWELL_MS`]. Opening and closing at once reads nothing, and the
//! read covers only the entries there were when the dwell began.

use leptos::prelude::*;
use leptos::task::spawn_local;
use quilt_ui_kit as kit;
use quilt_ui_kit::InboxFeed;
use quilt_uri::Namespace;

use crate::commands;
use crate::commands::ActionNeededCause;
use crate::commands::ActionNeededEntry;
use crate::commands::ActionNeededSubject;
use crate::commands::RecordEntry;
use crate::commands::RecordFileChange;
use crate::commands::RecordKind;
use crate::engine;
use crate::routes;

/// How long the panel or a package page stays open before its entries are
/// read.
pub const DWELL_MS: u64 = 2_000;

/// The kit's entry for one of the record's; `None` for a kind this window
/// does not know.
pub fn entry_for(entry: RecordEntry) -> Option<kit::InboxEntry> {
    let RecordEntry {
        id,
        kind,
        namespace,
        revision,
        files,
        outcome,
        at,
        read,
        installed,
    } = entry;
    let kind = match kind {
        RecordKind::Got => kit::EntryKind::Got,
        RecordKind::Published => kit::EntryKind::Published,
        RecordKind::Uninstalled => kit::EntryKind::Uninstalled,
        RecordKind::Created => kit::EntryKind::Created,
        RecordKind::RemovedRevisions { count } => kit::EntryKind::RemovedRevisions { count },
        RecordKind::Unrecognised => return None,
    };
    let groups: Vec<kit::FileGroup> = files
        .into_iter()
        .filter_map(|group| {
            let change = match group.change {
                RecordFileChange::New => kit::FileChange::New,
                RecordFileChange::NewNotDownloaded => kit::FileChange::NewNotDownloaded,
                RecordFileChange::Updated => kit::FileChange::Updated,
                RecordFileChange::Removed => kit::FileChange::Removed,
                RecordFileChange::Kept => kit::FileChange::Kept,
                RecordFileChange::LeftOut => kit::FileChange::LeftOut,
                RecordFileChange::Unrecognised => return None,
            };
            Some(kit::FileGroup {
                change,
                count: group.count,
                paths: group.paths,
            })
        })
        .collect();
    // The page of a package no longer installed cannot load it.
    let href = if !installed || kind == kit::EntryKind::Uninstalled {
        "/".to_string()
    } else {
        routes::package_page_href(&namespace)
    };
    Some(kit::InboxEntry {
        id,
        kind,
        package: namespace.to_string(),
        revision: revision.map(|r| kit::Revision {
            from: r.from.as_deref().map(short),
            to: short(&r.to),
        }),
        report: if groups.is_empty() {
            kit::FileReport::None
        } else {
            kit::FileReport::Files(groups)
        },
        outcome,
        at: js_sys::Date::parse(&at),
        unread: !read,
        href,
    })
}

fn short(hash: &str) -> String {
    hash.chars().take(7).collect()
}

/// How long a publish has failed when it enters the set, so its run began
/// that long before. The backend's `action_needed::PUBLISH_FAILING_AFTER`.
const PUBLISH_FAILING_AFTER_MS: f64 = 3_600_000.0;

/// What needs action, as the panel leads with it: a count for what the main
/// page's queue shows, and a line with its remedy for each cause it does not.
pub fn action_needed_for(set: &[ActionNeededEntry], now: f64) -> kit::ActionNeeded {
    let mut action_needed = kit::ActionNeeded {
        in_queue: 0,
        queue_href: "/".to_string(),
        elsewhere: Vec::new(),
    };
    for entry in set {
        match (&entry.cause, &entry.subject) {
            (ActionNeededCause::Watcher { .. }, _) => action_needed
                .elsewhere
                .push(kit::Cause::watcher_stopped("/settings")),
            (
                ActionNeededCause::PublishFailing { .. },
                ActionNeededSubject::Package { namespace },
            ) => {
                action_needed.elsewhere.push(kit::Cause::publish_failing(
                    &namespace.to_string(),
                    js_sys::Date::parse(&entry.since) - PUBLISH_FAILING_AFTER_MS,
                    now,
                    routes::package_page_href(namespace),
                ));
            }
            (ActionNeededCause::Unrecognised | ActionNeededCause::PublishFailing { .. }, _) => {}
            _ => action_needed.in_queue += 1,
        }
    }
    action_needed
}

/// Whether the window can be seen: not hidden behind another tab, nor
/// minimized.
fn page_visible() -> Signal<bool> {
    let visible = RwSignal::new(!document().hidden());
    let handle = window_event_listener(leptos::ev::visibilitychange, move |_| {
        visible.set(!document().hidden());
    });
    on_cleanup(move || handle.remove());
    visible.into()
}

/// The newest unread entry `wanted` keeps.
fn newest_unread(feed: InboxFeed, wanted: impl Fn(&kit::InboxEntry) -> bool) -> Option<u64> {
    feed.entries.with(|all| {
        all.iter()
            .filter(|e| e.unread && wanted(e))
            .map(|e| e.id)
            .max()
    })
}

/// Read every entry up to the id `due` names, and `namespace`'s only when it
/// names one, once it has named that id for [`DWELL_MS`]. A newer entry starts
/// the dwell again, and `None` stops it.
fn read_on_dwell(due: Memo<Option<(u64, Option<Namespace>)>>) {
    let pending: StoredValue<Option<TimeoutHandle>> = StoredValue::new(None);
    let stop = move || {
        if let Some(handle) = pending.try_update_value(Option::take).flatten() {
            handle.clear();
        }
    };
    Effect::new(move |_| {
        stop();
        let Some((up_to, namespace)) = due.get() else {
            return;
        };
        let read = move || {
            spawn_local(async move {
                if let Err(err) = commands::mark_read(up_to, namespace).await {
                    leptos::logging::warn!("inbox: could not mark entries read: {err}");
                }
            });
        };
        if let Ok(handle) =
            set_timeout_with_handle(read, std::time::Duration::from_millis(DWELL_MS))
        {
            pending.set_value(Some(handle));
        }
    });
    on_cleanup(stop);
}

/// Provide the inbox's feed to every page below, and keep it filled. Call
/// once, above the router.
pub fn provide_inbox() {
    let feed = InboxFeed::new();
    provide_context(feed);

    // Every read takes a stamp, and only the newest read's answer is drawn.
    let stamp = StoredValue::new(0_u64);
    let load = move || {
        let mine = stamp.try_update_value(|s| {
            *s += 1;
            *s
        });
        spawn_local(async move {
            let Ok(record) = commands::get_record().await else {
                return;
            };
            if mine.is_some() && stamp.try_get_value() == mine {
                feed.entries
                    .try_set(record.into_iter().filter_map(entry_for).collect());
            }
        });
    };
    let record =
        engine::listen_then::<serde::de::IgnoredAny>(commands::RECORD_EVENT, move |_| load(), load);
    // An entry's link depends on whether its package is installed.
    let packages =
        engine::listen::<serde::de::IgnoredAny>(commands::PACKAGES_CHANGED_EVENT, move |_| load());

    // The same for the set: an event is the newest word, and a read is
    // adopted only while nothing has arrived since it was asked for.
    let set_stamp = StoredValue::new(0_u64);
    let bump = move || {
        set_stamp.try_update_value(|s| {
            *s += 1;
            *s
        })
    };
    // Kept as it came, so a failing publish's duration is worked out again
    // each time the panel opens.
    let current = RwSignal::new(Vec::<ActionNeededEntry>::new());
    Effect::new(move |_| {
        feed.open.track();
        let now = js_sys::Date::now();
        current.with(|set| feed.action_needed.set(action_needed_for(set, now)));
    });
    let follow = move |set: &[ActionNeededEntry]| {
        current.try_set(set.to_vec());
    };
    let action_needed = engine::listen_then::<Vec<ActionNeededEntry>>(
        commands::ACTION_NEEDED_EVENT,
        move |set| {
            bump();
            follow(&set);
        },
        move || {
            let mine = bump();
            spawn_local(async move {
                if let Ok(set) = commands::get_action_needed().await
                    && mine.is_some()
                    && set_stamp.try_get_value() == mine
                {
                    follow(&set);
                }
            });
        },
    );
    on_cleanup(move || {
        drop(record);
        drop(packages);
        drop(action_needed);
    });

    let visible = page_visible();
    read_on_dwell(Memo::new(move |_| {
        if !(feed.open.get() && visible.get()) {
            return None;
        }
        newest_unread(feed, |_| true).map(|id| (id, None))
    }));
}

/// Read `namespace`'s entries while its page stays in a visible window. A
/// page with no feed above it, as in the gallery, reads nothing.
pub fn read_package_on_dwell(namespace: Signal<String>) {
    let Some(feed) = use_context::<InboxFeed>() else {
        return;
    };
    let visible = page_visible();
    read_on_dwell(Memo::new(move |_| {
        if !visible.get() {
            return None;
        }
        let package = namespace.get();
        let ns = Namespace::try_from(package.as_str()).ok()?;
        newest_unread(feed, |e| e.package == package).map(|id| (id, Some(ns)))
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen_test::wasm_bindgen_test;

    fn record_entry(kind: RecordKind) -> RecordEntry {
        RecordEntry {
            id: 3,
            kind,
            namespace: ("acme", "demo").into(),
            revision: Some(commands::RecordRevision {
                from: None,
                to: "abcdef0123".into(),
            }),
            files: vec![commands::RecordFileGroup {
                change: RecordFileChange::Kept,
                count: 2,
                paths: vec!["a".into()],
            }],
            outcome: None,
            at: "1970-01-01T00:00:01Z".into(),
            read: false,
            installed: true,
        }
    }

    #[wasm_bindgen_test]
    fn an_entry_links_to_its_package_page_and_keeps_its_files() {
        let entry = entry_for(record_entry(RecordKind::Got)).unwrap();
        assert_eq!(entry.kind, kit::EntryKind::Got);
        assert_eq!(entry.package, "acme/demo");
        assert_eq!(
            entry.href,
            routes::package_page_href(&("acme", "demo").into())
        );
        assert_eq!(entry.revision.unwrap().to, "abcdef0");
        assert!(entry.unread);
        let kit::FileReport::Files(groups) = entry.report else {
            panic!("a file report");
        };
        assert_eq!(groups[0].change, kit::FileChange::Kept);
        assert_eq!(groups[0].count, 2);
    }

    #[wasm_bindgen_test]
    fn an_uninstall_opens_the_main_page() {
        let entry = entry_for(record_entry(RecordKind::Uninstalled)).unwrap();
        assert_eq!(entry.href, "/");
    }

    #[wasm_bindgen_test]
    fn an_entry_of_a_package_no_longer_installed_opens_the_main_page() {
        let entry = entry_for(RecordEntry {
            installed: false,
            ..record_entry(RecordKind::Got)
        })
        .unwrap();
        assert_eq!(entry.href, "/");
    }

    #[wasm_bindgen_test]
    fn an_unknown_kind_is_left_out() {
        assert!(entry_for(record_entry(RecordKind::Unrecognised)).is_none());
    }

    fn needs(subject: ActionNeededSubject, cause: ActionNeededCause) -> ActionNeededEntry {
        ActionNeededEntry {
            subject,
            cause,
            since: "1970-01-01T00:00:00Z".into(),
        }
    }

    /// The queue's causes are counted; the two it does not show get a line
    /// each, with the page that fixes them.
    #[wasm_bindgen_test]
    fn the_queue_is_counted_and_the_rest_named() {
        let package = || ActionNeededSubject::Package {
            namespace: ("acme", "demo").into(),
        };
        let set = [
            needs(package(), ActionNeededCause::SignIn { expired: true }),
            needs(ActionNeededSubject::App, ActionNeededCause::Credentials),
            needs(
                package(),
                ActionNeededCause::PublishFailing {
                    error: "boom".into(),
                },
            ),
            needs(
                ActionNeededSubject::App,
                ActionNeededCause::Watcher {
                    message: "no".into(),
                },
            ),
        ];
        let action_needed = action_needed_for(&set, 2.0 * 3_600_000.0);
        // Its run began an hour before it entered the set, at the epoch.
        assert_eq!(action_needed.in_queue, 2);
        assert_eq!(action_needed.count(), 4);
        let hrefs: Vec<&str> = action_needed
            .elsewhere
            .iter()
            .map(|c| c.href.as_str())
            .collect();
        assert_eq!(
            hrefs,
            [
                routes::package_page_href(&("acme", "demo").into()).as_str(),
                "/settings"
            ]
        );
        assert_eq!(
            action_needed.elsewhere[0].text,
            "acme/demo has failed to publish for 3 hours"
        );
    }
}
