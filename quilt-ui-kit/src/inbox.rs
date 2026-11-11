//! The inbox: what happened while the reader was not looking, behind a bell
//! in the appbar.
//!
//! # What it lists
//!
//! The backend's record, one [`InboxEntry`] per event, newest first and never
//! merged: ten pulls are ten entries. Nothing pops up on its own; an entry
//! lands and the bell's [`Badge`] says so. An entry is one link, to its
//! package's page, and has no other control.
//!
//! # What leads it
//!
//! While anything needs action, the panel leads with live lines drawn
//! from [`ActionNeeded`]: one counting what the main page's queue holds, and one
//! for each cause the queue does not show, with its remedy. They are state,
//! not entries, so they leave when the cause clears.
//!
//! # The bell's badge
//!
//! Two levels. An amber count of what needs action, which drops only when
//! a cause clears, so looking cannot clear it. Otherwise a grey dot while any
//! entry is unread. The count wins when both hold, and the button's name says
//! the same in words.
//!
//! # Unread stays marked while the panel is open
//!
//! An unread entry is highlighted, and one the backend marks read while the
//! panel is open keeps its highlight until the panel closes: the panel
//! remembers every entry it has seen unread since it opened.
//!
//! # On every page
//!
//! The [`Appbar`](crate::Appbar) draws the bell when an [`InboxFeed`] is
//! provided above it, as the activity line follows
//! [`Activities`](crate::Activities): the app fills the feed once, and every
//! page's bar shows it, whatever controls the page puts beside it.
//!
//! Presentation only. The record, reading and the action-needed set are the
//! caller's.

use std::collections::BTreeSet;

use leptos::prelude::*;

use super::Align;
use super::AnchoredOverlay;
use super::countdown::EpochMillis;
use super::state_label::StateTone;

mod entry;

use entry::EntryView;

stylance::import_crate_style!(style, "src/inbox.module.css");

/// What one entry records.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EntryKind {
    /// A pull brought a newer revision.
    Got,
    Published,
    Uninstalled,
    /// Created, from a folder or empty.
    Created,
    RemovedRevisions {
        count: usize,
    },
    /// Another tool on this computer moved the package's revision. `published`
    /// when the new revision is the copy's own commit; otherwise a publish
    /// cannot be told from a pull.
    Outside {
        published: bool,
    },
}

/// One kind of change in a file report.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileChange {
    New,
    NewNotDownloaded,
    Updated,
    Removed,
    /// Removed by the remote, but edited here, so kept and no longer tracked.
    Kept,
    /// Kept out of a created package by ignore rules.
    LeftOut,
}

impl FileChange {
    fn words(self) -> &'static str {
        match self {
            Self::New => "new",
            Self::NewNotDownloaded => "new, not downloaded",
            Self::Updated => "updated",
            Self::Removed => "removed",
            Self::Kept => "kept, removed on the remote",
            Self::LeftOut => "left out by ignore rules",
        }
    }
}

/// The files of one kind of change. `count` is whole; `paths` may hold fewer,
/// as the record keeps only so many.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileGroup {
    pub change: FileChange,
    pub count: usize,
    pub paths: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum FileReport {
    #[default]
    None,
    /// What reached the disk.
    Files(Vec<FileGroup>),
    /// What differs between two installed manifests, for a change another
    /// tool made: that tool's own report of what it wrote is not available.
    Differs(Vec<FileGroup>),
}

/// Short hashes. `from` is absent for a first revision.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revision {
    pub from: Option<String>,
    pub to: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct InboxEntry {
    pub id: u64,
    pub kind: EntryKind,
    /// `owner/name`.
    pub package: String,
    pub revision: Option<Revision>,
    pub report: FileReport,
    /// The sentence its kind reports beyond the files: space freed, files kept
    /// for a busy package, a prune that failed.
    pub outcome: Option<String>,
    pub at: EpochMillis,
    pub unread: bool,
    /// The package page, or the main page for a package no longer installed.
    pub href: String,
}

/// A action-needed cause the main page's queue does not show, with where it is
/// fixed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Cause {
    pub text: String,
    pub href: String,
}

impl Cause {
    /// The filesystem watcher has failed; Settings is where it is seen to.
    #[must_use]
    pub fn watcher_stopped(settings_href: impl Into<String>) -> Self {
        Self {
            text: "File watching stopped".to_string(),
            href: settings_href.into(),
        }
    }

