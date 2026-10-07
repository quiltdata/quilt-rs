//! A page's header: a way up, the page's name, an optional state, and the
//! page's actions on the trailing side.
//!
//! # Slots, because the pages differ in what fills them
//!
//! The installed-package page fills it with a `BackLink`, the namespace, the
//! package's state label and a state-driven primary; the commit page with a
//! [`Trail`], `New revision`, no label, and its publish button. The geometry is
//! what they share — one row, the name truncating rather than wrapping, the
//! actions pushed to the end with one uniform gap — so the geometry is what
//! lives here and the words stay with the pages.
//!
//! It was page-local while one page drew it. A second page is what promoted it:
//! two copies of a row are two rows that drift.
//!
//! # The name truncates
//!
//! A long namespace must not push the controls onto a second line and change
//! the header's height with the package. The row still wraps as a floor for
//! narrow windows: the widest arrangement was measured at 609px against the 992
//! the page has at 1024.

use leptos::prelude::*;

use super::SkeletonBox;

stylance::import_crate_style!(style, "src/kit/page_header.module.scss");

#[component]
pub fn PageHeader(
    /// The way up: a [`BackLink`](super::BackLink), or a [`Trail`] when the
    /// page sits two levels down.
    trail: AnyView,
    /// The page's name, as its `h2`.
    #[prop(into)]
    title: String,
    /// Drawn right after the name: the installed-package page's state label.
    #[prop(optional)]
    label: Option<AnyView>,
    /// The trailing controls, primary first.
    actions: AnyView,
) -> impl IntoView {
    view! {
        <div class=style::root>
            {trail}
            <div class=style::row>
                <h2 class=style::name>{title}</h2>
                {label}
                <div class=style::actions>{actions}</div>
            </div>
        </div>
    }
}

/// The header's shape while its page is read.
///
/// Built from the same stylesheet as the header itself, so the gap between the
/// two bands is the real one and the height it reserves cannot drift from the
/// header's. A hard-coded pixel height would agree with the header only until
/// somebody changed a control.
#[component]
pub fn PageHeaderSkeleton() -> impl IntoView {
    view! {
        <div class=style::root>
            <SkeletonBox width="88px" height="16px" />
            <div class=style::row>
                <SkeletonBox width="240px" height="var(--q-control-height)" />
            </div>
        </div>
    }
}

/// The way up from a page two levels down: each level above, by its own name.
///
/// `BackLink` argues breadcrumbs earn themselves at three levels, and this is
/// that case: the commit page sits under a package, which sits under the
/// packages. The page's own name is the header's title, so the trail stops
/// one short of it rather than restating it an inch above itself.
///
/// The same accent links as `BackLink`, without its chevron: the separators
/// already say which way is up.
#[component]
pub fn Trail(
    /// `(href, label)`, outermost first.
    crumbs: Vec<(String, String)>,
) -> impl IntoView {
    let last = crumbs.len().saturating_sub(1);
    view! {
        <nav class=style::trail aria-label="Breadcrumb">
            <ol class=style::crumbs>
                {crumbs
                    .into_iter()
                    .enumerate()
                    .map(|(i, (href, label))| {
                        view! {
                            <li class=style::crumb>
                                <a class=style::link href=href>
                                    {label}
                                </a>
                                {(i < last)
                                    .then(|| {
                                        view! {
                                            <span class=style::separator aria-hidden="true">
                                                "›"
                                            </span>
                                        }
                                    })}
                            </li>
                        }
                    })
                    .collect_view()}
            </ol>
        </nav>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use wasm_bindgen_test::*;

    /// The trail reads as its links and nothing else: the separators are drawn
    /// and never read aloud.
    #[wasm_bindgen_test]
    fn the_trail_reads_as_its_links() {
        let el = mount(|| {
            view! {
                <Trail crumbs=vec![
                    ("/".to_string(), "Packages".to_string()),
                    ("/p".to_string(), "team/data".to_string()),
                ] />
            }
        });
        let links = el.query_selector_all("a").unwrap();
        assert_eq!(links.length(), 2);
        let separators = el.query_selector_all("[aria-hidden='true']").unwrap();
        assert_eq!(separators.length(), 1, "one between two, none trailing");
    }
}
