//! `Countdown` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Countdown;

/// Deadlines are relative to page load, so the rings are genuinely mid-cycle.
fn in_secs(secs: f64) -> Signal<Option<f64>> {
    let at = js_sys::Date::now() + secs * 1000.0;
    Signal::derive(move || Some(at))
}

#[component]
pub fn CountdownStories() -> impl IntoView {
    view! {
        <Story
            title="Countdown"
            note="Driven entirely by CSS — no timer, no per-second re-render. Rust reads the \
                  clock once to seed a negative animation-delay, and CSS advances from there. \
                  The arc fills toward the event because this is a recurring cycle. Hover for \
                  what the cycle is; a ring cannot say that on its own. \
                  \
                  These are meant to be READ, not watched: a 30s cycle sweeps 12°/s and a \
                  5min one 1.2°/s, so their position is informative and their motion is not. \
                  The 3s cell is the reference for what motion actually looks like."
        >
            <Cell label="3s cycle — the only cadence fast enough to look like motion">
                {move || {
                    view! {
                        <Countdown
                            deadline=in_secs(3.0)
                            interval=3_000.0
                            label="Not a real cadence, kept as the reference for what motion looks like"
                            repeat=true
                        />
                    }
                }}
            </Cell>
            <Cell label="30s cycle, 23s left — repeating, so it loops">
                {move || {
                    view! {
                        <Countdown
                            deadline=in_secs(23.0)
                            interval=30_000.0
                            label="Checks for new revisions every 30 seconds"
                            repeat=true
                        />
                    }
                }}
            </Cell>
            <Cell label="5 min window, barely started — one pass, then holds">
                {move || {
                    view! {
                        <Countdown
                            deadline=in_secs(272.0)
                            interval=300_000.0
                            label="Publishes 5 minutes after your last edit"
                        />
                    }
                }}
            </Cell>
            <Cell label="about to fire — fills, then holds full">
                {move || {
                    view! {
                        <Countdown
                            deadline=in_secs(3.0)
                            interval=30_000.0
                            label="Checks for new revisions every 30 seconds"
                        />
                    }
                }}
            </Cell>
            <Cell label="already past — full, never over-full">
                {move || {
                    view! {
                        <Countdown
                            deadline=in_secs(-90.0)
                            interval=30_000.0
                            label="Checks for new revisions every 30 seconds"
                        />
                    }
                }}
            </Cell>
            <Cell label="no deadline — renders nothing, caller supplies idle text">
                <Countdown
                    deadline=Signal::derive(|| None)
                    interval=30_000.0
                    label="Checks for new revisions every 30 seconds"
                />
            </Cell>
        </Story>
    }
}