    /// A publish that has kept failing since `since`.
    #[must_use]
    pub fn publish_failing(
        package: &str,
        since: EpochMillis,
        now: EpochMillis,
        href: impl Into<String>,
    ) -> Self {
        Self {
            text: format!(
                "{package} has failed to publish {}",
                for_how_long(now - since)
            ),
            href: href.into(),
        }
    }
}

/// `for an hour`, `for 2 hours`, `for 3 days`. Never under an hour: that is
/// when a failing publish starts to need action.
fn for_how_long(ms: f64) -> String {
    #[expect(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "a whole, non-negative number of hours"
    )]
    let hours = (ms / 3_600_000.0).floor().max(1.0) as u64;
    match hours {
        1 => "for an hour".to_string(),
        2..48 => format!("for {hours} hours"),
        _ => format!("for {} days", hours / 24),
    }
}

/// Everything that needs action now.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ActionNeeded {
    /// How many the main page's queue holds.
    pub in_queue: usize,
    /// The main page, where the queue is.
    pub queue_href: String,
    pub elsewhere: Vec<Cause>,
}

impl ActionNeeded {
    #[must_use]
    pub fn count(&self) -> usize {
        self.in_queue + self.elsewhere.len()
    }
}

/// The bell's mark.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Badge {
    None,
    Unread,
    ActionNeeded(usize),
}

impl Badge {
    #[must_use]
    pub fn of(action_needed: usize, unread: bool) -> Self {
        match (action_needed, unread) {
            (0, false) => Self::None,
            (0, true) => Self::Unread,
            (n, _) => Self::ActionNeeded(n),
        }
    }

    /// The bell's accessible name.
    #[must_use]
    pub fn name(self) -> String {
        match self {
            Self::None => "Inbox".to_string(),
            Self::Unread => "Inbox, unread news".to_string(),
            Self::ActionNeeded(1) => "Inbox, 1 needs action".to_string(),
            Self::ActionNeeded(n) => format!("Inbox, {n} need action"),
        }
    }
}

/// What the appbar's bell draws, provided once above every page. The caller
/// fills `entries` and `action_needed`, and reads `open` to tell when the
/// panel has stayed open long enough to read it.
#[derive(Clone, Copy, Debug)]
pub struct InboxFeed {
    pub entries: RwSignal<Vec<InboxEntry>>,
    pub action_needed: RwSignal<ActionNeeded>,
    pub open: RwSignal<bool>,
}

impl InboxFeed {
    #[must_use]
    pub fn new() -> Self {
        Self {
            entries: RwSignal::new(Vec::new()),
            action_needed: RwSignal::new(ActionNeeded::default()),
            open: RwSignal::new(false),
        }
    }
}

impl Default for InboxFeed {
    fn default() -> Self {
        Self::new()
    }
}

/// The lead line about the queue.
fn queue_line(count: usize) -> String {
    match count {
        1 => "1 thing needs your attention".to_string(),
        n => format!("{n} things need your attention"),
    }
}

/// The bell and the panel it opens.
#[component]
pub fn Inbox(
    #[prop(into)] entries: Signal<Vec<InboxEntry>>,
    #[prop(into)] action_needed: Signal<ActionNeeded>,
    /// The caller's, so it can tell when the panel has stayed open long enough
    /// to read it.
    open: RwSignal<bool>,
) -> impl IntoView {
    let badge = Signal::derive(move || {
        let unread = entries.with(|all| all.iter().any(|e| e.unread));
        Badge::of(action_needed.with(ActionNeeded::count), unread)
    });

    let trigger = move |surface_id: String| {
        view! {
            <button
                type="button"
                class=style::bell
                aria-label=move || badge.get().name()
                aria-expanded=move || open.get().to_string()
                aria-controls=surface_id
                on:click=move |_| open.update(|o| *o = !*o)
            >
                {bell_glyph()}
                {move || match badge.get() {
                    Badge::None => None,
                    Badge::Unread => {
                        Some(view! { <span class=style::dot aria-hidden="true"></span> }.into_any())
                    }
                    Badge::ActionNeeded(n) => Some(
                        view! { <span class=style::count aria-hidden="true">{n}</span> }
                            .into_any(),
                    ),
                }}
            </button>
        }
        .into_any()
    };

    view! {
        <AnchoredOverlay
            trigger=trigger
            open=open
            aria_label="Inbox"
            align=Align::End
            tight=true
            contained=true
        >
            <InboxPanel entries=entries action_needed=action_needed open=open />
        </AnchoredOverlay>
    }
}

