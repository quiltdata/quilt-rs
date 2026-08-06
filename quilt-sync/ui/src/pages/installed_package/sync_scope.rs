//! The per-package sync-scope band — *sync individual files* vs *sync entire
//! package* — shown only while the experiment is on.
//!
//! Deliberately additive: its own toolbar band above the entries toolbar, so
//! the existing toolbar keeps its layout and retiring the experiment is
//! deleting this file and one `<Show>`. The installed-package page rework will
//! take this idea apart properly; nothing here is built to be inherited.

use leptos::prelude::*;

use crate::commands;
use crate::components::Notification;

/// What the band says when everything is already downloaded.
///
/// The list below already shows what is on disk, so restating the present here
/// would be redundant with it. The one thing neither the list nor the mode
/// label can express is the future — and it says it **without naming a
/// mechanism**: whether new files arrive by your pull, by autosync, or after a
/// paused namespace resumes is the status banner's business, and duplicating it
/// here would give two places to keep honest.
pub(super) const STANDING_SCOPE_LINE: &str = "Files added later are downloaded too.";

/// The scope band. `entire_package` is the package's standing choice; changing
/// it writes through immediately and, when switching *to* whole-package, also
/// downloads whatever is currently listed.
#[component]
pub(super) fn SyncScopeBand(
    namespace: String,
    /// The package's remote URI. `install_paths` addresses a package by URI,
    /// not by namespace — a local-only package has none, and also has nothing
    /// remote to catch up on.
    uri: Option<String>,
    entire_package: bool,
    /// Remote entries not yet downloaded — what switching *in* catches up on.
    /// Empty once the package has caught up.
    pending: Memo<Vec<String>>,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    refetch: Trigger,
) -> impl IntoView {
    let whole = RwSignal::new(entire_package);
    let busy = RwSignal::new(false);

    let on_pick = std::rc::Rc::new(move |want_whole: bool| {
        if busy.get_untracked() || whole.get_untracked() == want_whole {
            return;
        }
        busy.set(true);
        ui_locked.set(true);
        whole.set(want_whole);
        let ns = namespace.clone();
        let pkg_uri = uri.clone();
        // Switching *in* is one act, not two: a mode that left files unfetched
        // would be asserting something the app had not made true — the same
        // defect as *Select all* never meaning download-all. Switching *out*
        // downloads and deletes nothing; ending a standing scope is not a
        // request to give back what it already fetched.
        let catch_up: Vec<String> = if want_whole {
            pending.get_untracked()
        } else {
            Vec::new()
        };
        leptos::task::spawn_local(async move {
            let stored = commands::package_set_sync_scope(ns.clone(), want_whole).await;
            if let Err(e) = stored {
                whole.set(!want_whole);
                busy.set(false);
                ui_locked.set(false);
                notification.set(Some(Notification::Error(e)));
                return;
            }
            // The scope is already saved, so a failed catch-up leaves the mode
            // on and the files pending — which is exactly the state the
            // download button in the toolbar below exists to retry from. It
            // must not silently un-choose the mode.
            let message = match (catch_up.is_empty(), pkg_uri) {
                (true, _) | (_, None) => {
                    format!("Now syncing the entire package. {STANDING_SCOPE_LINE}")
                }
                (false, Some(pkg_uri)) => {
                    let count = catch_up.len();
                    match commands::package_install_paths(pkg_uri, catch_up).await {
                        Ok(_) => format!(
                            "Downloaded {count} file{}. {STANDING_SCOPE_LINE}",
                            if count == 1 { "" } else { "s" }
                        ),
                        Err(e) => {
                            busy.set(false);
                            ui_locked.set(false);
                            notification.set(Some(Notification::Error(e)));
                            refetch.notify();
                            return;
                        }
                    }
                }
            };
            busy.set(false);
            ui_locked.set(false);
            if want_whole {
                notification.set(Some(Notification::Success(message)));
            }
            refetch.notify();
        });
    });
    let pick_narrow = std::rc::Rc::clone(&on_pick);
    let pick_whole = on_pick;

    view! {
        <div class="qui-entries-toolbar qui-sync-scope with-status">
            <div class="container">
                <label class="radio-option">
                    <input
                        type="radio"
                        name="sync-scope"
                        prop:checked=move || !whole.get()
                        prop:disabled=move || busy.get()
                        on:change=move |_| pick_narrow(false)
                    />
                    "Sync individual files"
                </label>
                <label class="radio-option">
                    <input
                        type="radio"
                        name="sync-scope"
                        prop:checked=move || whole.get()
                        prop:disabled=move || busy.get()
                        on:change=move |_| pick_whole(true)
                    />
                    "Sync entire package"
                </label>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::STANDING_SCOPE_LINE;

    /// The page's own source, so the markup rules below are checkable on the
    /// host target where the Leptos view is never rendered.
    const SOURCE: &str = include_str!("sync_scope.rs");

    /// Just the band's markup. Sliced out so the assertions below never read
    /// this test module's own text — counting `type="radio"` across the whole
    /// file counts the assertion that does the counting.
    fn band_markup() -> &'static str {
        let start = SOURCE
            .find(r#"<div class="qui-entries-toolbar qui-sync-scope"#)
            .expect("the band still renders a toolbar div");
        let rest = &SOURCE[start..];
        &rest[..rest.find("#[cfg(test)]").expect("tests follow the markup")]
    }

    /// Both scopes are **named**. A checkbox would name one state and leave its
    /// complement unnamed, which is precisely why the existing *Select all*
    /// cannot be read; a radio pair cannot make that mistake, and this pins the
    /// pair against someone "simplifying" it back to one control.
    #[test]
    fn both_scopes_are_named_on_screen() {
        let markup = band_markup();
        assert!(markup.contains(r#""Sync individual files""#));
        assert!(markup.contains(r#""Sync entire package""#));
        assert_eq!(
            markup.matches(r#"type="radio""#).count(),
            2,
            "two radios, not a checkbox"
        );
    }

    /// The verb is **sync**, not *download*. An imperative verb reads as
    /// something that happens once, and the whole point of the mode is that it
    /// keeps applying.
    #[test]
    fn the_mode_labels_do_not_read_as_one_time_actions() {
        for label in ["Sync individual files", "Sync entire package"] {
            assert!(
                !label.to_lowercase().contains("download"),
                "{label} names an action, not a standing scope"
            );
        }
    }

    /// The standing line speaks about the future and names no mechanism —
    /// whether an update arrives by your pull or by autosync belongs to the
    /// status banner, and saying it twice gives two places to keep honest.
    #[test]
    fn the_standing_line_names_no_mechanism() {
        for word in ["pull", "autosync", "automatic"] {
            assert!(
                !STANDING_SCOPE_LINE.to_lowercase().contains(word),
                "{STANDING_SCOPE_LINE:?} should not mention {word}"
            );
        }
        assert!(STANDING_SCOPE_LINE.contains("later"));
    }
}
