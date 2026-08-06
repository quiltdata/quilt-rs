//! Select stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Select;

#[component]
pub fn SelectStories() -> impl IntoView {
    let group = RwSignal::new("Bucket".to_string());
    let sort = RwSignal::new("Changed".to_string());
    let role = RwSignal::new("analyst".to_string());
    let long = RwSignal::new("s3://vir-quilt-res-3-in-progress".to_string());
    let one = RwSignal::new("analyst".to_string());
    let off = RwSignal::new("Bucket".to_string());

    let axes = || {
        vec![
            "Bucket".to_string(),
            "Prefix".to_string(),
            "None".to_string(),
        ]
    };
    let sorts = || vec!["Changed".to_string(), "Name".to_string()];
    let roles = || {
        vec![
            "analyst".to_string(),
            "bench-scientist".to_string(),
            "admin".to_string(),
        ]
    };

    view! {
        <Story
            title="Select"
            note="A native select in a bordered wrapper: our closed state, the OS's open \
                  dropdown. Open one — the list is the platform's, which is the whole point \
                  and the thing to check in Epiphany. The wrapper is a `label`, so clicking \
                  the prefix opens it and the accessible name needs no aria attribute."
        >
            <Cell label="visible label — the toolbar's form">
                <Select label="Group" options=axes() selected=group visible_label=true />
            </Cell>
            <Cell label="label hidden — named, but not shown">
                <Select label="Role" options=roles() selected=role />
            </Cell>
            <Cell label="disabled">
                <Select label="Group" options=axes() selected=off visible_label=true disabled=true />
            </Cell>
            <Cell label="long option truncates, control does not grow">
                <Select
                    label="Bucket"
                    options=vec![
                        "s3://vir-quilt-res-3-in-progress".to_string(),
                        "s3://quilt-example".to_string(),
                    ]
                    selected=long
                    visible_label=true
                />
            </Cell>
            <Cell label="one option — a dead control; callers must not render this">
                <Select label="Role" options=vec!["analyst".to_string()] selected=one />
            </Cell>
            <Cell label="a pair, as the list toolbar uses them">
                <div class="g-inline">
                    <Select label="Group" options=axes() selected=group visible_label=true />
                    <Select label="Sort" options=sorts() selected=sort visible_label=true />
                </div>
            </Cell>
        </Story>
    }
}
