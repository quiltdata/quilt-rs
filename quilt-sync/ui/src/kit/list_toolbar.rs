//! Container for the list region's controls.
//!
//! Deliberately holds nothing itself: the two views need different controls — the
//! packages view adds Sort and Create package, the files view drops both, and
//! Group's options differ between them — so composition belongs to the caller.
//!
//! # Which control ends up nearest the list
//!
//! The one thing the container does decide is line order, because only it knows
//! there is a wrap to order. A toolbar sits above a list, and when it stacks, the
//! control that acts on **rows** wants to be the line touching them rather than
//! the one furthest away. `reverse_when_stacked` says so. On a single line it
//! changes nothing at all, which is why it is a property of stacking and not of
//! arrangement.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/list_toolbar.module.scss");

#[component]
pub fn ListToolbar(
    /// Stack upwards: the first child ends on the **last** line, nearest the
    /// list. `wrap-reverse` reverses the lines and leaves each line's own order
    /// alone, so nothing moves until the toolbar actually wraps.
    #[prop(optional)]
    reverse_when_stacked: bool,
    children: Children,
) -> impl IntoView {
    let class = if reverse_when_stacked {
        format!("{} {}", style::root, style::reversed)
    } else {
        String::from(style::root)
    };
    view! { <div class=class>{children()}</div> }
}
