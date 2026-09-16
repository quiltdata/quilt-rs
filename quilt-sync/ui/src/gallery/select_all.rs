//! `SelectAll` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::SelectAll;

#[component]
pub fn SelectAllStories() -> impl IntoView {
    let live = RwSignal::new(3_usize);

    view! {
        <Story
            title="SelectAll"
            note="A checkbox and not a button, because a button cannot say `unselect` — it \
                  has to pick a word before it knows which of the two it is, and neither \
                  word covers the third state. \
                  \
                  The label states its own extent. Search and the facets change what is on \
                  screen and therefore what this would tick, so rather than asking the \
                  reader to trust a control whose reach they cannot see, it says `Select \
                  all 12 shown`. That sentence is the whole reason this is a component \
                  rather than a Checkbox with a string beside it. \
                  \
                  Click the live one: empty to full, full back to empty, and the partial \
                  state resolves upwards like any tri-state box."
        >
            <Cell wide=true label="none ticked">
                <SelectAll selected=Signal::derive(|| 0) total=Signal::derive(|| 17) on_toggle=|_| () />
            </Cell>
            <Cell wide=true label="some — indeterminate">
                <SelectAll selected=Signal::derive(|| 3) total=Signal::derive(|| 17) on_toggle=|_| () />
            </Cell>
            <Cell wide=true label="all">
                <SelectAll selected=Signal::derive(|| 17) total=Signal::derive(|| 17) on_toggle=|_| () />
            </Cell>
            <Cell wide=true label="narrowed by a search or a facet — `shown`">
                <SelectAll
                    selected=Signal::derive(|| 0)
                    total=Signal::derive(|| 12)
                    narrowed=true
                    on_toggle=|_| ()
                />
            </Cell>
            <Cell wide=true label="live">
                <SelectAll
                    selected=live
                    total=Signal::derive(|| 17)
                    on_toggle=move |next| live.set(if next { 17 } else { 0 })
                />
            </Cell>
            <Cell wide=true label="nothing to select — callers must not render this">
                <SelectAll selected=Signal::derive(|| 0) total=Signal::derive(|| 0) on_toggle=|_| () />
            </Cell>
        </Story>
    }
}
