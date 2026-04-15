use leptos::prelude::*;

use super::{button_cta::CtaLink, ButtonKind};

const KIND: ButtonKind = ButtonKind::CreateNewRevision;

#[component]
pub fn CreateNewRevision(
    href: String,
    #[prop(optional, into)] primary: MaybeProp<bool>,
) -> impl IntoView {
    view! {
        <CtaLink href=href icon=KIND.icon() primary=primary>
            {KIND.label()}
        </CtaLink>
    }
}
