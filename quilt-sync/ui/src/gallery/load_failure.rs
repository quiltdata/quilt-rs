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
                  find out. \
                  \
                  The sentence is the caller's, because it names which read failed. \
                  `Try again` is not, because four surfaces are how two words become three \
                  wordings."
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
            <Cell wide=true label="centred — in a box whose padding is its rows' own">
                <Card flush=true label="Files">
                    <LoadFailure
                        centred=true
                        words="Could not read this package's files."
                        on_retry=Callback::new(|()| ())
                    />
                </Card>
            </Cell>
            <Cell wide=true label="with a detail — the reason, where the spec shows it">
                <div style="width:264px">
                    <LoadFailure
                        words="Could not compare the revisions."
                        detail="The bucket did not answer."
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
