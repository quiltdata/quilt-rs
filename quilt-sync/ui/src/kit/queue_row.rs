//! One package needing a decision.
//!
//! # The only row in the design that carries a text button
//!
//! And it is the payoff for the rule that stripped buttons off the list rows, not an
//! exception to it. The rule is that an action appears **where its condition is
//! true**: a queue row exists *because* the package needs the action, so the button
//! and the row are the same fact. On today's list, `Publish` renders on 43 rows and
//! applies to two — the other 41 are disabled chrome the user has to interpret.
//!
//! # Inert apart from its action
//!
//! The row does not navigate. The list below is where you go to a package; the queue
//! is where you decide about one, and a row that both navigated and carried a button
//! would put two meanings on one target. That also keeps the tab order honest — one
//! stop per row, and it is the button.

use leptos::prelude::*;

use super::SkeletonBox;
use super::StateLabel;
use super::state_label::StateTone;

stylance::import_crate_style!(style, "src/kit/queue_row.module.scss");

#[component]
pub fn QueueRow(
    /// `owner/name`. Truncates right, as in `PackageRow`.
    #[prop(into)]
    namespace: String,
    /// The state and its tone. **Both or neither** — the label renders only when both
    /// are given, which is what stops a tone from being set without the words that
    /// carry the meaning.
    ///
    /// Neither, for a sub-row: expanding `Signed out — 11 packages` answers *which*
    /// packages, and repeating `Signed out` on all eleven is exactly the redundancy
    /// the cause row above exists to remove.
    #[prop(optional, into)]
    state: Option<String>,
    #[prop(optional, into)] tone: Option<StateTone>,
    /// The one thing to do — a `Button`, passed in rather than named, because the row
    /// has no business knowing whether `Publish` is primary here. Absent on a
    /// sub-row, whose action belongs to the cause above it.
    ///
    /// `optional_no_strip`, like `detail`: the caller decides whether a state names
    /// an operation and already holds the `Option` that answer comes in.
    #[prop(optional_no_strip)]
    action: Option<AnyView>,
    /// Indented, as one of the packages revealed by an expanded `CauseRow`.
    #[prop(optional)]
    sub: bool,
    /// A second line under the first, for a state whose account of itself is longer
    /// than a label. Engine prose, so it is shown verbatim and its line breaks kept.
    ///
    /// `optional_no_strip`, like `MainPageRegions::on_store`: the queue's own caller
    /// already holds an `Option`, and the plain form would make the builder demand a
    /// bare `String`.
    #[prop(optional_no_strip)]
    detail: Option<String>,
) -> impl IntoView {
    let class = if sub {
        format!("{} {}", style::root, style::sub)
    } else {
        style::root.to_string()
    };

    // Ellipsised by the stylesheet, so the whole value rides in `title`.
    let full_namespace = namespace.clone();
    view! {
        <div class=class>
            <div class=style::line>
                // The list bullet, filling the column `CauseRow` uses for its expander.
                // Empty of text, so it says nothing to a screen reader — the row's own
                // words are the content and a bullet is not one of them.
                <span class=style::bullet></span>
                <span class=style::namespace title=full_namespace>{namespace}</span>
                {state
                    .zip(tone)
                    .map(|(state, tone)| view! { <StateLabel tone=tone>{state}</StateLabel> })}
                {action.map(|action| view! { <span class=style::action>{action}</span> })}
            </div>
            {detail.map(|detail| view! { <p class=style::detail>{detail}</p> })}
        </div>
    }
}

/// The same row with its content unknown. Shares `.root` for the reason
/// [`PackageRowSkeleton`](super::PackageRowSkeleton) does — equal height, by construction
/// rather than by copying numbers.
///
/// The bullet stays. It is structure rather than data: it holds the column `CauseRow`
/// uses for its expander, and a skeleton that dropped it would shift every row sideways
/// when the real content arrived.
#[component]
pub fn QueueRowSkeleton() -> impl IntoView {
    view! {
        <div class=style::root>
            <div class=style::line>
                <span class=style::bullet></span>
                <span class=style::namespace>
                    <SkeletonBox width="32%" />
                </span>
                <SkeletonBox width="120px" height="22px" />
                <span class=style::action>
                    <SkeletonBox width="76px" height="32px" />
                </span>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    /// `main_page.rs`'s pattern: mount a view into a fresh, attached `div` and
    /// hand back the element to query against.
    fn mount<N: IntoView + 'static>(f: impl FnOnce() -> N + 'static) -> web_sys::Element {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        leptos::mount::mount_to(container.clone(), f).forget();
        container.into()
    }

    /// A queue row asks the reader to act on this package, so its name must survive.
    #[wasm_bindgen_test]
    fn the_truncating_namespace_carries_its_whole_value() {
        let el = mount(
            || view! { <QueueRow namespace="a-long-owner-name/a-much-longer-package-name-than-the-column" /> },
        );
        let span = el
            .query_selector("[class*=namespace]")
            .unwrap()
            .expect("the namespace");
        assert_eq!(
            span.get_attribute("title").as_deref(),
            Some("a-long-owner-name/a-much-longer-package-name-than-the-column")
        );
    }
}
