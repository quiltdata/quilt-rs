//! A package row's autosync line: when it publishes, or why it doesn't.
//!
//! Text, not a `StateLabel`: the row's own label already says what the package
//! holds, and this line says what autosync does about it. A spinner leads it while
//! the package publishes.

use leptos::prelude::*;

use super::Spinner;
use super::state_label::StateTone;

stylance::import_crate_style!(style, "src/kit/autosync_note.module.scss");

#[component]
pub fn AutosyncNote(
    #[prop(into)] words: String,
    /// `Attention` for a retry or a pause; anything else draws muted.
    #[prop(optional)]
    tone: StateTone,
    #[prop(optional)] busy: bool,
) -> impl IntoView {
    let class = match tone {
        StateTone::Attention | StateTone::Danger => format!("{} {}", style::root, style::attention),
        StateTone::Success | StateTone::Neutral => style::root.to_string(),
    };
    view! {
        <span class=class>
            {busy.then(|| view! { <Spinner /> })}
            <span class=style::words>{words}</span>
        </span>
    }
}
