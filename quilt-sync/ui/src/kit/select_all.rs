//! The tri-state box that ticks a list, and says what it would tick.
//!
//! # A checkbox, because a button cannot say *unselect*
//!
//! `Select all` and `Clear` are one control with two meanings, and a button has
//! to pick a word before it knows which it is. A checkbox already holds three
//! states, and the third — some but not all — is the one a pair of buttons
//! cannot draw.
//!
//! # The label states its own extent
//!
//! Search and the facets change what is on screen, and therefore what this
//! ticks. Rather than asking the reader to trust a control whose reach they
//! cannot see, the label says it: `Select all 12 shown` when the view is
//! narrowed, and `Select all 17` when it is not. That is the whole reason this
//! is a component and not a `Checkbox` with a string beside it.

use leptos::prelude::*;

use super::CheckState;
use super::Checkbox;

stylance::import_crate_style!(style, "src/kit/select_all.module.scss");

#[component]
pub fn SelectAll(
    /// How many rows are ticked.
    #[prop(into)]
    selected: Signal<usize>,
    /// How many rows the **current view** offers. Not how many the package
    /// holds: the two differ under a search or a facet, and this is the number
    /// the control can act on.
    #[prop(into)]
    total: Signal<usize>,
    /// Whether a view control has narrowed the list, which puts `shown` in the
    /// label. The caller knows; the count alone cannot say, because a facet can
    /// narrow to exactly the number of rows the package has.
    #[prop(optional, into)]
    narrowed: MaybeProp<bool>,
    /// `true` to tick everything in view, `false` to clear.
    on_toggle: impl Fn(bool) + 'static,
    #[prop(optional, into)] disabled: MaybeProp<bool>,
) -> impl IntoView {
    let state = Signal::derive(move || {
        let (picked, all) = (selected.get(), total.get());
        if picked == 0 || all == 0 {
            CheckState::Off
        } else if picked >= all {
            CheckState::On
        } else {
            CheckState::Mixed
        }
    });

    let text = move || {
        let (picked, all) = (selected.get(), total.get());
        if picked == 0 {
            if narrowed.get().unwrap_or(false) {
                format!("Select all {all} shown")
            } else {
                format!("Select all {all}")
            }
        } else {
            format!("{picked} of {all} selected")
        }
    };

    view! {
        // The whole thing is the label, so the words are the target too — this
        // sits above a list of rows whose own boxes are 17px, and a 17px target
        // in a toolbar is a miss waiting to happen.
        <label class=style::root>
            <Checkbox state=state on_toggle=on_toggle disabled=disabled />
            <span class=style::text>{text}</span>
        </label>
    }
}
