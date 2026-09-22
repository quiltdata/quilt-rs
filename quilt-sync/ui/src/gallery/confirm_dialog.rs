//! `ConfirmDialog` — the confirmation every Danger command is followed by.

use leptos::prelude::*;

use crate::Scene;
use crate::gallery::forms::after_a_beat;
use crate::kit::Banner;
use crate::kit::BannerVariant;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::ConfirmDialog;
use crate::kit::Submit;

/// Names the package: a consequence that does not say *what* is removed is a warning,
/// not a question.
const CONSEQUENCE: &str =
    "Removes user/plate-07 from this machine, including edits never committed.";
/// Says what did not happen and why, as `Submit`'s doc asks of an action's `Err`.
const REFUSAL: &str = "Could not remove user/plate-07: a file in it is open in another program.";

/// The confirmation at rest with a refusal drawn — and the real thing one click away.
///
/// Composed inline for the reason `RefusedScene` is: a modal in the top layer takes the
/// page's pointer events, so a scene holding one open makes the rest of the gallery
/// unclickable. The resting copy is the same kit pieces the dialog uses — `Banner`, the
/// sentence, `Cancel`, the `Danger` verb — and the button beside it opens the real one,
/// whose banner arrives through an actual refused action.
#[component]
pub fn ConfirmScene() -> impl IntoView {
    let live = RwSignal::new(false);
    let dismissed = RwSignal::new(false);

    view! {
        <Scene
            title="Scene · a confirmation"
            note="What a Danger command is followed by: one sentence naming the consequence, \
                  Cancel — where focus lands, and what Escape answers — and the verb on a \
                  Danger button, last. It runs on FormDialog's machinery: open the real one \
                  and press Remove to watch both buttons refuse while it runs, then the \
                  refusal arrive inside the dialog and leave it open. This copy is inline, \
                  at the modal's width, so it can be read and screenshotted."
        >
            <div class="g-bars g-dialog-inline">
                <Show when=move || !dismissed.get()>
                    <Banner
                        variant=BannerVariant::Critical
                        on_dismiss=move |_| dismissed.set(true)
                    >
                        {REFUSAL}
                    </Banner>
                </Show>
                <p class="g-consequence">{CONSEQUENCE}</p>
                // The footer, in the dialog's own arrangement: right-aligned, Cancel first.
                <div class="g-inline g-inline--end">
                    <Button on_click=move |_| ()>"Cancel"</Button>
                    <Button variant=ButtonVariant::Danger on_click=move |_| ()>"Remove"</Button>
                </div>
            </div>
            <div class="g-inline">
                <Button on_click=move |_| live.set(true)>"Open the real one"</Button>
            </div>
            <ConfirmDialog
                open=live
                title="Remove package"
                consequence=CONSEQUENCE
                confirm=Submit::new(
                    "Remove",
                    || async {
                        after_a_beat(700).await;
                        Err(REFUSAL.to_string())
                    },
                )
            />
        </Scene>
    }
}
