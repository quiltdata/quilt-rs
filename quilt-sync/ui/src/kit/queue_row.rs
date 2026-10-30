//! One package needing a decision.
//!
//! # The row is the link
//!
//! Every state that names an operation names a page that performs it — `Resolve` opens
//! the package page's resolve mode, the others the package's own page —
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
//! The row draws no tone. The clause states the row's state in words, which is the
//! whole account; the chip needed a tint and a glyph because its words were a short
//! label.

use leptos::prelude::*;

use super::PackageAction;
use super::SkeletonBox;

stylance::import_crate_style!(style, "src/kit/queue_row.module.css");

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

#[component]
pub fn QueueRow(
    /// `owner/name`. Truncates right, as in `PackageRow`.
    #[prop(into)]
    namespace: String,
    /// The state's words. None for a sub-row: expanding `Signed out — 11 packages`
    /// answers *which* packages, and repeating `Signed out` on all eleven is exactly
    /// the redundancy the cause row above exists to remove.
    #[prop(optional, into)]
    state: Option<String>,
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
    if href.is_some() {
        class.push(' ');
        class.push_str(style::linked);
    }

    // The column `CauseRow` fills with its expander, held open so a cause and a
    // package start their text at the same x.
    //
    // No bullet on a row that has a state: the clause marks it. A row that is only a
    // name gets a bullet, which is the one case where a list marker is the only
    // marker there is.
    let bullet = match state {
        Some(_) => view! { <span class=style::bullet></span> }.into_any(),
        None => {
            view! { <span class=format!("{} {}", style::bullet, style::dot)></span> }.into_any()
        }
    };

    // Ellipsised by the stylesheet, so the whole value rides in `title`.
    let full_namespace = namespace.clone();
    let line = view! {
        {bullet}
        <span class=style::subject>
            <span class=style::namespace title=full_namespace>{namespace}</span>
            {state.map(|state| view! { <span class=style::clause>{state}</span> })}
        </span>
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
    // around the line alone would leave the row's own padding outside the target, so the tint would blink off between rows and a click in
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
                <span class=style::subject>
                    <span class=style::namespace>
                        <SkeletonBox width="120px" />
                    </span>
                    <span class=style::clause>
                        <SkeletonBox width="55%" />
                    </span>
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
    use crate::test_support::{element_saying, mount};
    use wasm_bindgen_test::*;

    /// The row is the link, so the whole of it goes to the page that fixes the state
    /// — and the verb still says which page that is.
    #[wasm_bindgen_test]
    fn a_row_with_a_remedy_is_a_link_to_the_page_that_fixes_it() {
        let el = mount(|| {
            view! {
                <QueueRow
                    namespace="org/dataset-c"
                    state="has conflicts in 2 files"
                    remedy=Some(Remedy {
                        action: PackageAction::Publish,
                        href: "/installed-package?namespace=org%2Fdataset-c&filter=unmodified".to_string(),
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
            Some("/installed-package?namespace=org%2Fdataset-c&filter=unmodified")
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
                />
            }
        });
        assert!(
            el.query_selector("a").unwrap().is_none(),
            "a row with no operation on offer is inert"
        );
    }

    /// The leading column holds a list marker only where there is nothing else to
    /// mark the row: a package revealed by an expanded cause is a bare name, while a
    /// row with a state has its clause.
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
                />
            }
        });
        assert!(
            stated
                .query_selector(&format!("[class*={}]", style::dot))
                .unwrap()
                .is_none(),
            "a second marker beside the clause"
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

    /// A queue row `width` px wide, with its stylesheet loaded: the name's and
    /// the clause's boxes.
    async fn drawn(width: u32, name: &'static str) -> (web_sys::DomRect, web_sys::HtmlElement) {
        crate::test_support::load_module_style(
            include_str!("queue_row.module.css"),
            &[
                ("root", style::root),
                ("line", style::line),
                ("linked", style::linked),
                ("detail", style::detail),
                ("bullet", style::bullet),
                ("dot", style::dot),
                ("subject", style::subject),
                ("namespace", style::namespace),
                ("clause", style::clause),
                ("action", style::action),
                ("sub", style::sub),
            ],
        );
        let el = mount(move || {
            view! {
                <div style=format!("width:{width}px")>
                    <QueueRow
                        namespace=name
                        state="has conflicts in 2 files"
                        remedy=Some(Remedy {
                            action: PackageAction::Publish,
                            href: "/installed-package".to_string(),
                        })
                    />
                </div>
            }
        });
        crate::test_support::sleep_ms(0).await;
        let name = element_saying(&el, name).get_bounding_client_rect();
        (name, element_saying(&el, "has conflicts in 2 files"))
    }

    /// The clause stays beside a name it fits beside.
    #[wasm_bindgen_test]
    async fn the_clause_sits_beside_a_short_name() {
        let (name, clause) = drawn(560, "org/dataset-c").await;
        let line = clause.get_bounding_client_rect();
        assert!(line.left() >= name.right(), "beside the name");
        assert!(line.top() < name.bottom(), "on its line");
    }

    /// A name that leaves no room puts the clause on a line of its own, whole.
    #[wasm_bindgen_test]
    async fn the_clause_moves_under_a_long_name_whole() {
        let (name, clause) = drawn(
            420,
            "a-team-with-a-long-name/another-analysis-package-whose-name-runs-on",
        )
        .await;
        let line = clause.get_bounding_client_rect();
        // Within half a pixel: far down the test page the two edges round
        // apart by a fraction.
        assert!(line.top() >= name.bottom() - 0.5, "under the name");
        assert!(
            clause.scroll_width() <= clause.client_width(),
            "and not cut: {} of {}",
            clause.client_width(),
            clause.scroll_width()
        );
    }
}
