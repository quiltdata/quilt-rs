//! The first complete slice of the package context pane.
//!
//! It is deliberately read-only: one current revision and the bucket it belongs
//! to. History, scope, and resolution controls arrive with the data and actions
//! that can make them truthful.

use leptos::prelude::*;

use crate::commands;
use crate::kit::{Card, PaneSection, RevisionRow, SkeletonBox};

stylance::import_crate_style!(
    style,
    "src/pages/installed_package_v2/context_pane.module.scss"
);

/// The current revision and package bucket.
#[component]
pub fn CurrentRevisionPane(data: commands::PackageContextData) -> impl IntoView {
    let bucket = data.bucket.filter(|bucket| !bucket.is_empty()).map_or_else(
        || "No S3 bucket".to_string(),
        |bucket| format!("s3://{bucket}"),
    );

    view! {
        <aside aria-label="About this package" class=style::root>
            // The card is visually titleless, but its hidden h2 keeps the
            // PaneSection's h3 in a complete heading hierarchy.
            <Card label="About this package">
                <PaneSection label="Revision">
                    <RevisionRow
                        message=data.revision.message.unwrap_or_default()
                        at=data.revision.obtained_at
                    />
                    <span class=style::bucket>{bucket}</span>
                </PaneSection>
            </Card>
        </aside>
    }
}

/// The pane's loading geometry, occupying the same card and section structure.
#[component]
pub fn CurrentRevisionPaneSkeleton() -> impl IntoView {
    view! {
        <aside aria-label="About this package" class=style::root>
            <Card label="About this package" busy=Signal::stored(true)>
                <PaneSection label="Revision">
                    <SkeletonBox width="80%" />
                    <SkeletonBox width="42%" />
                    <SkeletonBox width="68%" />
                </PaneSection>
            </Card>
        </aside>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::CurrentRevisionData;
    use crate::test_support::{element_saying, mount};
    use wasm_bindgen_test::*;

    fn data(message: Option<&str>, bucket: Option<&str>) -> commands::PackageContextData {
        commands::PackageContextData {
            revision: CurrentRevisionData {
                message: message.map(ToString::to_string),
                obtained_at: 1_758_500_000_000.0,
            },
            bucket: bucket.map(ToString::to_string),
        }
    }

    #[wasm_bindgen_test]
    fn the_pane_names_and_renders_the_current_revision() {
        let el = mount(|| {
            view! {
                <CurrentRevisionPane data=data(Some("Initial upload"), Some("quilt-lab-plates")) />
            }
        });

        let aside = el.query_selector("aside").unwrap().expect("an aside");
        assert_eq!(
            aside.get_attribute("aria-label").as_deref(),
            Some("About this package")
        );
        element_saying(&el, "Revision");
        assert!(
            el.text_content()
                .unwrap_or_default()
                .contains("Initial upload"),
            "RevisionRow quotes the message, but preserves its words"
        );
        element_saying(&el, "s3://quilt-lab-plates");

        let time = el
            .query_selector("time")
            .unwrap()
            .expect("an obtained time");
        assert!(
            time.get_attribute("datetime")
                .is_some_and(|value| !value.is_empty()),
            "the machine-readable time travels with the relative one"
        );
        assert!(
            el.query_selector("button, a, input").unwrap().is_none(),
            "the first slice makes no unimplemented action look available"
        );
    }

    #[wasm_bindgen_test]
    fn absent_facts_have_honest_words() {
        for message in [None, Some("")] {
            let el = mount(move || view! { <CurrentRevisionPane data=data(message, None) /> });
            element_saying(&el, "No message");
            element_saying(&el, "No S3 bucket");
        }
    }

    #[wasm_bindgen_test]
    fn the_skeleton_marks_the_composing_region_busy() {
        let el = mount(|| view! { <CurrentRevisionPaneSkeleton /> });
        let card = el
            .query_selector("aside > section")
            .unwrap()
            .expect("the composing card");
        assert_eq!(card.get_attribute("aria-busy").as_deref(), Some("true"));

        let bars = el.query_selector_all("[aria-hidden='true']").unwrap();
        assert_eq!(bars.length(), 3);
    }
}
