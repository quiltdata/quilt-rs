//! A page's header: a way up, the name, an optional label, and the actions.
//!
//! The geometry lives here and the contents stay with the pages, which fill
//! the slots. The name truncates, so a long one cannot change the height.

use leptos::prelude::*;

use super::SkeletonBox;

stylance::import_crate_style!(style, "src/kit/page_header.module.scss");

#[component]
pub fn PageHeader(
    /// A [`BackLink`](super::BackLink), or a [`Trail`] two levels down.
    trail: AnyView,
    #[prop(into)] title: String,
    /// After the name, such as a state label.
    #[prop(optional)]
    label: Option<AnyView>,
    /// Primary first.
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

/// The header's shape while its page loads, from the same stylesheet.
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

/// The way up from two levels down, such as *Packages › ns*. It stops short
/// of the page itself, whose name is the title.
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

    /// The separators are never read aloud.
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
