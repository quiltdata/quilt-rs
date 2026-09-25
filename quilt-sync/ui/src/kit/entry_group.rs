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
//! # A group with nothing to select carries no box
//!
//! Only a downloadable row has a checkbox, so a group whose files are all
//! present has none to tick. Its heading box would be a control that cannot act
//! — the dead control [`Select`](super::Select) already refuses to be. Pass no
//! [`GroupSelection`] and the column stays open so the names still line up, but
//! nothing is drawn in it.
//!
//! # The gutter is the list's
//!
//! The disclosure sits in `--q-entry-gutter`, the same column the rows leave
//! empty. A list that renders no group at all sets that value to `0` — see
//! [`EntryRow`](super::EntryRow) — which is why the width lives on the list
//! rather than in here.
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

/// A group's tick: where its selectable rows stand, and what to do when the
/// heading box moves.
///
/// A group has one exactly when at least one row under it can be downloaded.
/// Same shape as [`EntrySelection`](super::EntrySelection), and for the same
/// reason: there is no way to say "selectable" without saying what it means.
#[derive(Clone, Copy)]
pub struct GroupSelection {
    pub state: Signal<CheckState>,
    pub on_toggle: Callback<bool>,
    /// The box is shown but cannot move, as while a download runs.
    pub disabled: Signal<bool>,
}

impl GroupSelection {
    #[must_use]
    pub fn new(state: impl Into<Signal<CheckState>>, on_toggle: Callback<bool>) -> Self {
        Self {
            state: state.into(),
            on_toggle,
            disabled: Signal::stored(false),
        }
    }

    /// Hold the box still while `disabled` is true.
    #[must_use]
    pub fn disabled(self, disabled: impl Into<Signal<bool>>) -> Self {
        Self {
            disabled: disabled.into(),
            ..self
        }
    }
}

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
    /// Present when at least one row under this heading can be downloaded.
    /// Absent draws no box at all.
    #[prop(optional)]
    selection: Option<GroupSelection>,
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
                {match selection {
                    Some(GroupSelection { state, on_toggle, disabled }) => {
                        view! {
                            <Checkbox
                                state=state
                                on_toggle=move |next| on_toggle.run(next)
                                aria_label=box_label
                                disabled=disabled
                            />
                        }
                            .into_any()
                    }
                    None => view! { <span class=style::nobox /> }.into_any(),
                }}
                <span class=style::name title=full_name>{name}</span>
                <span class=style::count>{move || count.get()}</span>
            </div>
            <Show when=move || open.get()>{children()}</Show>
        </div>
    }
}
