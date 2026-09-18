//! Segmented control — one choice, all options visible.
//!
//! The same data as [`Select`](super::Select): a list of options and the chosen
//! one. The difference is only whether the options are on screen or behind a
//! dropdown, so the rule of thumb is a count — two or three short options here,
//! more than that in a `Select`.
//!
//! # The rule of thumb is about width, and it loses to discoverability
//!
//! The installed-package page's file facets break it on purpose: four options,
//! none short, `All 53 · Changed 2 · Not downloaded 17 · Ignored 3`. Measured at
//! the shipping width they are 396px of a 680px toolbar and they push it to a
//! third row.
//!
//! Kept anyway, decided 2026-09-18: those segments are not four filters, they
//! are the page's statement of *what can be filtered*. `Ignored` is the case —
//! a user who has never ignored a file learns that ignoring exists by seeing the
//! segment, and that view replaced a hidden checkbox precisely because nobody
//! found it. Behind a `Select` it would be hidden again.
//!
//! So: reach for a `Select` when the options are a choice the user already knows
//! they have, and stay here when the options *are* the disclosure. The cost is
//! width, and it is worth naming in a review rather than discovering in a wrap.
//!
//! # An option can be present and unchoosable
//!
//! Same reasoning one step further: a facet whose count is zero must stay on
//! screen — a control that disappears when it is empty teaches nothing and
//! moves everything beside it. [`Segment::inert`] is that state. It is a
//! disabled radio, so the platform takes it out of the tab order and announces
//! it, and the segment keeps its place in the row.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/segmented_control.module.scss");

/// One segment: its words, and whether it can be chosen.
///
/// **One value, not a list of labels beside a list of disabled ones.** Two lists
/// can disagree — a disabled label that is in neither, or the selected one — and
/// the same discipline governs [`EntryAction`](super::EntryAction) and
/// [`CheckState`](super::CheckState).
#[derive(Clone)]
pub struct Segment {
    label: String,
    enabled: bool,
}

impl Segment {
    /// A segment the user can choose.
    #[must_use]
    pub fn new(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            enabled: true,
        }
    }

    /// Visible, in place, and unchoosable — a facet that currently matches
    /// nothing. Not the same as leaving it out: the row would reflow and the
    /// view would stop being discoverable, which is the whole reason these are
    /// segments and not a `Select`.
    ///
    /// The caller must not hand this the value `selected` currently holds. A
    /// disabled radio cannot be deselected by the user, so the control would be
    /// stuck — and the state has no meaning anyway: whatever is on screen came
    /// from a view that has rows.
    #[must_use]
    pub fn inert(label: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            enabled: false,
        }
    }
}

impl From<&str> for Segment {
    fn from(label: &str) -> Self {
        Self::new(label)
    }
}

impl From<String> for Segment {
    fn from(label: String) -> Self {
        Self::new(label)
    }
}

#[component]
pub fn SegmentedControl(
    /// Names the group for assistive technology. Never drawn — the options are visible, so
    /// a visible group label would be redundant.
    ///
    /// `aria_label` and not a [`FormControl`](super::FormControl): this only ever appears in a toolbar,
    /// where every option is already on screen. Same reason `SearchInput` takes one.
    #[prop(into)]
    aria_label: String,
    /// Groups the radios. Must be unique on the page — two controls sharing a name
    /// become one group, and selecting in either clears the other.
    ///
    /// `&'static str` deliberately: a name that varied at runtime would silently
    /// regroup the inputs, so it is not something a caller should be able to
    /// compute.
    name: &'static str,
    options: Vec<Segment>,
    selected: RwSignal<String>,
) -> impl IntoView {
    view! {
        <div class=style::root role="radiogroup" aria-label=aria_label>
            {options
                .into_iter()
                .map(|Segment { label, enabled }| {
                    let value = label.clone();
                    let is_selected = {
                        let value = value.clone();
                        move || selected.get() == value
                    };
                    let on_change = {
                        let value = value.clone();
                        move |_| selected.set(value.clone())
                    };
                    view! {
                        <label class=style::option>
                            <input
                                type="radio"
                                class=style::input
                                name=name
                                value=value
                                disabled=!enabled
                                prop:checked=is_selected
                                on:change=on_change
                            />
                            <span class=style::text>{label}</span>
                        </label>
                    }
                })
                .collect_view()}
        </div>
    }
}
