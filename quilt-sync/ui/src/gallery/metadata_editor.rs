//! The commit page's metadata editor, open, in the states that draw its own
//! parts: the tree, the start screen of an empty document, and the bar it
//! shows over JSON it cannot read. Switch the theme: it follows.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::pages::commit::MetadataField;

const NESTED: &str = "{\"assay\": \"ELISA\", \"plate\": 7, \"reviewed\": true, \"operator\": null, \
    \"wells\": [\"A1\", \"A2\", \"A3\"], \"protocol\": {\"version\": 3, \"incubation_min\": 60}, \
    \"source\": \"https://example.com/plates/7\"}";

fn editor(metadata: &str) -> AnyView {
    view! {
        <div style="width:480px; max-width:100%">
            <MetadataField metadata=RwSignal::new(metadata.to_string()) editing=RwSignal::new(true) />
        </div>
    }
    .into_any()
}

const NOTE: &str = "The metadata editor in the app's palette: keys and values in the text \
    colour, punctuation muted, its menu and context menu on the kit's surfaces. Right-click a \
    key for the context menu.";

#[component]
pub fn MetadataEditorScene() -> impl IntoView {
    view! {
        <Scene title="The metadata editor" note=NOTE>
            <Cell label="every kind of value, nested">{editor(NESTED)}</Cell>
            <Cell label="empty — the editor's start screen">{editor("")}</Cell>
            <Cell label="not valid JSON — the editor offers to repair it">
                {editor("{\"assay\": \"ELISA\", \"plate\": 7,")}
            </Cell>
        </Scene>
    }
}
