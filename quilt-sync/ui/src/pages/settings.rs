mod account;
mod autosync;
mod diagnostics;
mod fswatcher;
mod general;
mod publish;

use leptos::prelude::*;

use account::AccountSection;
use autosync::AutosyncSection;
use diagnostics::DiagnosticsSection;
use fswatcher::FsWatcherSection;
use general::GeneralSection;
use publish::PublishSection;

use crate::commands::{self, SettingsData};
use crate::components::layout::{BreadcrumbItem, BreadcrumbLink};
use crate::components::{Layout, Notification, Spinner};

// ── Settings page ──

#[component]
pub fn Settings() -> impl IntoView {
    let notification = RwSignal::new(None);
    let refetch = Trigger::new();

    let data = LocalResource::new(move || {
        refetch.track();
        async { commands::get_settings_data().await }
    });

    let breadcrumbs = vec![
        BreadcrumbItem::Link(BreadcrumbLink {
            href: "/installed-packages-list".to_string(),
            title: String::new(),
        }),
        BreadcrumbItem::Current("Settings".to_string()),
    ];

    // Layout wraps Suspense here (not the other way around) because
    // breadcrumbs are static and can render immediately while data loads.
    // Pages with data-dependent breadcrumbs use Suspense outside Layout.
    view! {
        <Layout breadcrumbs=breadcrumbs notification=notification>
            <Suspense fallback=move || {
                view! { <Spinner /> }
            }>
                {move || Suspend::new(async move {
                    match data.await {
                        Ok(d) => {
                            view! { <SettingsContent data=d notification=notification refetch=refetch /> }.into_any()
                        }
                        Err(e) => {
                            crate::error_handler::handle_or_display(&e, notification)
                        }
                    }
                })}
            </Suspense>
        </Layout>
    }
}

// ── Main content (rendered after data loads) ──

#[component]
fn SettingsContent(
    data: SettingsData,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
) -> impl IntoView {
    let zip_path = RwSignal::new(None::<String>);

    view! {
        <div class="qui-page-settings container">
            <GeneralSection
                version=data.version.clone()
                home_dir=data.home_dir
                data_dir=data.data_dir
                changelog=data.changelog
                notification=notification
            />
            <PublishSection publish=data.publish notification=notification refetch=refetch />
            <AutosyncSection autosync=data.autosync notification=notification refetch=refetch />
            <FsWatcherSection fswatcher=data.fswatcher notification=notification refetch=refetch />
            <AccountSection auth_hosts=data.auth_hosts notification=notification refetch=refetch />
            <DiagnosticsSection
                version=data.version
                os=data.os
                log_level=data.log_level
                logs_dir=data.logs_dir
                logs_dir_is_temporary=data.logs_dir_is_temporary
                notification=notification
                zip_path=zip_path
            />
        </div>
    }
}

// ── Shared helpers ──

fn event_target_checked(ev: &leptos::ev::Event) -> bool {
    use wasm_bindgen::JsCast;
    ev.target()
        .and_then(|t| t.dyn_into::<web_sys::HtmlInputElement>().ok())
        .is_some_and(|el| el.checked())
}
