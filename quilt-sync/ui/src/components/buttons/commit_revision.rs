use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::{button_cta::ButtonCta, ButtonKind};

const KIND: ButtonKind = ButtonKind::CommitRevision;

#[component]
pub fn CommitRevision(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional, into)] busy: MaybeProp<bool>,
) -> impl IntoView {
    view! {
        <ButtonCta icon=KIND.icon() on_click=UnsyncCallback::new(on_click) primary=true disabled=busy>
            {move || if busy.get().unwrap_or(false) { "Committing\u{2026}" } else { KIND.label() }}
        </ButtonCta>
    }
}
