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
/// # Which app bar
///
/// The redesigned [`Appbar`] when the reader is in the redesign
/// ([`theme::is_v2`]), v1's own otherwise. **Only the app bar.** The toolbar
/// under it is this page's breadcrumbs and actions, not a shared unit with a
/// replacement, so it stays v1's either way — a preview reader sees the new bar
/// over v1's breadcrumbs, and that is the intended picture.
///
/// The redesigned bar takes v1's slot as it is: v1's height, because the
/// toolbar's sticky offset and the `ui_locked` overlay both start below it, and
/// v1's container, so the logo lines up with the breadcrumbs.
///
/// Its Refresh is the whole-window reload v1's has always been. Only a v2 page
/// has a refetch to hand it, and taking the redesigned bar does not make a v1
/// page adopt one.
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
    /// A transit screen — one whose mount *is* its operation, like the
    /// deep-link relay — keeps v1's bar whatever the preview says.
    ///
    /// The redesign replaces the bar a route draws; it does not hand a working
    /// Refresh to a route where Refresh is a hazard. On such a route the reload
    /// re-enters the screen and runs its operation again, and on
    /// `/remote-package` that re-opens a path-naming URI in the OS default
    /// application. The v1 bar it already draws has the same reload, which is a
    /// known defect and not this flag's to fix: all this does is keep the
    /// preview from carrying it forward.
    ///
    /// Inherited: a `Layout` drawn inside a transit one is transit too. The
    /// relay's failure path draws the shared error page, which brings a
    /// `Layout` of its own, and that one must not hand out the bar either.
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

/// Marks everything under a transit [`Layout`], so a nested one inherits it.
#[derive(Clone, Copy)]
struct TransitScreen;

/// v1's Refresh, in either bar: reload the whole window, which re-mounts the
/// route and so re-reads whatever it shows.
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

    /// A v1 page with a trail, as Settings draws it, under whichever generation
    /// the root says. Synchronous from marker to assertion and back, so no other
    /// test can run between them and see the marker this one set.
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

    /// The reason the bar was extracted from the frame: the frame's marker is
    /// what turns a surface dark, and the v1 page under the bar must stay on
    /// v1's light canvas.
    #[wasm_bindgen_test]
    fn the_redesigned_bar_does_not_mark_the_v1_page_as_a_v2_surface() {
        let el = settings_like(true, false);
        assert!(
            el.query_selector("[data-v2-page]").unwrap().is_none(),
            "a v1 page wearing the bar must not be repainted; markup was {}",
            el.inner_html()
        );
    }

    /// Only the appbar is replaced. The toolbar is the page's own trail and
    /// actions, and stays v1's under either bar.
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

    /// The redesigned bar carries the same pair v1's does.
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

    /// A page nested in a transit one — the shared error page is — is transit
    /// too, or the relay's failure path hands out the bar the relay withholds.
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

    /// The deep-link relay. Its mount is its operation, so the preview must not
    /// hand it a bar whose Refresh reloads the window and runs the link again.
    #[wasm_bindgen_test]
    fn a_transit_screen_keeps_v1s_bar_under_the_preview() {
        let el = settings_like(true, true);
        assert!(draws_v1_bar(&el), "markup was {}", el.inner_html());
        assert!(!draws_redesigned_bar(&el), "markup was {}", el.inner_html());
    }
}
