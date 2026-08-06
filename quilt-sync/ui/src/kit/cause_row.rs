//! A cause shared by several packages, with a count and an expander.
//!
//! # One component, not two, and the slot is why
//!
//! It has two appearances — one carrying `[Sign in]`, one carrying a pointer line and
//! no control — and they are the same job: name a cause, say how many packages it
//! holds, let the user see which. What differs is *what sits in the trailing slot*,
//! and that is data.
//!
//! The distinction matters because the kit bans the other kind of flag. `sub` on
//! [`QueueRow`](super::QueueRow) changes indentation; a boolean like
//! "is the row itself clickable" would change the row's **job**, which is why
//! `PackageRow` and `FileRow` stayed separate. A slot that accepts either a button or
//! a sentence changes neither.
//!
//! # Why one of them has no control
//!
//! Role-denied is fixed by switching role, which is host-scoped — so the control
//! belongs to the host row in the Accounts card, and this row points at it. A link
//! asserts nothing about what it governs and may be duplicated across scopes; a
//! control asserts "the fix is here, at this scope", so the same control at two
//! granularities makes one of them a lie.

use leptos::prelude::*;

use super::state_label::StateTone;

stylance::import_crate_style!(style, "src/kit/cause_row.module.scss");

#[component]
pub fn CauseRow(
    /// The cause, in the page's words — `Signed out from custom.registry.io`.
    #[prop(into)]
    text: String,
    /// How many packages this cause holds. Rendered as `— 11 packages`, and singular
    /// at one: a cause affecting one package is still worth stating once rather than
    /// twice.
    count: usize,
    /// Owned by the caller, because the caller renders what the expansion reveals —
    /// its own `QueueRow`s with `sub`. This row owns the control, not the content.
    expanded: RwSignal<bool>,
    /// `Attention` or `Danger`. The other two tones are accepted by the type and
    /// meaningless here: a shared cause is never success, and never merely a number.
    tone: StateTone,
    /// `[Sign in]`, or the line that points at where the fix lives. `AnyView` rather
    /// than `Children`, matching `ToggleRow`'s trailing slot and `EmptyState`'s action
    /// — in this kit `Children` means the one unnamed child, and a named slot is a
    /// view.
    trailing: AnyView,
) -> impl IntoView {
    let class = move || {
        let tone = match tone {
            StateTone::Danger => style::danger,
            _ => style::attention,
        };
        format!("{} {tone}", style::root)
    };

    let packages = if count == 1 {
        "1 package".to_string()
    } else {
        format!("{count} packages")
    };
    // Two closures need it, and neither can move a `String`. The label is generated
    // rather than passed because the count is the only thing that makes it useful:
    // "Show" alone does not say show what.
    let expand_label = {
        let packages = packages.clone();
        move || {
            if expanded.get() {
                "Hide the packages".to_string()
            } else {
                format!("Show the {packages}")
            }
        }
    };

    view! {
        <div class=class>
            // Leading, not trailing. The right edge belongs to actions, and every row
            // in the region has to agree on where that edge is — an expander sitting
            // after the action pushed this row's button 30px left of the five below
            // it. A disclosure control also conventionally starts the thing it
            // expands, as in any file tree, and `QueueRow` reserves the same column
            // so a cause and a package align on their text rather than the parent
            // being indented past its own children.
            //
            // `aria-expanded` and no `aria-controls`: what this reveals is a sibling
            // the caller renders, and we have no id for it. Announcing a control over
            // an element we cannot name would be worse than announcing none.
            <button
                type="button"
                class=style::expander
                aria-expanded=move || expanded.get().to_string()
                title=expand_label.clone()
                aria-label=expand_label
                on:click=move |_| expanded.update(|open| *open = !*open)
            >
                <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
                    stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M4 6.2 8 10.2 12 6.2" />
                </svg>
            </button>
            {tone.glyph()}
            <span class=style::text>
                {text}
                <span class=style::count>{format!(" — {packages}")}</span>
            </span>
            <span class=style::trailing>{trailing}</span>
        </div>
    }
}
