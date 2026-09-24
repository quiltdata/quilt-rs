//! The context pane's Resolve mode: this copy's revision against the published
//! one, and the two whole-revision choices between them.
//!
//! The words are pure so their rules are host-tested: the sentence counts the
//! page's differing set and points at the marks only when there are some
//! (`#resolve-flow`).

use std::collections::BTreeSet;
use std::sync::Arc;

use leptos::prelude::*;

use super::Wiring;
use crate::commands;
use crate::kit::{
    BackLink, Button, ButtonVariant, Card, DIFFERS_ID, LoadFailure, PaneSection, RevisionRow,
};

stylance::import_crate_style!(style, "src/pages/installed_package_v2/resolve.module.scss");

/// The pane's one sentence, counting the page's differing set.
pub(super) fn differs_sentence(differing: usize) -> String {
    if differing == 0 {
        // Nothing is marked, so the sentence points at nothing (R12).
        return "No files differ between these revisions.".to_string();
    }
    let files = plural(differing, "file differs", "files differ");
    format!("{differing} {files} between these revisions — marked in the list.")
}

fn plural<'a>(n: usize, one: &'a str, many: &'a str) -> &'a str {
    if n == 1 { one } else { many }
}

/// The pane's other mode: this copy's revision beside the published one, and
/// the two choices between them. It swaps the ordinary pane whole.
#[component]
pub fn ResolvePane(
    #[prop(into)] namespace: String,
    /// This copy's revision: the ordinary pane's facts, drawn as *Yours*.
    revision: commands::CurrentRevisionData,
    resolve: commands::ResolveData,
    /// The page's one differing set. The sentence counts this, not `resolve`,
    /// so the count and the file pane's marks cannot disagree.
    #[prop(into)]
    marks: Signal<Option<Arc<BTreeSet<String>>>>,
    /// The plain package address on the page; the cell's own anchor in the gallery.
    #[prop(into)]
    back_href: String,
    w: Wiring,
) -> impl IntoView {
    let compared = matches!(resolve, commands::ResolveData::Compared { .. });
    let sentence = move || differs_sentence(marks.with(|m| m.as_ref().map_or(0, |m| m.len())));
    let published = match resolve {
        commands::ResolveData::Compared {
            published_message, ..
        } => view! {
            <p id=DIFFERS_ID>{sentence}</p>
            <PaneSection nested=true label="Yours">
                <RevisionRow
                    message=revision.message.clone().unwrap_or_default()
                    at=revision.obtained_at
                />
            </PaneSection>
            <PaneSection nested=true label="Published">
                // The kit draws `No message` for an empty one; no time, since
                // this copy has not obtained it.
                <RevisionRow message=published_message.unwrap_or_default() />
            </PaneSection>
        }
        .into_any(),
        commands::ResolveData::Refused { reason } => view! {
            <PaneSection nested=true label="Yours">
                <RevisionRow
                    message=revision.message.clone().unwrap_or_default()
                    at=revision.obtained_at
                />
            </PaneSection>
            <LoadFailure
                words="Could not compare the revisions."
                on_retry=Callback::new(move |()| w.reload.notify())
            />
            <p>{reason}</p>
        }
        .into_any(),
    };
    // A refused comparison cannot say what either choice costs, so neither is offered.
    let sealed = Signal::derive(move || !compared || w.busy.get());

    view! {
        <aside aria-label="About this package" class=style::root>
            <Card label="About this package">
                <BackLink href=back_href label=namespace />
                <PaneSection>
                    {published}
                    <div class=style::choices>
                        <Button
                            variant=ButtonVariant::Primary
                            wrap=true
                            disabled=sealed
                            on_click=|_| {}
                        >
                            "Make mine the shared one"
                        </Button>
                        <Button wrap=true disabled=sealed on_click=|_| {}>
                            "Replace mine with the published one"
                        </Button>
                    </div>
                </PaneSection>
            </Card>
        </aside>
    }
}

