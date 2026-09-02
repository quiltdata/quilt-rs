//! The list region's toolbar, composed.
//!
//! Four controls in one row — the view toggle, search, two selects and a button —
//! which is the first place their heights and baselines have to agree. A control
//! that looks right alone and sits a pixel high in a row is only visible here.

use leptos::prelude::*;

use crate::Scene;
use crate::kit::Button;
use crate::kit::Naming;
use crate::kit::SearchInput;
use crate::kit::SegmentedControl;
use crate::kit::Select;

#[component]
pub fn ListToolbarScene() -> impl IntoView {
    let view_mode = RwSignal::new("Packages".to_string());
    let query = RwSignal::new(String::new());
    let group = RwSignal::new("Bucket".to_string());
    let sort = RwSignal::new("Changed".to_string());

    view! {
        <Scene
            title="Scene · list toolbar"
            note="Check the baselines: five controls of three different constructions, which \
                  is where a one-pixel disagreement shows. Switch the view toggle to Recent \
                  files — grouping options differ per view, so Group is the control the two \
                  views will disagree about."
        >
            <div style="display:flex; gap:var(--q-space-2); align-items:center; flex-wrap:wrap;">
                <SegmentedControl
                    aria_label="List view"
                    name="toolbar-view"
                    options=vec!["Packages".to_string(), "Recent files".to_string()]
                    selected=view_mode
                />
                <SearchInput value=query aria_label="Search packages" placeholder="Search…" />
                <Select
                    naming=Naming::Prefix("Group".to_string())
                    options=vec![
                        "Bucket".to_string(),
                        "Prefix".to_string(),
                        "None".to_string(),
                    ]
                    selected=group
                />
                <Select
                    naming=Naming::Prefix("Sort".to_string())
                    options=vec!["Changed".to_string(), "Name".to_string()]
                    selected=sort
                />
                <Button on_click=|_| ()>"Create package"</Button>
            </div>
        </Scene>
    }
}
