//! `BackLink` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::BackLink;

#[component]
pub fn BackLinkStories() -> impl IntoView {
    view! {
        <Story
            title="BackLink"
            note="An up-link, not browser Back: it goes to the parent route whatever the \
                  history holds, which is exactly why it names its destination instead of \
                  saying 'Back'. Someone can reach a package from the queue, from a deep \
                  link or from the commit page, and all three leave by the same door. \
                  \
                  The label follows the destination's own name, so if the page above ever \
                  renames itself this changes with it. A breadcrumb trail was the \
                  alternative and it is a trail of two — the second crumb restating the \
                  page heading an inch below itself."
        >
            <Cell label="the package page's own">
                <BackLink href="/" label="Packages" />
            </Cell>
            <Cell label="a long destination — no truncation, it is two words">
                <BackLink href="/" label="Installed packages" />
            </Cell>
        </Story>
    }
}
