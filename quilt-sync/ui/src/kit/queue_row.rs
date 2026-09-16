//! One package needing a decision.
//!
//! # The row is the link
//!
//! Every state that names an operation names a page that performs it — `Publish`
//! opens the commit page, `Resolve` the merge page, the other two the package's own —
//! so a row has exactly one destination and the whole row goes there. The verb rides
//! along as text rather than as a button, because a button promises the operation
//! happens on press and none of these do. One tab stop per row either way, and the
//! accent stays free for a page that has something to spend it on.
//!
//! A state that names no operation — a refused sync, a denial, a package this build
//! cannot read — has nowhere to send anyone, and its row stays inert.
//!
//! # The state reads as a clause
//!
//! `org/dataset-c` then `has conflicts in 2 files`, rather than a name beside a chip.
//! The words are [`render`](super::render)'s at [`Site::QueueRow`](super::Site), which
//! is where the queue's grammar is chosen; this file only draws them.
//!
//! The tone the chip used to carry moves to a rule on the row's edge, which is what
//! makes a column of rows scannable. Colour is all it is, and that is enough here
//! precisely because it carries nothing: the clause states the row's state in words,
//! so two rows of different severity always read differently with the colour taken
//! away. The chip needed a glyph beside it because its words were a short label
//! doing the same job as its tint; these words are the whole account.

use leptos::prelude::*;

use super::PackageAction;
use super::SkeletonBox;
use super::state_label::StateTone;

stylance::import_crate_style!(style, "src/kit/queue_row.module.scss");

/// What to do about a row, and where doing it happens.
///
/// One value rather than two props, because a row has somewhere to go exactly when
/// it has an operation on offer — as two independent `Option`s they could disagree,
/// and a link with no verb or a verb with no link is a row that lies.
///
/// The verb is a [`PackageAction`] and not its label, so the words stay the
/// vocabulary's. A caller free to pass a string is a caller free to put `Resolve` on
/// a pull conflict, which is the one pairing the design record corrects twice.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Remedy {
    pub action: PackageAction,
    /// The page that performs it, already resolved — routes belong to the page, not
    /// to the kit.
    pub href: String,
}

/// The tone's own class, which sets the edge rule's and the glyph's colour.
fn tone_class(tone: StateTone) -> &'static str {
    match tone {
        StateTone::Success => style::success,
        StateTone::Neutral => style::neutral,
        StateTone::Attention => style::attention,
        StateTone::Danger => style::danger,
    }
}

