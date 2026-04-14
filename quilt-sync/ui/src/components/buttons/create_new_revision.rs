use leptos::prelude::*;

use super::{button_cta::ButtonCta, ButtonKind};

const KIND: ButtonKind = ButtonKind::CreateNewRevision;

#[component]
pub fn CreateNewRevision(
    href: String,
    #[prop(optional, into)] primary: MaybeProp<bool>,
) -> impl IntoView {
    view! {
        <ButtonCta href=href icon=KIND.icon() primary=primary>
            {KIND.label()}
        </ButtonCta>
    }
}
