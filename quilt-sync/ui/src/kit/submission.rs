//! The machinery a dialog that owns its submission runs on — shared by
//! [`FormDialog`](super::FormDialog) and [`ConfirmDialog`](super::ConfirmDialog).
//!
//! `FormDialog`'s module doc argues each rule: success closes and failure stays open with
//! the reason; the seal while the action runs; an outcome belonging to the session that
//! asked for it and to no other. This module holds the code for them, so a confirmation —
//! the same rules over one sentence instead of fields — is not a second copy that drifts.
//! Private to the kit: the two dialogs are its callers, and nothing else has a submission
//! to own.

use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;

use leptos::prelude::*;

use super::Banner;
use super::BannerVariant;
use super::readable;

/// The caller's action, boxed so a component's signature does not carry its future's
/// type. `Rc` and not `Arc`: this is a single-threaded wasm document, and the event
/// handler only needs to clone it.
pub(super) type Action = Rc<dyn Fn() -> Pin<Box<dyn Future<Output = Result<(), String>>>>>;

/// What a dialog's primary is called and what it does — `FormDialog`'s submit, or the
/// verb on a `ConfirmDialog`.
///
/// The two travel together because neither is useful alone: a label with no action is a
/// button that lies, and an action with no label has nothing to draw.
#[derive(Clone)]
pub struct Submit {
    pub(super) label: String,
    pub(super) action: Action,
}

impl Submit {
    /// `label` is the verb on the primary — `Save`, `Create`, `Remove`.
    ///
    /// `action` runs on submit and its `Err` becomes the dialog's banner, so its message is
    /// read by a user rather than a log: it says what did not happen, not which layer
    /// refused.
    #[must_use]
    pub fn new<F, Fut>(label: impl Into<String>, action: F) -> Self
    where
        F: Fn() -> Fut + 'static,
        Fut: Future<Output = Result<(), String>> + 'static,
    {
        Self {
            label: label.into(),
            action: Rc::new(move || Box::pin(action())),
        }
    }
}

/// One dialog's in-flight state, bound to its `open`. `Copy` because it is signals, so a
/// click handler and the spawned task can each hold one.
#[derive(Clone, Copy)]
pub(super) struct Submission {
    open: RwSignal<bool>,
    submitting: RwSignal<bool>,
    /// `submitting`, or the caller's `running`, as the `Signal` that `MaybeProp` props
    /// take. Three props read it — `disabled` on Cancel, `loading` on the primary, `held`
    /// on the dialog — and they are one fact.
    pub(super) busy: Signal<bool>,
    error: RwSignal<Option<String>>,
    /// Which opening is current. An action carries the one it was asked under, and an
    /// outcome from any other is discarded.
    session: RwSignal<usize>,
}

impl Submission {
    /// `running` is the caller's own in-flight state, which outlives this dialog: a
    /// dialog rebuilt while its predecessor's action still runs starts with none of its
    /// own, and without the caller's it would accept a second submit.
    pub(super) fn new(open: RwSignal<bool>, running: MaybeProp<bool>) -> Self {
        let submitting = RwSignal::new(false);
        let error = RwSignal::new(None::<String>);
        let session = RwSignal::new(0_usize);

        // Every open and every close starts a new session and drops the last one's
        // in-flight state, so a dialog reopened never arrives sealed or carrying the
        // previous answer.
        // Only a real transition counts. An effect's first run lands after the first
        // render, so bumping there would invalidate a submit made in between — including,
        // in a test, one issued the moment the component mounted.
        Effect::new(move |previous: Option<bool>| {
            let now = open.get();
            if previous.is_some_and(|was| was != now) {
                session.update(|n| *n += 1);
                submitting.set(false);
                error.set(None);
            }
            now
        });

        Self {
            open,
            submitting,
            busy: Signal::derive(move || submitting.get() || running.get().unwrap_or(false)),
            error,
            session,
        }
    }

    /// Runs `action` under the current session. `Ok` closes the dialog — the caller's
    /// reload happens inside the action, before it returns; `Err` becomes the banner. A
    /// second call while one is in flight — this dialog's or the caller's — is dropped
    /// rather than queued.
    pub(super) fn run(self, action: &Action) {
        if self.busy.get_untracked() {
            return;
        }
        self.submitting.set(true);
        self.error.set(None);
        let mine = self.session.get_untracked();
        let action = Rc::clone(action);
        leptos::task::spawn_local(async move {
            let outcome = action().await;
            // The dialog was unmounted while this ran, and its session, seal and banner
            // went with it. `open` is the caller's and may have outlived them — a page
            // that rebuilds its dialogs over one flag — so a success still closes it; a
            // refusal has nowhere left to be drawn.
            let Some(current) = self.session.try_get_untracked() else {
                if outcome.is_ok() {
                    self.open.try_set(false);
                }
                return;
            };
            // Closed and reopened while this ran, so it answers a question nobody is
            // asking any more. Touching anything here would be this outcome editing
            // somebody else's dialog.
            if current != mine {
                return;
            }
            self.submitting.set(false);
            match outcome {
                Ok(()) => self.open.set(false),
                Err(message) => self.error.set(Some(message)),
            }
        });
    }

    /// The refusal, drawn where the reader is looking: a `Critical` banner inside the
    /// dialog, dismissable, and gone on the next submit or the next opening.
    pub(super) fn banner(self) -> impl IntoView {
        let error = self.error;
        view! {
            <Show when=move || error.get().is_some()>
                <Banner variant=BannerVariant::Critical on_dismiss=move |_| error.set(None)>
                    {move || error.get().map(|e| readable(&e)).unwrap_or_default()}
                </Banner>
            </Show>
        }
    }
}
