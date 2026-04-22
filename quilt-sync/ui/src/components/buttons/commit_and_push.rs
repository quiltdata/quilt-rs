use leptos::callback::UnsyncCallback;
use leptos::prelude::*;

use super::{base::cta::ButtonCta, ButtonKind};

const KIND: ButtonKind = ButtonKind::Publish;

#[component]
pub fn CommitAndPush(
    on_click: impl Fn(leptos::ev::MouseEvent) + 'static,
    #[prop(optional, into)] busy: MaybeProp<bool>,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
) -> impl IntoView {
    let is_disabled =
        Signal::derive(move || busy.get().unwrap_or(false) || disabled.get().unwrap_or(false));
    view! {
        <ButtonCta icon=KIND.icon() on_click=UnsyncCallback::new(on_click) primary=true disabled=is_disabled>
            {move || if busy.get().unwrap_or(false) { "Committing and pushing\u{2026}" } else { "Commit and Push" }}
        </ButtonCta>
    }
}
