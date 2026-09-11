//! `SearchInput` stories.

use leptos::prelude::*;

use crate::Cell;
use crate::Story;
use crate::kit::SearchInput;

#[component]
pub fn SearchInputStories() -> impl IntoView {
    let empty = RwSignal::new(String::new());
    let typed = RwSignal::new("plate-07".to_string());
    let long = RwSignal::new("vir-quilt-res-3/plate-screening-2026-08-cohort-b".to_string());

    view! {
        <Story
            title="SearchInput"
            note="Type in the first cell and a clear button appears — it is absent while the \
                  field is empty, because a clear button with nothing to clear is a control \
                  that does nothing. WebKit's own cancel button is suppressed: it is \
                  unthemeable and differs per platform."
        >
            <Cell wide=true label="empty — no clear button">
                <SearchInput value=empty aria_label="Search packages" placeholder="Search…" />
            </Cell>
            <Cell wide=true label="with text — clear button appears">
                <SearchInput value=typed aria_label="Search packages" placeholder="Search…" />
            </Cell>
            <Cell wide=true label="long value truncates, control does not grow">
                <SearchInput value=long aria_label="Search packages" placeholder="Search…" />
            </Cell>
        </Story>
    }
}
