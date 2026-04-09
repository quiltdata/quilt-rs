use leptos::prelude::*;

/// Breadcrumb link item (navigates to a page).
#[derive(Clone)]
pub struct BreadcrumbLink {
    pub href: &'static str,
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
/// Provides: app bar, toolbar with breadcrumbs, notification area,
/// content slot (children), and popup overlay.
#[component]
pub fn Layout(
    breadcrumbs: Vec<BreadcrumbItem>,
    notification: RwSignal<String>,
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
                        <a href="settings.html">
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
                </div>
                <div class="qui-notify">
                    <div
                        id="notify"
                        class="root"
                        inner_html=move || notification.get()
                        on:click=move |_| {
                            if let Some(el) = web_sys::window()
                                .and_then(|w| w.document())
                                .and_then(|d| d.get_element_by_id("notify"))
                            {
                                el.set_inner_html("");
                            }
                        }
                    ></div>
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
