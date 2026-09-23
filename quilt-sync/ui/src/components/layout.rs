use leptos::prelude::*;

use super::appbar::appbar_actions;
use super::buttons;
use crate::kit::Appbar;
use crate::theme;

/// Notification variant for the layout notification bar.
#[derive(Clone)]
pub enum Notification {
    Success(String),
    /// A soft warning: the operation succeeded but something needs the user's
    /// attention (e.g. the remote was set but its default workflow could not be
    /// resolved).
    Warning(String),
    Error(String),
}

/// Breadcrumb link item (navigates to a page).
#[derive(Clone)]
pub struct BreadcrumbLink {
    pub href: String,
    pub title: String,
}

/// Breadcrumb items for the toolbar.
#[derive(Clone)]
pub enum BreadcrumbItem {
    /// Navigable link.
    Link(BreadcrumbLink),
    /// Current page (non-navigable).
    Current(String),
}

/// Top-level page layout.
///
/// Provides: app bar, toolbar with breadcrumbs and optional actions,
/// notification area, content slot (children), and popup overlay.
///
/// With the design preview on, the app bar is the kit [`Appbar`]; the toolbar
/// stays v1's either way.
#[component]
pub fn Layout(
    breadcrumbs: Vec<BreadcrumbItem>,
    notification: RwSignal<Option<Notification>>,
    /// Optional toolbar actions rendered to the right of breadcrumbs.
    #[prop(optional)]
    actions: Option<ToolbarActions>,
    /// When `true`, the layout shows a disabled overlay (progress indicator).
    #[prop(optional)]
    ui_locked: Option<RwSignal<bool>>,
    /// Keeps v1's bar under the preview. For a screen whose mount is its
    /// operation, where Refresh's window reload would repeat it. Inherited by
    /// nested `Layout`s, such as the error page's.
    #[prop(optional)]
    transit: bool,
    children: Children,
) -> impl IntoView {
    let transit = transit || use_context::<TransitScreen>().is_some();
    if transit {
        provide_context(TransitScreen);
    }
    let appbar = if !transit && theme::is_v2() {
        view! {
            <div class="layout-appbar layout-appbar-v2">
                <Appbar actions=Some(appbar_actions(reload_window, false.into())) />
            </div>
        }
        .into_any()
    } else {
        view! {
            <div class="qui-appbar layout-appbar">
                <div class="container">
                    <a class="qui-logo" href="/">
                        <img class="img" src="/assets/img/quilt.png" />
                    </a>
                    <div class="nav">
                        <buttons::Refresh on_click=move |_| reload_window() />
                        <buttons::Settings />
                    </div>
                </div>
            </div>
        }
        .into_any()
    };

    view! {
        <div
            class="qui-layout"
            id="layout"
            class:disabled=move || {
                ui_locked.is_some_and(|s| s.get())
            }
        >
            // ── App bar ──
            {appbar}

            // ── Toolbar ──
            <div class="layout-toolbar qui-toolbar">
                <div class="container">
                    <Breadcrumbs items=breadcrumbs />
                    {actions.map(|a| view! {
                        <div class="actions">
                            <ul class="secondary-actions">
                                {(a.0)()}
                            </ul>
                        </div>
                    })}
                </div>
            </div>

            // ── Notification dismiss overlay ──
            {move || {
                if notification.get().is_some() {
                    Some(view! {
                        <div
                            class="popup-overlay"
                            on:click=move |_| notification.set(None)
                        ></div>
                    })
                } else {
                    None
                }
            }}

            // ── Notification bar ──
            <div class="qui-notify">
                <div id="notify" class="root">
                    {move || notification.get().map(|n| match n {
                        Notification::Success(msg) => view! {
                            <div class="js-success success">{msg}</div>
                        }.into_any(),
                        Notification::Warning(msg) => view! {
                            <div class="warning">{msg}</div>
                        }.into_any(),
                        Notification::Error(msg) => view! {
                            <div class="error">{msg}</div>
                        }.into_any(),
                    })}
                </div>
            </div>

            // ── Page content ──
            {children()}

            // ── Popup overlay ──
            <div class="qui-popup">
                <div id="popup" class="root"></div>
            </div>
        </div>
    }
}

#[derive(Clone, Copy)]
struct TransitScreen;

fn reload_window() {
    let _ = web_sys::window().and_then(|w| w.location().reload().ok());
}

/// Wrapper for toolbar action content (passed as `actions` prop to Layout).
pub struct ToolbarActions(pub Box<dyn FnOnce() -> AnyView>);

impl ToolbarActions {
    pub fn new(f: impl FnOnce() -> AnyView + 'static) -> Self {
        Self(Box::new(f))
    }
}

