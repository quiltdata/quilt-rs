use leptos::prelude::*;

use super::{ButtonKind, base::cta::CtaLink};

const KIND: ButtonKind = ButtonKind::CreateNewRevision;

#[component]
pub fn CreateNewRevision(
    href: String,
    #[prop(optional, into)] primary: MaybeProp<bool>,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
) -> impl IntoView {
    view! {
        <CtaLink href=href icon=KIND.icon() primary=primary disabled=disabled>
            {KIND.label()}
        </CtaLink>
    }
}
