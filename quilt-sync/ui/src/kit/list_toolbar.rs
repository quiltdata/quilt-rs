//! Container for the list region's controls.
//!
//! Deliberately holds nothing itself: the two views need different controls — the
//! packages view adds Sort and Create package, the files view drops both, and
//! Group's options differ between them — so composition belongs to the caller.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/list_toolbar.module.scss");

#[component]
pub fn ListToolbar(children: Children) -> impl IntoView {
    view! { <div class=style::root>{children()}</div> }
}
