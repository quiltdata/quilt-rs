//! The page frame: an appbar and a width-capped column for the regions.
//!
//! # What it owns, and why that matters
//!
//! **The space between regions.** The state strip, the queue and the list region set
//! no outer margins of their own — this sets one gap and they inherit the rhythm. A
//! region that spaced itself would look right alone and wrong beside the others, and
//! there would be no single place to change how the page breathes.
//!
//! # What is deliberately not here yet
//!
//! v1's `components::layout::Layout` also carries breadcrumbs, a notification host and
//! a `ui_locked` overlay. None of the three is in this one:
//!
//! - **Breadcrumbs** — the main page is the root, so its trail is one item, and a
//!   breadcrumb of one is decoration. The first v2 page with a parent adds them.
//! - **`ui_locked`** — its overlay wants a decision about whether a modal spinner is even
//!   the right answer, given the page already reports work per region. `Spinner` exists
//!   now, so this is a design question rather than a missing part.
//!
//! The notification host is no longer among them — see `notification` below.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/layout.module.scss");

#[component]
pub fn Layout(
    /// Appbar controls, pushed to the right — on the main page, refresh and settings
    /// as Framed `IconButton`s. A slot rather than named props, because the appbar has
    /// no opinion about which page needs which controls.
    #[prop(optional)]
    actions: Option<AnyView>,
    /// A `Notification`, when there is one. In the flow directly under the appbar, so it
    /// pushes the page down rather than floating over it — the design bans anchored
    /// positioning, and a bar cannot be missed by someone looking at the bottom of a long
    /// list. A slot rather than the signal itself, so the layout stays ignorant of what
    /// kinds exist and who dismisses them.
    #[prop(optional)]
    notification: Option<AnyView>,
    children: Children,
) -> impl IntoView {
    view! {
        <div class=style::root>
            // `header` and `main` rather than divs: they are the two landmarks a
            // screen reader offers to skip between, and they cost nothing.
            <header class=style::appbar>
                <div class=style::bar>
                    <a class=style::logo href="/">
                        // Alt text, not `aria-hidden`: it is the only content of a
                        // link, so hiding it would leave the link unnamed.
                        <img src="/assets/img/quilt.png" alt="QuiltSync home" />
                    </a>
                    {actions.map(|actions| view! { <span class=style::actions>{actions}</span> })}
                </div>
            </header>
            {notification
                .map(|notification| {
                    view! { <div class=style::notice>{notification}</div> }
                })}
            <main class=style::main>{children()}</main>
        </div>
    }
}
