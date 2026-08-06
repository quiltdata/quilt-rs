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
//! # No tone, and that is a loss worth naming
//!
//! There is no `tone` prop. The row used to carry the tone's silhouette as a leading
//! glyph, and removing that glyph left nothing for a tone to colour — the text stays
//! at full contrast deliberately, because a cause is the most important line in the
//! region and tinting it would make it recede. So an `Attention` cause and a `Danger`
//! one now look identical, and the words are the only thing distinguishing them.
//!
//! That is consistent with the kit's rule that the words are the meaning and the tone
//! is only emphasis, but it does mean this region has no severity channel at all. If
//! one is wanted back, the cheap options are a left stripe on the row or tinting the
//! count — not a leading glyph, which is what was just removed.
//!
//! # Why one of them has no control
//!
//! Role-denied is fixed by switching role, which is host-scoped — so the control
//! belongs to the host row in the Accounts card, and this row points at it. A link
//! asserts nothing about what it governs and may be duplicated across scopes; a
//! control asserts "the fix is here, at this scope", so the same control at two
//! granularities makes one of them a lie.

use leptos::prelude::*;

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
    /// `[Sign in]`, or the line that points at where the fix lives. `AnyView` rather
    /// than `Children`, matching `ToggleRow`'s trailing slot and `EmptyState`'s action
    /// — in this kit `Children` means the one unnamed child, and a named slot is a
    /// view.
    trailing: AnyView,
) -> impl IntoView {
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
        <div class=style::root>
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
                // Points RIGHT at rest and rotates to point down when open, which is
                // the direction a disclosure control conventionally reads: sideways
                // for "there is more this way", down for "it is below you now". A
                // chevron that starts downward is already claiming to be open.
                <svg viewBox="0 0 16 16" aria-hidden="true" fill="none" stroke="currentColor"
                    stroke-width="1.6" stroke-linecap="round" stroke-linejoin="round">
                    <path d="M6.2 4 10.2 8 6.2 12" />
                </svg>
            </button>
            <span class=style::text>
                {text}
                <span class=style::count>{format!(" — {packages}")}</span>
            </span>
            <span class=style::trailing>{trailing}</span>
        </div>
    }
}
