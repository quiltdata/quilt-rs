//! `Checkbox` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Button;
use crate::kit::CheckState;
use crate::kit::Checkbox;

#[component]
pub fn CheckboxStories() -> impl IntoView {
    // Every enabled box here owns a real signal. A cell pinned to one state with
    // a no-op callback would be a control that takes a click and discards it,
    // which is the defect this kit just removed from `EntryRow` — a gallery is
    // the last place it should reappear.
    let off = RwSignal::new(CheckState::Off);
    let on = RwSignal::new(CheckState::On);
    let mixed = RwSignal::new(CheckState::Mixed);
    // Its own, so the two cells do not move together and look wired to each other.
    let labelled = RwSignal::new(CheckState::On);
    let disabled_off = Signal::derive(|| CheckState::Off);
    let disabled_on = Signal::derive(|| CheckState::On);
    let disabled_mixed = Signal::derive(|| CheckState::Mixed);

    view! {
        <Story
            title="Checkbox"
            note="Three states, not two booleans. The DOM keeps `indeterminate` beside \
                  `checked` and lets them disagree — a box set to both draws mixed and \
                  silently ignores the tick — so the state is one value with three cases. \
                  \
                  Click any of the first three. Mixed resolves upwards, to everything \
                  selected, which is what `3 of 17 selected` reading `17 of 17` means — and \
                  there is no way to click your way back to it, which is why that cell \
                  carries a reset. The third state is a report about a set, not something a \
                  reader can ask for. \
                  \
                  The input is real and off-screen rather than replaced, so space still \
                  toggles it and the accessibility tree gets a checkbox. Tab to one."
        >
            <Cell label="off — click it">
                <Checkbox
                    state=off
                    on_toggle=move |next| off.set(next.into())
                    aria_label="Off"
                />
            </Cell>
            <Cell label="on — click it">
                <Checkbox
                    state=on
                    on_toggle=move |next| on.set(next.into())
                    aria_label="On"
                />
            </Cell>
            <Cell wide=true label="mixed — some, not all. Click it, then reset">
                <div class="g-inline">
                    <Checkbox
                        state=mixed
                        on_toggle=move |next| mixed.set(next.into())
                        aria_label="Mixed"
                    />
                    <Button on_click=move |_| mixed.set(CheckState::Mixed)>
                        "Reset to mixed"
                    </Button>
                </div>
            </Cell>
            <Cell label="off · disabled">
                <Checkbox
                    state=disabled_off
                    on_toggle=|_| ()
                    disabled=true
                    aria_label="Off, disabled"
                />
            </Cell>
            <Cell label="on · disabled">
                <Checkbox
                    state=disabled_on
                    on_toggle=|_| ()
                    disabled=true
                    aria_label="On, disabled"
                />
            </Cell>
            <Cell label="mixed · disabled">
                <Checkbox
                    state=disabled_mixed
                    on_toggle=|_| ()
                    disabled=true
                    aria_label="Mixed, disabled"
                />
            </Cell>
            <Cell wide=true label="in a label — the words are the target, and the name">
                <label style="display:inline-flex;align-items:center;gap:8px;cursor:pointer">
                    <Checkbox state=labelled on_toggle=move |next| labelled.set(next.into()) />
                    "Sync entire package"
                </label>
            </Cell>
        </Story>
    }
}