#[component]
pub fn QueueRow(
    /// `owner/name`. Truncates right, as in `PackageRow`.
    #[prop(into)]
    namespace: String,
    /// The state's words and its tone. **Both or neither** — a tone with no words is
    /// a colour that means nothing, and words with no tone have no edge to sit on.
    ///
    /// Neither, for a sub-row: expanding `Signed out — 11 packages` answers *which*
    /// packages, and repeating `Signed out` on all eleven is exactly the redundancy
    /// the cause row above exists to remove.
    #[prop(optional, into)]
    state: Option<String>,
    #[prop(optional, into)] tone: Option<StateTone>,
    /// What to do and where. Absent when the state names no operation, and the row is
    /// then not a link.
    ///
    /// `optional_no_strip`, like `detail`: the caller decides whether a state names an
    /// operation and already holds the `Option` that answer comes in.
    #[prop(optional_no_strip)]
    remedy: Option<Remedy>,
    /// Indented, as one of the packages revealed by an expanded `CauseRow`.
    #[prop(optional)]
    sub: bool,
    /// A second line under the first, for a state whose account of itself is longer
    /// than a label. Engine prose, so it is shown verbatim and its line breaks kept.
    ///
    /// Reactive: it arrives on a payload of its own, after the row is drawn.
    #[prop(optional, into)]
    detail: MaybeProp<String>,
) -> impl IntoView {
    // Both or neither, enforced here rather than at each of the three places the pair
    // draws — the glyph, the edge rule and the clause.
    let (state, tone) = state.zip(tone).unzip();

    // Split once: the verb draws inside the row and the destination wraps it, and
    // neither exists without the other.
    let (verb, href) = match remedy {
        Some(Remedy { action, href }) => (Some(action), Some(href)),
        None => (None, None),
    };

    let mut class = String::from(style::root);
    if sub {
        class.push(' ');
        class.push_str(style::sub);
    }
    if let Some(tone) = tone {
        class.push(' ');
        class.push_str(tone_class(tone));
    }
    if href.is_some() {
        class.push(' ');
        class.push_str(style::linked);
    }

    // The column `CauseRow` fills with its expander, held open so a cause and a
    // package start their text at the same x.
    //
    // Empty on a row that has a state: the edge rule marks it, the clause names it,
    // and a glyph or a bullet here would be a third marker saying nothing the other
    // two do not. A row that is only a name gets a bullet, which is the one case
    // where a list marker is the only marker there is.
    let bullet = match tone {
        Some(_) => view! { <span class=style::bullet></span> }.into_any(),
        None => {
            view! { <span class=format!("{} {}", style::bullet, style::dot)></span> }.into_any()
        }
    };

    // Ellipsised by the stylesheet, so the whole value rides in `title`.
    let full_namespace = namespace.clone();
    let line = view! {
        {bullet}
        <span class=style::namespace title=full_namespace>{namespace}</span>
        {state.map(|state| view! { <span class=style::clause>{state}</span> })}
        {verb
            .map(|verb| {
                view! {
                    <span class=style::action>
                        {verb.label()}
                        // Decoration: the link is already announced as one, and
                        // "right arrow" after every verb is noise.
                        <span aria-hidden="true">"\u{2192}"</span>
                    </span>
                }
            })}
    };

    // The ROW is the link, not the line inside it: `PackageRow` is an anchor at its
    // root for the same reason, and the two kinds of row share a region. An anchor
    // around the line alone would leave the row's own padding and its tone rule
    // outside the target, so the tint would blink off between rows and a click in
    // the gap would do nothing.
    let body = view! {
        <div class=style::line>{line}</div>
        {move || detail.get().map(|detail| view! { <p class=style::detail>{detail}</p> })}
    };

    view! {
        {match href {
            Some(href) => view! { <a class=class href=href>{body}</a> }.into_any(),
            None => view! { <div class=class>{body}</div> }.into_any(),
        }}
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
                <span class=format!("{} {}", style::bullet, style::dot)></span>
                // A width in px and not a percentage: `.namespace` is `flex: none`,
                // so the span sizes to this box and a percentage would have nothing
                // to resolve against — it collapses the column to nothing.
                <span class=style::namespace>
                    <SkeletonBox width="120px" />
                </span>
                <span class=style::clause>
                    <SkeletonBox width="55%" />
                </span>
                <span class=style::action>
                    <SkeletonBox width="76px" height="20px" />
                </span>
            </div>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kit::PackageAction;
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

    /// The row is the link, so the whole of it goes to the page that fixes the state
    /// — and the verb still says which page that is.
    #[wasm_bindgen_test]
    fn a_row_with_a_remedy_is_a_link_to_the_page_that_fixes_it() {
        let el = mount(|| {
            view! {
                <QueueRow
                    namespace="org/dataset-c"
                    state="has conflicts in 2 files"
                    tone=StateTone::Danger
                    remedy=Some(Remedy {
                        action: PackageAction::Publish,
                        href: "/commit?namespace=org/dataset-c".to_string(),
                    })
                />
            }
        });
        let link = el
            .query_selector("a")
            .unwrap()
            .expect("the row is the link");
        assert_eq!(
            link.get_attribute("href").as_deref(),
            Some("/commit?namespace=org/dataset-c")
        );
        assert!(
            link.text_content().unwrap().contains("Publish"),
            "the remedy is named, not only linked"
        );
    }

    /// Nothing in the app restarts a sync the remote refused, so this row has
    /// nowhere to go and must not offer to take the reader anywhere.
    #[wasm_bindgen_test]
    fn a_row_with_no_remedy_is_not_a_link() {
        let el = mount(|| {
            view! {
                <QueueRow
                    namespace="team/imaging-cohort-b"
                    state="has stopped syncing"
                    tone=StateTone::Danger
                />
            }
        });
        assert!(
            el.query_selector("a").unwrap().is_none(),
            "a row with no operation on offer is inert"
        );
    }

    /// The edge rule is the tone's only channel, and it hangs off this class.
    /// Dropping it leaves a row that reads correctly — the clause says everything —
    /// and loses the mark that makes a column of them scannable.
    #[wasm_bindgen_test]
    fn the_tone_also_reaches_the_row_that_draws_its_edge_rule() {
        let el = mount(|| {
            view! {
                <QueueRow
                    namespace="org/dataset-c"
                    state="has conflicts in 2 files"
                    tone=StateTone::Danger
                />
            }
        });
        let root = el.first_element_child().expect("the row");
        let class = root.get_attribute("class").unwrap_or_default();
        assert!(
            class.contains(style::danger),
            "the edge rule hangs off the tone class, and it is not on the row: {class}"
        );
    }

    /// The leading column holds a list marker only where there is nothing else to
    /// mark the row: a package revealed by an expanded cause is a bare name, while a
    /// row with a state has an edge rule and a clause already.
    #[wasm_bindgen_test]
    fn only_a_bare_name_carries_a_bullet() {
        let named = mount(|| view! { <QueueRow namespace="user/package-x" sub=true /> });
        assert!(
            named
                .query_selector(&format!("[class*={}]", style::dot))
                .unwrap()
                .is_some(),
            "a package under a cause is one of a set of names, and a bullet is what \
             marks one"
        );

        let stated = mount(|| {
            view! {
                <QueueRow
                    namespace="org/dataset-c"
                    state="has conflicts in 2 files"
                    tone=StateTone::Danger
                />
            }
        });
        assert!(
            stated
                .query_selector(&format!("[class*={}]", style::dot))
                .unwrap()
                .is_none(),
            "a third marker beside the edge rule and the clause"
        );
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
