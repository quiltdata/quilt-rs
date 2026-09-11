//! Select stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::FormControl;
use crate::kit::Naming;
use crate::kit::Select;

#[component]
pub fn SelectStories() -> impl IntoView {
    let group = RwSignal::new("Bucket".to_string());
    let sort = RwSignal::new("Changed".to_string());
    let role = RwSignal::new("analyst".to_string());
    let long = RwSignal::new("s3://vir-quilt-res-3-in-progress".to_string());
    let one = RwSignal::new("analyst".to_string());
    let off = RwSignal::new("Bucket".to_string());
    let workflow = RwSignal::new("Default".to_string());

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
                  and the thing to check in Epiphany. Clicking anywhere on the box opens it, \
                  because the select itself is stretched over the whole wrapper at opacity 0 \
                  — a wrapping label would not have done it, since browsers deliberately \
                  never synthesise 'open the dropdown' from a label click. \
                  \
                  Naming is a required enum with no anonymous variant, so there is no way to \
                  build a select nobody can name. Three ways, all three below: Prefix draws \
                  the name inside the control, Hidden puts it in aria-label, and FormControl \
                  hands the naming to a FormControl's label and takes its ids."
        >
            <Cell label="visible label — the toolbar's form">
                <Select naming=Naming::Prefix("Group".to_string()) options=axes() selected=group />
            </Cell>
            <Cell label="label hidden — named, but not shown">
                <Select naming=Naming::Hidden("Role".to_string()) options=roles() selected=role />
            </Cell>
            <Cell label="disabled">
                <Select naming=Naming::Prefix("Group".to_string()) options=axes() selected=off disabled=true />
            </Cell>
            <Cell label="long option truncates, control does not grow">
                <Select
                    naming=Naming::Prefix("Bucket".to_string())
                    options=vec![
                        "s3://vir-quilt-res-3-in-progress".to_string(),
                        "s3://quilt-example".to_string(),
                    ]
                    selected=long
                />
            </Cell>
            <Cell label="one option — a dead control; callers must not render this">
                <Select naming=Naming::Hidden("Role".to_string()) options=vec!["analyst".to_string()] selected=one />
            </Cell>
            <Cell wide=true label="Naming::FormControl — the form's shape, no name of its own">
                <FormControl
                    label="Workflow"
                    caption="Rules the bucket applies when you publish."
                    control=move |id| {
                        view! {
                            <Select
                                naming=Naming::FormControl(id)
                                options=vec!["Default".to_string(), "None".to_string()]
                                selected=workflow
                            />
                        }
                            .into_any()
                    }
                />
            </Cell>
            <Cell label="a pair, as the list toolbar uses them">
                <div class="g-inline">
                    <Select naming=Naming::Prefix("Group".to_string()) options=axes() selected=group />
                    <Select naming=Naming::Prefix("Sort".to_string()) options=sorts() selected=sort />
                </div>
            </Cell>
        </Story>
    }
}