/// What the panel holds. Its own component so the gallery can draw it in the
/// page, where it can be read and screenshotted without opening the popover.
#[component]
pub fn InboxPanel(
    #[prop(into)] entries: Signal<Vec<InboxEntry>>,
    #[prop(into)] action_needed: Signal<ActionNeeded>,
    /// Whether the panel shows. Absent, it always does.
    #[prop(optional, into)]
    open: Option<Signal<bool>>,
) -> impl IntoView {
    let open = open.unwrap_or_else(|| Signal::stored(true));
    // Every entry seen unread since the panel opened.
    let marked = RwSignal::new(BTreeSet::<u64>::new());
    // Counts openings, so each one redraws the entries' ages.
    let openings = RwSignal::new(0_u64);
    Effect::new(move |was_open: Option<bool>| {
        let now = open.get();
        if now && was_open == Some(false) {
            openings.update(|n| *n += 1);
        }
        now
    });
    Effect::new(move |_| {
        if open.get() {
            let unread: Vec<u64> =
                entries.with(|all| all.iter().filter(|e| e.unread).map(|e| e.id).collect());
            marked.update(|m| m.extend(unread));
        } else if !marked.with_untracked(BTreeSet::is_empty) {
            marked.set(BTreeSet::new());
        }
    });

    // Newest first, whatever order they arrive in.
    let ordered = move || {
        let mut all = entries.get();
        all.sort_by(|a, b| b.at.total_cmp(&a.at));
        all
    };

    let lead = move || {
        let ActionNeeded {
            in_queue,
            queue_href,
            elsewhere,
        } = action_needed.get();
        let queue = (in_queue > 0).then(|| (queue_line(in_queue), queue_href));
        let lines: Vec<(String, String)> = queue
            .into_iter()
            .chain(elsewhere.into_iter().map(|c| (c.text, c.href)))
            .collect();
        (!lines.is_empty()).then(|| {
            let items = lines
                .into_iter()
                .map(|(text, href)| {
                    view! {
                        <li>
                            <a class=style::needs href=href>
                                {StateTone::Attention.glyph()}
                                <span>{text}</span>
                            </a>
                        </li>
                    }
                })
                .collect_view();
            view! { <ul class=style::lead aria-label="Action needed">{items}</ul> }
        })
    };

    view! {
        <div class=style::panel>
            {lead}
            <Show
                when=move || entries.with(|all| !all.is_empty())
                fallback=|| view! { <p class=style::empty>"Nothing new"</p> }
            >
                <ol class=style::entries>
                    // Keyed by id and link, not the read flag, so a row read
                    // while focused keeps its link, and one whose package went
                    // takes its new one.
                    <For each=ordered key=|entry| (entry.id, entry.href.clone()) let:entry>
                        {
                            let id = entry.id;
                            let highlighted = Signal::derive(move || {
                                marked.with(|m| m.contains(&id))
                                    || entries.with(|all| all.iter().any(|e| e.id == id && e.unread))
                            });
                            view! {
                                <li>
                                    <EntryView entry=entry highlighted=highlighted openings=openings />
                                </li>
                            }
                        }
                    </For>
                </ol>
            </Show>
        </div>
    }
}