#[cfg(test)]
mod tests {
    use super::{ResolvePane, differs_sentence};
    use crate::commands::{CurrentRevisionData, ResolveData};
    use crate::kit::DIFFERS_ID;
    use crate::pages::installed_package_v2::Wiring;
    use crate::test_support::{element_saying, mount};
    use leptos::prelude::*;
    use std::cell::Cell;
    use std::collections::BTreeSet;
    use std::sync::Arc;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    #[test]
    fn the_sentence_counts_the_files_that_differ() {
        assert_eq!(
            differs_sentence(2),
            "2 files differ between these revisions — marked in the list."
        );
        assert_eq!(
            differs_sentence(1),
            "1 file differs between these revisions — marked in the list."
        );
    }

    #[test]
    fn with_nothing_marked_the_sentence_points_at_nothing() {
        let words = differs_sentence(0);
        assert_eq!(words, "No files differ between these revisions.");
        assert!(
            !words.contains("marked"),
            "{words:?} should not mention marks"
        );
    }

    const BACK: &str = "/installed-package?namespace=team%2Fdataset&filter=unmodified";

    fn revision() -> CurrentRevisionData {
        CurrentRevisionData {
            message: Some("Mine".into()),
            obtained_at: 1_758_500_000_000.0,
        }
    }

    fn compared(unpublished: usize, uncommitted: usize) -> ResolveData {
        ResolveData::Compared {
            published_message: Some("Theirs".into()),
            differing: vec!["plate/a.csv".into(), "plate/b.csv".into()],
            unpublished,
            uncommitted,
        }
    }

    fn refused() -> ResolveData {
        ResolveData::Refused {
            reason: "AccessDenied".into(),
        }
    }

    fn marks(keys: &[&str]) -> Arc<BTreeSet<String>> {
        Arc::new(keys.iter().map(ToString::to_string).collect())
    }

    fn pane(
        resolve: ResolveData,
        marked: impl Into<Option<Arc<BTreeSet<String>>>>,
        w: Wiring,
    ) -> web_sys::Element {
        let marked = marked.into();
        mount(move || {
            view! {
                <ResolvePane
                    namespace="team/dataset"
                    revision=revision()
                    resolve=resolve
                    marks=Signal::stored(marked)
                    back_href=BACK
                    w=w
                />
            }
        })
    }

    /// The `section` a nested heading names.
    fn section(el: &web_sys::Element, heading: &str) -> web_sys::Element {
        let all = el.query_selector_all("h4").unwrap();
        (0..all.length())
            .map(|i| all.item(i).unwrap().unchecked_into::<web_sys::Element>())
            .find(|h| h.text_content().unwrap_or_default().trim() == heading)
            .unwrap_or_else(|| panic!("no h4 says {heading:?}; markup was {}", el.inner_html()))
            .closest("section")
            .unwrap()
            .expect("the heading's section")
    }

    fn has_heading(el: &web_sys::Element, heading: &str) -> bool {
        let all = el.query_selector_all("h4").unwrap();
        (0..all.length()).any(|i| {
            all.item(i)
                .unwrap()
                .text_content()
                .unwrap_or_default()
                .trim()
                == heading
        })
    }

    /// The button whose text is `label`, as `header.rs`'s helper finds one.
    fn button(el: &web_sys::Element, label: &str) -> web_sys::HtmlButtonElement {
        let all = el.query_selector_all("button").unwrap();
        (0..all.length())
            .map(|i| all.item(i).unwrap().unchecked_into::<web_sys::Element>())
            .find(|b| b.text_content().unwrap_or_default().trim() == label)
            .unwrap_or_else(|| panic!("no button says {label:?}; markup was {}", el.inner_html()))
            .unchecked_into()
    }

    const CERTIFY: &str = "Make mine the shared one";
    const REPLACE: &str = "Replace mine with the published one";

