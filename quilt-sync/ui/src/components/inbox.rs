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
use crate::commands::ActionNeededEntry;
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

/// What needs action, as the panel leads with it: a line per entry, worded
/// and linked as the tray menu's.
pub fn action_needed_for(set: &[ActionNeededEntry]) -> kit::ActionNeeded {
    kit::ActionNeeded {
        causes: set
            .iter()
            .map(|entry| kit::Cause {
                text: entry.line.clone(),
                href: entry.page.clone(),
            })
            .collect(),
    }
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
/// names one, once it has named an id for [`DWELL_MS`]. The dwell covers the
/// entries there were when it began: a newer entry neither restarts it nor is
/// read by it, and gets a dwell of its own after. `None` stops it, and so does
/// a change of scope. A read that fails, as across a reconnect, starts again.
fn read_on_dwell(due: Memo<Option<(u64, Option<Namespace>)>>) {
    // The running dwell's timer and the scope it reads.
    type Running = (TimeoutHandle, Option<Namespace>);
    let pending: StoredValue<Option<Running>> = StoredValue::new(None);
    // Bumped when a dwell ends, so the next one starts for what is left.
    let ended = RwSignal::new(0_u64);
    let stop = move || {
        if let Some((handle, _)) = pending.try_update_value(Option::take).flatten() {
            handle.clear();
        }
    };
    Effect::new(move |_| {
        ended.track();
        let Some((up_to, namespace)) = due.get() else {
            stop();
            return;
        };
        let same_scope = pending
            .try_with_value(|p| p.as_ref().is_some_and(|(_, scope)| *scope == namespace))
            .unwrap_or(false);
        if same_scope {
            return;
        }
        stop();
        let scope = namespace.clone();
        let read = move || {
            pending.try_set_value(None);
            spawn_local(async move {
                if let Err(err) = commands::mark_read(up_to, namespace).await {
                    leptos::logging::warn!("inbox: could not mark entries read: {err}");
                }
                ended.try_update(|n| *n += 1);
            });
        };
        if let Ok(handle) =
            set_timeout_with_handle(read, std::time::Duration::from_millis(DWELL_MS))
        {
            pending.set_value(Some((handle, scope)));
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
    let follow = move |set: &[ActionNeededEntry]| {
        feed.action_needed.try_set(action_needed_for(set));
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

    /// Each entry leads as the tray menu words it, linking to its fix.
    #[wasm_bindgen_test]
    fn each_entry_is_a_line_as_the_tray_words_it() {
        let set = [ActionNeededEntry {
            subject: commands::ActionNeededSubject::App,
            cause: commands::ActionNeededCause::Credentials,
            since: "1970-01-01T00:00:00Z".into(),
            line: "AWS credentials on this computer aren't usable".into(),
            page: "/".into(),
        }];
        assert_eq!(
            action_needed_for(&set).causes,
            [kit::Cause {
                text: "AWS credentials on this computer aren't usable".into(),
                href: "/".into(),
            }]
        );
    }

    /// An entry that arrives during the dwell does not restart it: what was
    /// on the page when it began is read on time, and only that.
    #[wasm_bindgen_test]
    async fn an_arrival_does_not_restart_the_dwell() {
        use crate::engine::fake::FakeEngine;
        use quilt_ui_kit::test_support::{mount, sleep_ms};

        let engine = FakeEngine::serving();
        let feed = InboxFeed::new();
        let unread = |id| kit::InboxEntry {
            unread: true,
            id,
            ..entry_for(record_entry(RecordKind::Got)).unwrap()
        };
        feed.entries.set(vec![unread(1)]);
        let _el = mount(move || {
            provide_context(feed);
            read_package_on_dwell(Signal::stored("acme/demo".to_string()));
        });

        sleep_ms(1_200).await;
        feed.entries.update(|all| all.push(unread(2)));
        sleep_ms(1_200).await;

        let marks: Vec<_> = engine
            .requests()
            .into_iter()
            .filter(|r| r["cmd"] == "mark_read")
            .map(|r| r["args"]["upTo"].clone())
            .collect();
        assert_eq!(
            marks,
            [serde_json::json!(1)],
            "read on time, up to what was there"
        );
    }
}
