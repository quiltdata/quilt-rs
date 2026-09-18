//! `LoadFailure` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Card;
use crate::kit::LoadFailure;

#[component]
pub fn LoadFailureStories() -> impl IntoView {
    view! {
        <Story
            title="LoadFailure"
            note="A read that did not answer, and the one way to ask again. Not a \
                  Blankslate: that says there is nothing to show, this says we could not \
                  find out, and `No files yet` over a read that never returned \
                  manufactures a state the page does not know. \
                  \
                  The sentence is the caller's and fixed. A backend error string is not a \
                  word from the vocabulary — unreviewable, unchangeable without a release, \
                  and invisible to every mechanical test — so it goes to the log and this \
                  says what failed in the UI's own words. `Try again` is not a prop, \
                  because four surfaces are how two words become three wordings. \
                  \
                  What the caller does own is which read runs again: the failed one, never \
                  the page's. A card that could not load re-reads that card alone. \
                  \
                  It draws and does not speak. The region that swaps it in is the thing \
                  that knows when the swap happened, so the live region and the aria-busy \
                  it drops belong there — the same split SkeletonBox draws for the other \
                  end of the same wait."
        >
            <Cell wide=true label="in a card — the strip's failed read">
                <Card title="Autosync">
                    <LoadFailure
                        words="Could not load autosync."
                        on_retry=Callback::new(|()| ())
                    />
                </Card>
            </Cell>
            <Cell wide=true label="bare, at an overlay's width">
                <div style="width:264px">
                    <LoadFailure
                        words="Could not load your revisions."
                        on_retry=Callback::new(|()| ())
                    />
                </div>
            </Cell>
            <Cell wide=true label="a longer sentence — 70ch, so it is read as prose">
                <LoadFailure
                    words="Could not load your packages. Your files are still on this \
                           machine and nothing has been changed."
                    on_retry=Callback::new(|()| ())
                />
            </Cell>
        </Story>
    }
}
