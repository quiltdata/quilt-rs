use leptos::prelude::*;

/// Notification variant for the layout notification bar.
#[derive(Clone)]
pub enum Notification {
    Success(String),
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

/// Top-level page layout matching the existing Askama layout.html structure.
///
/// Provides: app bar, toolbar with breadcrumbs and optional actions,
/// notification area, content slot (children), and popup overlay.
#[component]
pub fn Layout(
    breadcrumbs: Vec<BreadcrumbItem>,
    notification: RwSignal<Option<Notification>>,
    /// Optional toolbar actions rendered to the right of breadcrumbs.
    #[prop(optional)]
    actions: Option<ToolbarActions>,
    children: Children,
) -> impl IntoView {
    view! {
        <div class="qui-layout" id="layout">
            // ── App bar ──
            <div class="qui-appbar layout-appbar">
                <div class="container">
                    <a class="qui-logo" href="/">
                        <img class="img" src="/assets/img/quilt.png" />
                    </a>
                    <div class="nav">
                        <button
                            class="qui-button link"
                            type="button"
                            on:click=move |_| {
                                let _ = web_sys::window()
                                    .and_then(|w| w.location().reload().ok());
                            }
                        >
                            <img class="qui-icon" src="/assets/img/icons/refresh.svg" />
                            <span>"Refresh"</span>
                        </button>
                        <a href="/settings">
                            <button class="qui-button link" type="button">
                                <img class="qui-icon" src="/assets/img/icons/gear.svg" />
                                <span>"Settings"</span>
                            </button>
                        </a>
                    </div>
                </div>
            </div>

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
