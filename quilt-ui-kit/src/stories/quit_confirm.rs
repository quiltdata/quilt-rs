//! `QuitConfirm` — the quit prompt as the kit's confirmation.

use leptos::prelude::*;

use crate::Button;
use crate::ButtonVariant;
use crate::QuitConfirm;
use crate::quit_confirm::QUIT_CONSEQUENCE;
use crate::story::Scene;

/// At rest inline, and the real one a click away, for the reason
/// `ConfirmScene` gives: a modal in the top layer takes the page's pointer.
#[component]
pub fn QuitScene() -> impl IntoView {
    let live = RwSignal::new(false);
    let answer = RwSignal::new("");

    view! {
        <Scene
            title="Scene · the quit prompt"
            note="What a quit during an apply asks, as a confirmation: one sentence, Stay — \
                  where focus lands — and Quit anyway on the Danger button, last. Escape \
                  answers Stay here, and so does every close that is not the verb; the \
                  backdrop closes nothing. Open the real one and answer it."
        >
            <div class="g-bars g-dialog-inline">
                <p class="g-consequence">{QUIT_CONSEQUENCE}</p>
                <div class="g-inline g-inline--end">
                    <Button on_click=move |_| ()>"Stay"</Button>
                    <Button variant=ButtonVariant::Danger on_click=move |_| ()>
                        "Quit anyway"
                    </Button>
                </div>
            </div>
            <div class="g-inline">
                <Button on_click=move |_| {
                    answer.set("");
                    live.set(true);
                }>"Open the real one"</Button>
                <span>{move || answer.get()}</span>
            </div>
            <QuitConfirm
                open=live
                on_quit=move |()| answer.set("Answered: Quit anyway")
                on_stay=move |()| answer.set("Answered: Stay")
            />
        </Scene>
    }
}
