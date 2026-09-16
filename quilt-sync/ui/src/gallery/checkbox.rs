//! `Checkbox` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::CheckState;
use crate::kit::Checkbox;

#[component]
pub fn CheckboxStories() -> impl IntoView {
    let live = RwSignal::new(CheckState::Mixed);
    let off = Signal::derive(|| CheckState::Off);
    let on = Signal::derive(|| CheckState::On);
    let mixed = Signal::derive(|| CheckState::Mixed);

    view! {
        <Story
            title="Checkbox"
            note="Three states, not two booleans. The DOM keeps `indeterminate` beside \
                  `checked` and lets them disagree — a box set to both draws mixed and \
                  silently ignores the tick — so the state is one value with three cases. \
                  \
                  Click the live one: mixed resolves upwards, to everything selected, which \
                  is what `3 of 17 selected` reading `17 of 17` means. There is no way to \
                  click your way back to mixed, and that is correct. \
                  \
                  The input is real and off-screen rather than replaced, so space still \
                  toggles it and the accessibility tree gets a checkbox. Tab to one."
        >
            <Cell label="off">
                <Checkbox state=off on_toggle=|_| () aria_label="Off" />
            </Cell>
            <Cell label="on">
                <Checkbox state=on on_toggle=|_| () aria_label="On" />
            </Cell>
            <Cell label="mixed — some, not all">
                <Checkbox state=mixed on_toggle=|_| () aria_label="Mixed" />
            </Cell>
            <Cell label="live — starts mixed, never returns">
                <Checkbox
                    state=live
                    on_toggle=move |next| live.set(next.into())
                    aria_label="Live"
                />
            </Cell>
            <Cell label="off · disabled">
                <Checkbox state=off on_toggle=|_| () disabled=true aria_label="Off, disabled" />
            </Cell>
            <Cell label="on · disabled">
                <Checkbox state=on on_toggle=|_| () disabled=true aria_label="On, disabled" />
            </Cell>
            <Cell label="mixed · disabled">
                <Checkbox
                    state=mixed
                    on_toggle=|_| ()
                    disabled=true
                    aria_label="Mixed, disabled"
                />
            </Cell>
            <Cell wide=true label="in a label — the words are the target, and the name">
                <label style="display:inline-flex;align-items:center;gap:8px;cursor:pointer">
                    <Checkbox state=on on_toggle=|_| () />
                    "Sync entire package"
                </label>
            </Cell>
        </Story>
    }
}
