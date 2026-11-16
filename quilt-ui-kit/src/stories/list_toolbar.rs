//! The list region's toolbar, composed.
//!
//! Search, two selects and a button in one row, which is the first place their
//! heights and baselines have to agree. A control that looks right alone and
//! sits a pixel high in a row is only visible here.

use leptos::prelude::*;

use crate::Button;
use crate::Naming;
use crate::SearchInput;
use crate::Select;
use crate::story::Scene;

#[component]
pub fn ListToolbarScene() -> impl IntoView {
    let query = RwSignal::new(String::new());
    let group = RwSignal::new("Bucket".to_string());
    let sort = RwSignal::new("Changed".to_string());

    view! {
        <Scene
            title="Scene · list toolbar"
            note="Four controls of three constructions, all --q-control-height: the row must \
                  read as one line, and any control that stands proud of it has stopped \
                  reading the token."
        >
            <div style="display:flex; gap:var(--q-space-2); align-items:center; flex-wrap:wrap;">
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
