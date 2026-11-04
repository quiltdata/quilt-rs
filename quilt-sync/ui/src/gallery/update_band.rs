//! `UpdateBand`: a newer release, as a band under the appbar.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::after_a_beat;
use crate::kit::UpdateBand;
use crate::kit::UpdateState;

fn available() -> UpdateState {
    UpdateState::Available {
        version: "0.42.0".to_string(),
    }
}

#[component]
pub fn UpdateBandStories() -> impl IntoView {
    let live = RwSignal::new(Some(available()));
    let install = move |()| {
        live.set(Some(UpdateState::Installing));
        leptos::task::spawn_local(async move {
            after_a_beat(1_500).await;
            // The story may be gone by now.
            live.try_set(Some(UpdateState::Failed {
                version: "0.42.0".to_string(),
                error: "the download was cut short".to_string(),
            }));
        });
    };

    view! {
        <Story
            title="UpdateBand"
            note="The update checker's notice, in the kit's band. An update is state, true \
                  until installed, so it stands in the page's flow under the appbar rather \
                  than in the inbox. Attention while offered, since a newer release is an \
                  ordinary step outstanding; Critical when the install failed. Dismiss \
                  snoozes it, as the checker always has; there is no dismiss while it \
                  installs. The words are the checker's."
        >
            <Cell full=true label="available">
                <div class="g-stack">
                    <UpdateBand state=available() on_install=|()| () on_snooze=|()| () />
                </div>
            </Cell>
            <Cell full=true label="installing">
                <div class="g-stack">
                    <UpdateBand state=UpdateState::Installing on_install=|()| () on_snooze=|()| () />
                </div>
            </Cell>
            <Cell full=true label="the install failed">
                <div class="g-stack">
                <UpdateBand
                    state=UpdateState::Failed {
                        version: "0.42.0".to_string(),
                        error: "the download was cut short".to_string(),
                    }
                    on_install=|()| ()
                    on_snooze=|()| ()
                />
                </div>
            </Cell>
            <Cell full=true label="live: install, then a failure; dismiss snoozes">
                <div class="g-stack">
                    <Show
                        when=move || live.with(Option::is_some)
                        fallback=move || {
                            view! {
                                <button type="button" on:click=move |_| live.set(Some(available()))>
                                    "Snoozed. Show it again"
                                </button>
                            }
                        }
                    >
                        <UpdateBand
                            state=Signal::derive(move || live.get().unwrap_or_else(available))
                            on_install=install
                            on_snooze=move |()| live.set(None)
                        />
                    </Show>
                </div>
            </Cell>
        </Story>
    }
}
