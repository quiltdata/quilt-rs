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

/// The same slot under *individual-file* scope, once nothing is left to fetch.
///
/// Its sibling above states a **rule**; this states a **fact**, and the
/// difference is the point of having both. Under individual-file scope the next
/// revision can add a file and this stops being true — which is exactly the gap
/// the other scope closes. So it reports the present and promises nothing.
pub(super) const ALL_DOWNLOADED_LINE: &str = "All files are downloaded.";

/// What to say once the scope is stored.
///
/// Leads with **saved**, this app's idiom for a written preference, because the
/// failure to avoid is the message reading as *a sync just started*: nothing has
/// been fetched, and the sentence must not imply otherwise. The rest names the
/// durable effect, and — only when there is a backlog to act on — the control
/// that clears it.
#[must_use]
fn scope_saved_message(pending: usize) -> String {
    if pending == 0 {
        "Sync scope saved. New files will be downloaded from now on.".to_owned()
    } else {
        format!(
            "Sync scope saved. New files will be downloaded from now on — press \
             Download all files for the {pending} file{} already listed.",
            if pending == 1 { "" } else { "s" }
        )
    }
}

/// The scope band. `entire_package` is the package's standing choice; picking a
/// scope writes it through and moves no bytes.
#[component]
pub(super) fn SyncScopeBand(
    namespace: String,
    entire_package: bool,
    /// Whether a status banner sits above. Must be the **same** value the
    /// entries toolbar gets: both bands are sticky, and they derive their
    /// offsets from it — if they disagree, whichever sticks higher covers the
    /// other.
    with_status: bool,
    /// Remote entries not yet downloaded, for the message only: what the user
    /// must press the download control for. Zero means the package is already
    /// complete and there is nothing to point them at.
    pending: usize,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    refetch: Trigger,
) -> impl IntoView {
    let band_class = if with_status {
        "qui-entries-toolbar qui-sync-scope with-status"
    } else {
        "qui-entries-toolbar qui-sync-scope"
    };
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
        // Storing the scope is the whole of it. Fetching the backlog is the
        // toolbar's `Download all files` button, one deliberate press away —
        // the scope is a standing choice about future updates, and the bytes
        // already outstanding can be gigabytes. The screen is honest about the
        // gap without this doing it silently: while files are pending the
        // toolbar shows that button, so the state reads "scope on, N files
        // outstanding, press this" rather than a claim with no way to resolve
        // it. Switching *out* likewise touches no files.
        leptos::task::spawn_local(async move {
            let stored = commands::package_set_sync_scope(ns, want_whole).await;
            busy.set(false);
            ui_locked.set(false);
            match stored {
                Ok(()) => {
                    if want_whole {
                        notification.set(Some(Notification::Success(scope_saved_message(pending))));
                    }
                }
                Err(e) => {
                    whole.set(!want_whole);
                    notification.set(Some(Notification::Error(e)));
                }
            }
            refetch.notify();
        });
    });
    let pick_narrow = std::rc::Rc::clone(&on_pick);
    let pick_whole = on_pick;

    view! {
        <div class=band_class>
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
    use super::{ALL_DOWNLOADED_LINE, STANDING_SCOPE_LINE, scope_saved_message};

    /// The page's own source, so the markup rules below are checkable on the
    /// host target where the Leptos view is never rendered.
    const SOURCE: &str = include_str!("sync_scope.rs");

    /// Just the band's markup. Sliced out so the assertions below never read
    /// this test module's own text — counting `type="radio"` across the whole
    /// file counts the assertion that does the counting.
    fn band_markup() -> &'static str {
        let start = SOURCE
            .find("<div class=band_class>")
            .expect("the band still renders its toolbar div");
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

    /// The two captions for the same slot must not be interchangeable: one
    /// promises something about files that do not exist yet, the other reports
    /// the present. Under individual-file scope the next revision can falsify
    /// "all files are downloaded", so it must not borrow the standing line's
    /// forward-looking wording.
    #[test]
    fn the_complete_caption_promises_nothing_about_later_files() {
        assert_ne!(ALL_DOWNLOADED_LINE, STANDING_SCOPE_LINE);
        for word in ["later", "will", "from now on"] {
            assert!(
                !ALL_DOWNLOADED_LINE.to_lowercase().contains(word),
                "{ALL_DOWNLOADED_LINE:?} should not mention {word}"
            );
        }
    }

    /// **The misread this message exists to avoid.** Picking the scope stores a
    /// preference and fetches nothing, so the toast must not read as a sync
    /// having started — no past-tense download claim, and no bare "syncing".
    #[test]
    fn the_saved_message_never_claims_a_download_ran() {
        for pending in [0, 1, 7] {
            let msg = scope_saved_message(pending).to_lowercase();
            assert!(
                msg.starts_with("sync scope saved."),
                "leads with the preference-written idiom: {msg:?}"
            );
            assert!(!msg.contains("downloaded the"), "{msg:?}");
            assert!(!msg.contains("now syncing"), "{msg:?}");
        }
    }

    /// It points at the control only when there is something for it to do, and
    /// counts what that is. Both arms asserted, so this cannot pass by the
    /// branch collapsing to one message.
    #[test]
    fn the_saved_message_names_the_control_only_with_a_backlog() {
        let complete = scope_saved_message(0);
        assert!(
            !complete.contains("Download all files"),
            "nothing to press: {complete:?}"
        );

        let one = scope_saved_message(1);
        assert!(one.contains("Download all files"), "{one:?}");
        assert!(one.contains("1 file already listed"), "singular: {one:?}");

        let many = scope_saved_message(7);
        assert!(many.contains("7 files already listed"), "plural: {many:?}");
    }
}
