//! One file in an installed package.
//!
//! # Not a `FileRow`
//!
//! That one is *a recently-changed file* for the cross-package feed: it carries
//! the package it belongs to, which a page about one package does not need, and
//! `qhq-8mgw.63` is filed against the `role=button` it wraps around a link. This
//! row does not inherit the defect — **the checkbox is the control and the name
//! is not a button.**
//!
//! # The resting state is silent
//!
//! `Downloaded` is what most of a seven-hundred-row list is, and printing it
//! seven hundred times spends the reader's attention on the one thing that needs
//! none. Absence says the file is here. Pass no `state` for it — and none for an
//! ignored file either, for a different reason: the `Ignored` facet is the only
//! view that shows those, so every row in it is ignored and the label would
//! repeat what the facet already said.
//!
//! # Only a downloadable row carries a box
//!
//! A tick on a file that is already here has nothing to act on, and it is what
//! made *select all 56* disagree with *download 17*. An unselectable row keeps
//! the column's width so the names still line up.
//!
//! # The `<label>` stops before the overflow
//!
//! Clicking the row toggles its box, which wants a `<label>` around the row —
//! but `<button>` is a labelable element, and one inside a `<label>` that is not
//! its control is invalid. So the label covers box, name, state and size, and the
//! menu is its sibling.

use leptos::prelude::*;

use super::ActionMenu;
use super::Checkbox;
use super::MenuAction;
use super::StateLabel;
use super::state_label::StateTone;

stylance::import_crate_style!(style, "src/kit/entry_row.module.scss");

/// What a marked row's `title` says. One sentence, in the page's own words —
/// no `remote`, no `diverged`, and no platform named as the other place.
pub const DIFFERS_TITLE: &str =
    "Your version of this file and the published version have different contents.";

#[component]
pub fn EntryRow(
    /// What to show. The caller decides whether that is the whole path or the
    /// leaf under a group heading; this row only truncates it.
    #[prop(into)]
    name: String,
    /// The state's words, or nothing at all for a resting state.
    #[prop(optional, into)]
    state: Option<String>,
    /// How loudly. Ignored when `state` is absent.
    #[prop(optional)]
    tone: StateTone,
    /// Already formatted — the kit has no opinion about units.
    #[prop(into)]
    size: String,
    /// Whether this file can be downloaded, and therefore ticked.
    #[prop(optional)]
    selectable: bool,
    #[prop(optional, into)] selected: MaybeProp<bool>,
    #[prop(optional, into)] on_toggle: Option<Callback<bool>>,
    /// The two revisions disagree about this file. **Information, never a
    /// control** — resolution happens at revision level, so there is nothing to
    /// click here and the marking must not look like the state beside it.
    #[prop(optional)]
    differs: bool,
    /// The row's `[⋯]`. Empty means no menu at all rather than an empty one.
    #[prop(optional)]
    actions: Vec<MenuAction>,
) -> impl IntoView {
    let is_selected = Signal::derive(move || selected.get().unwrap_or(false));
    let full_name = name.clone();

    let class = if differs {
        format!("{} {}", style::root, style::differs)
    } else {
        String::from(style::root)
    };

    view! {
        <div
            class=class
            // The whole answer is a sentence, so it is one: `title` is not
            // keyboard-reachable and is absent on touch, which is why the pane
            // also says it once in prose for the rows as a set.
            title=differs.then_some(DIFFERS_TITLE)
        >
            <label class=style::main>
                // Empty, and exactly a disclosure button wide, so a file's box
                // sits under its group's box rather than a triangle's width to
                // the left of it.
                <span class=style::gutter />
                {if selectable {
                    view! {
                        <Checkbox
                            state=Signal::derive(move || is_selected.get().into())
                            on_toggle=move |next| {
                                if let Some(toggle) = on_toggle {
                                    toggle.run(next);
                                }
                            }
                        />
                    }
                        .into_any()
                } else {
                    view! { <span class=style::nobox /> }.into_any()
                }}
                <span class=style::name title=full_name>{name}</span>
                // A fixed slot, so a size lands in the same column whether or not
                // the row above carries a label. Sizes exist to be compared, and
                // ragged ones cannot be.
                <span class=style::state>
                    {state.map(|words| view! { <StateLabel tone=tone>{words}</StateLabel> })}
                </span>
                <span class=style::size>{size}</span>
            </label>
            {if actions.is_empty() {
                // A menu-shaped hole, for the same reason an unselectable row keeps
                // a box-shaped one: without it the sizes in a list where one row has
                // no actions stop being a column.
                view! { <span class=style::nomenu /> }.into_any()
            } else {
                view! { <ActionMenu aria_label="More actions for this file" actions=actions /> }
                    .into_any()
            }}
        </div>
    }
}
