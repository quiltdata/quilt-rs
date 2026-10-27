//! The queue's status line: one line where a region would be, and the heading of
//! the rows when there are any.
//!
//! # Why it is not a `Blankslate`
//!
//! With autosync working the all-clear is the common case, seen most days by most
//! users — so it is the state that decides whether the page reads as calm or as
//! unfinished. A full-height empty state here would push the package list below the
//! fold to announce that nothing is wrong, which is the loudest possible way to say it.
//!
//! Acceptance criterion 8: it **must stay one line**.

use leptos::prelude::*;

use super::SkeletonBox;
use super::state_label::StateTone;
use super::unique_id;

stylance::import_crate_style!(style, "src/kit/zero_line.module.scss");

#[component]
pub fn ZeroLine(
    /// The whole line — `All packages are up to date`. One string rather than text
    /// plus a count, because the singular case is not a plural rule the component
    /// can apply.
    #[prop(into)]
    text: TextProp,
    /// Sets the glyph and its colour.
    #[prop(default = StateTone::Success)]
    tone: StateTone,
    /// Draws the line as a heading with this id, for a list it names.
    #[prop(optional, into)]
    id: Option<String>,
) -> impl IntoView {
    let class = format!("{} {}", style::root, tone_class(tone));
    // The tone's own glyph, from `StateTone`, rather than a second one drawn here.
    // Two components drawing two different ticks is how a silhouette stops being
    // a signal.
    let body = view! {
        <span class=style::icon>{tone.glyph()}</span>
        <span class=style::text>{move || text.get()}</span>
    };
    match id {
        Some(id) => view! { <h2 id=id class=class>{body}</h2> }.into_any(),
        None => view! { <p class=class>{body}</p> }.into_any(),
    }
}

fn tone_class(tone: StateTone) -> &'static str {
    match tone {
        StateTone::Success => style::success,
        StateTone::Neutral => style::neutral,
        StateTone::Attention => style::attention,
        StateTone::Danger => style::danger,
    }
}

/// The queue with rows: an attention line naming the list, then the list.
///
/// The children are the list's items, `li`s. A hairline is drawn above each one,
/// so the first sits under the line.
#[component]
pub fn AttentionQueue(
    /// The line's text — `3 packages need your attention`.
    #[prop(into)]
    text: TextProp,
    children: Children,
) -> impl IntoView {
    let heading_id = unique_id("queue-title");
    let labelled_by = heading_id.clone();
    view! {
        <section aria-labelledby=labelled_by>
            <ZeroLine text=text tone=StateTone::Attention id=heading_id />
            <ul class=style::rows role="list">
                {children()}
            </ul>
        </section>
    }
}

/// The line's geometry, while the page is still being read.
///
/// It reserves the queue's space, so the list below does not start high and jump
/// down when the queue arrives.
///
/// Drawn until every package's check has answered. The line is the busy region
/// (`aria-busy`); its bars are `aria-hidden`, per `skeleton_box.rs`.
#[component]
pub fn ZeroLineSkeleton() -> impl IntoView {
    view! {
        <p class=style::placeholder aria-busy="true">
            <span class=style::icon>
                <SkeletonBox width="14px" height="14px" />
            </span>
            <span class=style::text>
                <SkeletonBox width="240px" />
            </span>
        </p>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use wasm_bindgen_test::*;

    /// A `section` with no accessible name is not a region at all, so the queue
    /// points at its line.
    #[wasm_bindgen_test]
    fn the_attention_line_names_the_queue() {
        let el = mount(|| {
            view! {
                <AttentionQueue text="1 package needs your attention">
                    <li>"row"</li>
                </AttentionQueue>
            }
        });
        let section = el.query_selector("section").unwrap().expect("a section");
        let id = section
            .get_attribute("aria-labelledby")
            .expect("the queue names itself");
        let heading = el
            .query_selector(&format!("#{id}"))
            .unwrap()
            .expect("the id it points at");
        assert_eq!(heading.tag_name(), "H2");
        assert_eq!(
            heading.text_content().as_deref(),
            Some("1 package needs your attention")
        );
        assert_eq!(
            el.query_selector("ul[role=list] > li")
                .unwrap()
                .and_then(|li| li.text_content())
                .as_deref(),
            Some("row"),
        );
    }

    /// The all-clear is a sentence, not a heading over nothing.
    #[wasm_bindgen_test]
    fn the_all_clear_is_not_a_heading() {
        let el = mount(|| view! { <ZeroLine text="All packages are up to date" /> });
        assert!(el.query_selector("h2").unwrap().is_none());
        assert!(el.query_selector("p").unwrap().is_some());
    }
}
