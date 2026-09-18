//! `SegmentedControl` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::Segment;
use crate::kit::SegmentedControl;

#[component]
pub fn SegmentedControlStories() -> impl IntoView {
    let two = RwSignal::new("Packages".to_string());
    let three = RwSignal::new("Prefix".to_string());
    let long = RwSignal::new("Recently changed files".to_string());
    let inert = RwSignal::new("All 53".to_string());

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
                    aria_label="List view"
                    name="story-view"
                    options=vec!["Packages".into(), "Recent files".into()]
                    selected=two
                />
            </Cell>
            <Cell label="three options — still comfortable">
                <SegmentedControl
                    aria_label="Grouping"
                    name="story-group"
                    options=vec!["Bucket".into(), "Prefix".into(), "None".into()]
                    selected=three
                />
            </Cell>
            <Cell label="a facet at zero — present, greyed, and not choosable">
                <SegmentedControl
                    aria_label="Filter files"
                    name="story-inert"
                    options=vec![
                        Segment::new("All 53"),
                        Segment::inert("Changed 0"),
                        Segment::new("Not downloaded 17"),
                        Segment::inert("Ignored 0"),
                    ]
                    selected=inert
                />
            </Cell>
            <Cell label="long labels — past this, use a Select">
                <SegmentedControl
                    aria_label="List view"
                    name="story-long"
                    options=vec!["Installed packages".into(), "Recently changed files".into()]
                    selected=long
                />
            </Cell>
        </Story>
    }
}
