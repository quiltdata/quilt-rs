//! A heading and the rows under it, collapsible.
//!
//! # Not `GroupHeading`
//!
//! That one is a flat heading emitted *between* runs of rows, and the pages using
//! it want it to stay that way. This is a container: it holds its rows, owns
//! whether they are shown, and carries a box that ticks all of them. Different
//! contract, different component.
//!
//! # A collapsed group renders nothing
//!
//! `<Show>` and not `display: none`. The 1000-entry cap is argued partly on
//! grouping sparing the DOM, and a hidden subtree is still built — so hiding with
//! CSS would have made that argument false while looking identical.
//!
//! # The heading cannot be a `<label>`
//!
//! It holds the disclosure `<button>`, and a `<label>` may not contain another
//! labelable element. So the box names itself — this is the call site
//! [`Checkbox`](super::Checkbox)'s `aria_label` exists for, and the name has to
//! say which group, because a list of them all reading `Select all` names
//! nothing.

use leptos::prelude::*;

use super::CheckState;
use super::Checkbox;
use super::icons;

stylance::import_crate_style!(style, "src/kit/entry_group.module.scss");

#[component]
pub fn EntryGroup(
    /// The folder this run of rows shares.
    #[prop(into)]
    name: String,
    /// How many rows follow. Always shown, including one — "1" is information,
    /// and hiding it would make a single-row group look like a header with a bug.
    #[prop(into)]
    count: Signal<usize>,
    open: RwSignal<bool>,
    /// Off, On, or Mixed across the group's **selectable** rows.
    #[prop(into)]
    state: Signal<CheckState>,
    on_toggle: impl Fn(bool) + 'static,
    children: ChildrenFn,
) -> impl IntoView {
    let full_name = name.clone();
    let box_label = format!("Select all in {name}");

    view! {
        <div class=style::root>
            <div class=style::head>
                <button
                    type="button"
                    class=style::disclose
                    aria-expanded=move || open.get().to_string()
                    // Names what it does, not what it looks like. The glyph is
                    // `aria-hidden`, so without this the button announces nothing.
                    aria-label=move || {
                        if open.get() { "Collapse group" } else { "Expand group" }
                    }
                    on:click=move |_| open.update(|o| *o = !*o)
                >
                    {move || if open.get() { icons::chevron_down() } else { icons::chevron_right() }}
                </button>
                <Checkbox state=state on_toggle=on_toggle aria_label=box_label />
                <span class=style::name title=full_name>{name}</span>
                <span class=style::count>{move || count.get()}</span>
            </div>
            <Show when=move || open.get()>{children()}</Show>
        </div>
    }
}