/// Octicons `bell-16`, MIT License, Copyright (c) GitHub Inc. — see `icons.rs`.
fn bell_glyph() -> AnyView {
    view! {
        <svg viewBox="0 0 16 16" aria-hidden="true" fill="currentColor">
            <path d="M8 16a2 2 0 0 0 1.985-1.75c.017-.137-.097-.25-.235-.25h-3.5c-.138 0-.252.113-.235.25A2 2 0 0 0 8 16ZM3 5a5 5 0 0 1 10 0v2.947c0 .05.015.098.042.139l1.703 2.555A1.519 1.519 0 0 1 13.482 13H2.518a1.516 1.516 0 0 1-1.263-2.36l1.703-2.554A.255.255 0 0 0 3 7.947Zm5-3.5A3.5 3.5 0 0 0 4.5 5v2.947c0 .346-.102.683-.294.97l-1.703 2.556a.017.017 0 0 0-.003.01l.001.006c0 .002.002.004.004.006l.006.004.007.001h10.964l.007-.001.006-.004.004-.006.001-.007a.017.017 0 0 0-.003-.01l-1.703-2.554a1.745 1.745 0 0 1-.294-.97V5A3.5 3.5 0 0 0 8 1.5Z" />
        </svg>
    }
    .into_any()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use crate::test_support::sleep_ms;
    use wasm_bindgen_test::*;

    fn entry(id: u64, at: EpochMillis, unread: bool) -> InboxEntry {
        InboxEntry {
            id,
            kind: EntryKind::Got,
            package: format!("user/p{id}"),
            revision: None,
            report: FileReport::Files(vec![FileGroup {
                change: FileChange::New,
                count: 5,
                paths: vec![
                    "a.csv".into(),
                    "b.csv".into(),
                    "c.csv".into(),
                    "d.csv".into(),
                ],
            }]),
            outcome: None,
            at,
            unread,
            href: format!("/p{id}"),
        }
    }

    fn queue(n: usize) -> ActionNeeded {
        ActionNeeded {
            in_queue: n,
            queue_href: "/".to_string(),
            elsewhere: Vec::new(),
        }
    }

    #[test]
    fn the_count_wins_and_the_name_says_it() {
        assert_eq!(Badge::of(0, false).name(), "Inbox");
        assert_eq!(Badge::of(0, true).name(), "Inbox, unread news");
        assert_eq!(Badge::of(1, false).name(), "Inbox, 1 needs action");
        assert_eq!(Badge::of(2, true), Badge::ActionNeeded(2));
        assert_eq!(Badge::of(2, true).name(), "Inbox, 2 need action");
    }

    #[test]
    fn a_failing_publish_says_for_how_long() {
        let now = 100.0 * 3_600_000.0;
        let line = |hours: f64| {
            Cause::publish_failing("user/plate-07", now - hours * 3_600_000.0, now, "/").text
        };
        assert_eq!(line(1.2), "user/plate-07 has failed to publish for an hour");
        assert_eq!(line(2.5), "user/plate-07 has failed to publish for 2 hours");
        assert_eq!(line(72.0), "user/plate-07 has failed to publish for 3 days");
    }

    fn hrefs(el: &web_sys::Element, selector: &str) -> Vec<String> {
        let all = el.query_selector_all(selector).unwrap();
        (0..all.length())
            .filter_map(|i| {
                wasm_bindgen::JsCast::dyn_into::<web_sys::Element>(all.item(i)?)
                    .ok()?
                    .get_attribute("href")
            })
            .collect()
    }

    /// A package uninstalled elsewhere changes an entry's link, not its id.
    #[wasm_bindgen_test]
    async fn an_entry_takes_its_new_link() {
        let entries = RwSignal::new(vec![entry(1, 10.0, true)]);
        let el = mount(move || {
            view! { <InboxPanel entries=entries action_needed=ActionNeeded::default() /> }
        });
        entries.update(|all| all[0].href = "/".to_string());
        sleep_ms(0).await;
        assert_eq!(hrefs(&el, "ol a"), ["/"]);
    }

    #[wasm_bindgen_test]
    fn entries_are_links_newest_first() {
        let el = mount(|| {
            view! {
                <InboxPanel
                    entries=vec![entry(1, 10.0, false), entry(2, 30.0, false), entry(3, 20.0, false)]
                    action_needed=ActionNeeded::default()
                />
            }
        });
        assert_eq!(hrefs(&el, "ol a"), ["/p2", "/p3", "/p1"]);
        assert!(
            el.query_selector("ol button").unwrap().is_none(),
            "and nothing else"
        );
        let text = el.text_content().unwrap_or_default();
        assert!(
            text.contains("5 files new"),
            "the heading counts the group: {text}"
        );
        assert!(
            text.contains("and 2 more"),
            "three named, the rest stated: {text}"
        );
    }

    #[wasm_bindgen_test]
    fn with_no_entries_it_says_nothing_new() {
        let el = mount(|| view! { <InboxPanel entries=Vec::new() action_needed=queue(2) /> });
        let text = el.text_content().unwrap_or_default();
        assert!(text.contains("Nothing new"), "{text}");
        assert!(text.contains("2 things need your attention"), "{text}");
    }

    /// The whole line is the link, not only a word in it.
    #[wasm_bindgen_test]
    fn the_queue_line_is_one_link_to_the_main_page() {
        let el = mount(|| view! { <InboxPanel entries=Vec::new() action_needed=queue(1) /> });
        let link = el
            .query_selector("ul[aria-label='Action needed'] a")
            .unwrap()
            .expect("the line");
        assert_eq!(link.get_attribute("href").as_deref(), Some("/"));
        assert_eq!(
            link.text_content().as_deref(),
            Some("1 thing needs your attention")
        );
    }

    #[wasm_bindgen_test]
    fn the_causes_the_queue_does_not_show_lead_with_their_remedy() {
        let el = mount(|| {
            view! {
                <InboxPanel
                    entries=Vec::new()
                    action_needed=ActionNeeded {
                        in_queue: 1,
                        queue_href: "/".to_string(),
                        elsewhere: vec![Cause::watcher_stopped("/settings")],
                    }
                />
            }
        });
        assert_eq!(
            hrefs(&el, "ul[aria-label='Action needed'] a"),
            ["/", "/settings"]
        );
    }

    /// The backend reads the entries while the panel is open; the reader still
    /// sees which ones were new until the panel closes.
    #[wasm_bindgen_test]
    async fn an_entry_read_while_open_stays_marked_until_the_panel_closes() {
        let entries = RwSignal::new(vec![entry(1, 10.0, true), entry(2, 5.0, false)]);
        let open = RwSignal::new(false);
        let el = mount(move || {
            view! { <InboxPanel entries=entries action_needed=ActionNeeded::default() open=open /> }
        });
        let marked = || {
            let all = el.query_selector_all("ol a").unwrap();
            (0..all.length())
                .map(|i| {
                    all.item(i)
                        .and_then(|n| n.text_content())
                        .is_some_and(|t| t.contains("Unread"))
                })
                .collect::<Vec<_>>()
        };
        open.set(true);
        sleep_ms(0).await;
        assert_eq!(marked(), [true, false]);

        entries.update(|all| all.iter_mut().for_each(|e| e.unread = false));
        sleep_ms(0).await;
        assert_eq!(marked(), [true, false], "still marked while open");

        entries.update(|all| all.push(entry(3, 20.0, true)));
        sleep_ms(0).await;
        entries.update(|all| all.iter_mut().for_each(|e| e.unread = false));
        sleep_ms(0).await;
        assert_eq!(
            marked(),
            [true, true, false],
            "and so is one that arrived while it was open"
        );

        open.set(false);
        sleep_ms(0).await;
        open.set(true);
        sleep_ms(0).await;
        assert_eq!(
            marked(),
            [false, false, false],
            "and read on the next opening"
        );
    }

    /// A reader focused on an entry keeps their place when it is read.
    #[wasm_bindgen_test]
    async fn reading_an_entry_keeps_its_link() {
        let entries = RwSignal::new(vec![entry(1, 10.0, true)]);
        let el = mount(move || {
            view! { <InboxPanel entries=entries action_needed=ActionNeeded::default() /> }
        });
        sleep_ms(0).await;
        let before = el.query_selector("ol a").unwrap().expect("the entry");
        entries.update(|all| all.iter_mut().for_each(|e| e.unread = false));
        sleep_ms(0).await;
        let after = el.query_selector("ol a").unwrap().expect("the entry");
        assert!(before.is_same_node(Some(&after)), "the link was replaced");
    }

    /// The panel keeps its entries between openings; their ages are redrawn.
    #[wasm_bindgen_test]
    async fn each_opening_redraws_the_ages() {
        let open = RwSignal::new(true);
        // Just short of where *just now* becomes *1 min ago*.
        let at = js_sys::Date::now() - 44_800.0;
        let el = mount(move || {
            view! {
                <InboxPanel
                    entries=vec![entry(1, at, false)]
                    action_needed=ActionNeeded::default()
                    open=open
                />
            }
        });
        sleep_ms(0).await;
        let age = || {
            el.query_selector("ol time")
                .unwrap()
                .and_then(|t| t.text_content())
                .unwrap_or_default()
        };
        let link = el.query_selector("ol a").unwrap().expect("the entry");
        assert_eq!(age(), "just now");
        open.set(false);
        sleep_ms(400).await;
        open.set(true);
        sleep_ms(0).await;
        assert_eq!(age(), "1 min ago", "the age was redrawn");
        let after = el.query_selector("ol a").unwrap().expect("the entry");
        assert!(link.is_same_node(Some(&after)), "inside the same link");
    }

    #[wasm_bindgen_test]
    fn the_bell_is_named_by_its_badge() {
        let el = mount(|| {
            view! {
                <Inbox
                    entries=vec![entry(1, 10.0, true)]
                    action_needed=queue(3)
                    open=RwSignal::new(false)
                />
            }
        });
        let bell = el.query_selector("button").unwrap().expect("the bell");
        assert_eq!(
            bell.get_attribute("aria-label").as_deref(),
            Some("Inbox, 3 need action")
        );
        assert_eq!(
            bell.get_attribute("aria-expanded").as_deref(),
            Some("false")
        );
        assert_eq!(bell.text_content().as_deref(), Some("3"));
    }
}
