//! `SegmentedControl` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::SegmentedControl;

#[component]
pub fn SegmentedControlStories() -> impl IntoView {
    let two = RwSignal::new("Packages".to_string());
    let three = RwSignal::new("Prefix".to_string());
    let long = RwSignal::new("Recently changed files".to_string());

    view! {
        <Story
            title="SegmentedControl"
            note="Native radios, so Tab reaches the group and arrow keys move within it — \
                  free from the platform, where a div-based tablist would hand-write both. \
                  Focus one and press an arrow. Each instance needs a unique `name`: two \
                  sharing one become a single group."
        >
            <Cell label="two options — the list region's real use">
                <SegmentedControl
                    label="List view"
                    name="story-view"
                    options=vec!["Packages".to_string(), "Recent files".to_string()]
                    selected=two
                />
            </Cell>
            <Cell label="three options — still comfortable">
                <SegmentedControl
                    label="Grouping"
                    name="story-group"
                    options=vec![
                        "Bucket".to_string(),
                        "Prefix".to_string(),
                        "None".to_string(),
                    ]
                    selected=three
                />
            </Cell>
            <Cell label="long labels — past this, use a Select">
                <SegmentedControl
                    label="List view"
                    name="story-long"
                    options=vec![
                        "Installed packages".to_string(),
                        "Recently changed files".to_string(),
                    ]
                    selected=long
                />
            </Cell>
        </Story>
    }
}