#[component]
fn Breadcrumbs(items: Vec<BreadcrumbItem>) -> impl IntoView {
    view! {
        <nav class="qui-breadcrumbs">
            <ul class="list">
                {items
                    .into_iter()
                    .map(|item| {
                        view! {
                            <li class="item">
                                {match item {
                                    BreadcrumbItem::Link(link) => {
                                        let title_attr = link.title.clone();
                                        let title_text = link.title;
                                        view! {
                                            <a
                                                class="qui-breadcrumb-link"
                                                href=link.href
                                                title=title_attr
                                            >
                                                {title_text}
                                            </a>
                                        }
                                            .into_any()
                                    }
                                    BreadcrumbItem::Current(title) => {
                                        let title_attr = title.clone();
                                        view! {
                                            <strong class="qui-breadcrumb-current" title=title_attr>
                                                {title}
                                            </strong>
                                        }
                                            .into_any()
                                    }
                                }}
                            </li>
                        }
                    })
                    .collect_view()}
            </ul>
        </nav>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use leptos_router::components::Router;
    use wasm_bindgen_test::*;

    /// Synchronous, so no other test sees the marker it sets.
    fn settings_like(v2: bool, transit: bool) -> web_sys::Element {
        theme::set_v2(v2);
        let el = mount(move || {
            view! {
                <Router>
                    <Layout
                        breadcrumbs=vec![
                            BreadcrumbItem::Link(BreadcrumbLink {
                                href: "/".into(),
                                title: "Home".into(),
                            }),
                            BreadcrumbItem::Current("Settings".into()),
                        ]
                        notification=RwSignal::new(None)
                        transit=transit
                    >
                        "body"
                    </Layout>
                </Router>
            }
        });
        theme::set_v2(false);
        el
    }

    fn draws_redesigned_bar(el: &web_sys::Element) -> bool {
        el.query_selector("header").unwrap().is_some()
    }

    fn draws_v1_bar(el: &web_sys::Element) -> bool {
        el.query_selector(".qui-appbar").unwrap().is_some()
    }

    #[wasm_bindgen_test]
    fn the_preview_swaps_the_appbar_for_the_redesigned_one() {
        let el = settings_like(true, false);
        assert!(draws_redesigned_bar(&el), "markup was {}", el.inner_html());
        assert!(
            !draws_v1_bar(&el),
            "one bar, not two; markup was {}",
            el.inner_html()
        );
    }

    #[wasm_bindgen_test]
    fn without_the_preview_the_appbar_is_v1s() {
        let el = settings_like(false, false);
        assert!(draws_v1_bar(&el), "markup was {}", el.inner_html());
        assert!(!draws_redesigned_bar(&el), "markup was {}", el.inner_html());
    }

    #[wasm_bindgen_test]
    fn the_redesigned_bar_does_not_mark_the_v1_page_as_a_v2_surface() {
        let el = settings_like(true, false);
        assert!(
            el.query_selector("[data-v2-page]").unwrap().is_none(),
            "a v1 page wearing the bar must not be repainted; markup was {}",
            el.inner_html()
        );
    }

    #[wasm_bindgen_test]
    fn the_toolbar_is_the_same_under_either_bar() {
        let toolbar = |el: &web_sys::Element| {
            el.query_selector(".layout-toolbar")
                .unwrap()
                .expect("the toolbar is drawn")
                .outer_html()
        };
        assert_eq!(
            toolbar(&settings_like(true, false)),
            toolbar(&settings_like(false, false)),
        );
    }

    #[wasm_bindgen_test]
    fn the_redesigned_bar_offers_refresh_and_settings() {
        let el = settings_like(true, false);
        let header = el.query_selector("header").unwrap().expect("the bar");
        let labels: Vec<String> = (0..header.query_selector_all("button").unwrap().length())
            .map(|i| {
                header
                    .query_selector_all("button")
                    .unwrap()
                    .item(i)
                    .unwrap()
                    .text_content()
                    .unwrap_or_default()
                    .trim()
                    .to_string()
            })
            .collect();
        assert_eq!(labels, ["Refresh", "Settings"]);
    }

    #[wasm_bindgen_test]
    fn a_layout_inside_a_transit_screen_is_transit_too() {
        theme::set_v2(true);
        let el = mount(|| {
            view! {
                <Router>
                    <Layout breadcrumbs=vec![] notification=RwSignal::new(None) transit=true>
                        <Layout breadcrumbs=vec![] notification=RwSignal::new(None)>
                            "error page"
                        </Layout>
                    </Layout>
                </Router>
            }
        });
        theme::set_v2(false);
        assert!(!draws_redesigned_bar(&el), "markup was {}", el.inner_html());
    }

    #[wasm_bindgen_test]
    fn a_transit_screen_keeps_v1s_bar_under_the_preview() {
        let el = settings_like(true, true);
        assert!(draws_v1_bar(&el), "markup was {}", el.inner_html());
        assert!(!draws_redesigned_bar(&el), "markup was {}", el.inner_html());
    }
}
