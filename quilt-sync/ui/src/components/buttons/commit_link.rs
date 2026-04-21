use leptos::prelude::*;

use super::{ButtonKind, IconLink};

const KIND: ButtonKind = ButtonKind::CommitRevision;

#[component]
pub fn CommitLink(namespace: String, #[prop(optional)] small: bool) -> impl IntoView {
    let href = format!("/commit?namespace={}", urlencoding::encode(&namespace));

    view! {
        <IconLink href=href icon=KIND.icon() small=small link=true>
            "Commit\u{2026}"
        </IconLink>
    }
}
