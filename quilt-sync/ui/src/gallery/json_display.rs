//! `JsonDisplay` stories. The fixed-width cells match tests in
//! `kit/json_oneliner.rs`.

use leptos::prelude::*;
use serde_json::{Value, json};

use crate::Cell;
use crate::Story;
use crate::kit::Expanded;
use crate::kit::JsonDisplay;

/// The catalog spec's nested fixture.
fn nested() -> Value {
    json!({
        "A": [1, 2, 3],
        "B": "Lorem",
        "C": {"a": 1, "b": 2},
        "D": {"a": [1, {"c": 3}, "a"], "b": 2},
        "E": [1, {"b": ["c", {"d": "e"}]}, "a"],
    })
}

/// The width that leaves the printer `budget`: 2 for the chevron, 10 reserved.
fn printer(budget: f64) -> f64 {
    budget + 12.0
}

/// Package metadata as it is usually written: a few flat fields and one or two
/// nested ones.
fn metadata() -> Value {
    json!({
        "assay": "ELISA",
        "plate": 7,
        "instrument": "SpectraMax iD5",
        "reviewed": true,
        "wells": ["A1", "A2", "A3"],
        "protocol": {"version": 3, "incubation_min": 60, "notes": null},
        "source": "https://example.com/protocols/elisa-v3",
    })
}

#[component]
pub fn JsonDisplayStories() -> impl IntoView {
    view! {
        <Story
            title="JsonDisplay"
            note="The catalog's JSON viewer, ported: each array and object is one line until \
                  opened, and what does not fit is counted, `<…N>`. Unless a cell is fixed, \
                  the line refits as the window narrows."
        >
            <Cell full=true label="folded, measured width — package metadata">
                <JsonDisplay value=metadata() />
            </Cell>
            <Cell full=true label="one level open">
                <JsonDisplay value=metadata() expanded=Expanded::Levels(1) />
            </Cell>
            <Cell full=true label="everything open — arrays are keyed by index, as in the catalog">
                <JsonDisplay value=nested() expanded=Expanded::All />
            </Cell>
            <Cell full=true label="the catalog's 140 — everything fits">
                <JsonDisplay value=nested() chars=printer(140.0) />
            </Cell>
            <Cell full=true label="the catalog's 110 — the last nested value folds to <…2>">
                <JsonDisplay value=nested() chars=printer(110.0) />
            </Cell>
            <Cell full=true label="the catalog's 50 — nested values fold, and two keys drop off the end">
                <JsonDisplay value=nested() chars=printer(50.0) />
            </Cell>
            <Cell full=true label="keys only (show_values off): an object's line names its fields">
                <JsonDisplay value=metadata() show_values=false />
            </Cell>
            <Cell full=true label="URLs open through the caller, when it can — here, a no-op">
                <JsonDisplay
                    value=metadata()
                    expanded=Expanded::Levels(1)
                    on_open_url=Callback::new(|_: String| ())
                />
            </Cell>
            <Cell label="empty object — a line, not a control">
                <JsonDisplay value=json!({}) />
            </Cell>
            <Cell label="empty array">
                <JsonDisplay value=json!([]) />
            </Cell>
            <Cell label="a string at the top">
                <JsonDisplay value=json!("Lorem ipsum") />
            </Cell>
            <Cell label="a named number">
                <JsonDisplay value=json!(7) name="plate" />
            </Cell>
        </Story>
    }
}