    #[wasm_bindgen_test]
    fn the_mode_names_both_sides() {
        let el = pane(
            compared(0, 0),
            marks(&["plate/a.csv", "plate/b.csv"]),
            Wiring::new(),
        );

        let sentence = element_saying(
            &el,
            "2 files differ between these revisions — marked in the list.",
        );
        assert_eq!(sentence.id(), DIFFERS_ID);

        let yours = section(&el, "Yours");
        assert_eq!(yours.query_selector_all("time").unwrap().length(), 1);
        element_saying(&yours, "\u{201c}Mine\u{201d}");

        let published = section(&el, "Published");
        element_saying(&published, "\u{201c}Theirs\u{201d}");
        assert!(
            published.query_selector("time").unwrap().is_none(),
            "the published side has no time; markup was {}",
            published.inner_html()
        );

        let back = el
            .query_selector(&format!("a[href='{BACK}']"))
            .unwrap()
            .expect("the back link");
        assert_eq!(
            back.text_content().unwrap_or_default().trim(),
            "team/dataset"
        );

        for label in [CERTIFY, REPLACE] {
            assert!(!button(&el, label).disabled(), "{label} is enabled");
        }
    }

    #[wasm_bindgen_test]
    fn a_published_revision_with_no_message_says_so() {
        let resolve = ResolveData::Compared {
            published_message: None,
            differing: vec!["plate/a.csv".into()],
            unpublished: 0,
            uncommitted: 0,
        };
        let el = pane(resolve, marks(&["plate/a.csv"]), Wiring::new());
        element_saying(&section(&el, "Published"), "No message");
    }

    /// Pins the seam: the count reads the page's set, not the payload's.
    #[wasm_bindgen_test]
    fn the_count_is_the_page_s_set_not_the_payload_s() {
        let resolve = ResolveData::Compared {
            published_message: Some("Theirs".into()),
            differing: vec![
                "plate/a.csv".into(),
                "plate/b.csv".into(),
                "plate/c.csv".into(),
            ],
            unpublished: 0,
            uncommitted: 0,
        };
        let el = pane(
            resolve,
            marks(&["plate/a.csv", "plate/b.csv"]),
            Wiring::new(),
        );
        element_saying(
            &el,
            "2 files differ between these revisions — marked in the list.",
        );
    }

    #[wasm_bindgen_test]
    fn a_refused_comparison_keeps_yours_and_disables_both_choices() {
        let w = Wiring::new();
        let el = pane(refused(), None, w);
        element_saying(&el, "Could not compare the revisions.");
        element_saying(&el, "AccessDenied");
        section(&el, "Yours");
        assert!(
            !has_heading(&el, "Published"),
            "markup was {}",
            el.inner_html()
        );
        button(&el, "Try again");
        assert!(!w.busy.get_untracked());
        for label in [CERTIFY, REPLACE] {
            assert!(button(&el, label).disabled(), "{label} is disabled");
        }
    }

    #[wasm_bindgen_test]
    async fn try_again_re_reads_the_page() {
        thread_local! {
            static RELOADS: Cell<u32> = const { Cell::new(0) };
        }
        let w = Wiring::new();
        let el = mount(move || {
            Effect::new(move |seen: Option<()>| {
                w.reload.track();
                // The first run is the subscription, not a re-read.
                if seen.is_some() {
                    RELOADS.set(RELOADS.get() + 1);
                }
            });
            view! {
                <ResolvePane
                    namespace="team/dataset"
                    revision=revision()
                    resolve=refused()
                    marks=Signal::stored(None)
                    back_href=BACK
                    w=w
                />
            }
        });
        leptos::task::tick().await;
        button(&el, "Try again").click();
        leptos::task::tick().await;
        assert_eq!(RELOADS.get(), 1);
    }

    #[wasm_bindgen_test]
    async fn while_another_command_runs_both_choices_are_sealed() {
        let w = Wiring::new();
        let el = pane(compared(0, 0), marks(&["plate/a.csv"]), w);
        w.busy.set(true);
        leptos::task::tick().await;
        for label in [CERTIFY, REPLACE] {
            assert!(button(&el, label).disabled(), "{label} is disabled");
        }
    }

    #[wasm_bindgen_test]
    fn both_choices_may_take_two_lines() {
        let el = pane(compared(0, 0), marks(&["plate/a.csv"]), Wiring::new());
        for label in [CERTIFY, REPLACE] {
            assert!(
                button(&el, label).has_attribute("data-wrap"),
                "{label} wraps"
            );
        }
    }
}
