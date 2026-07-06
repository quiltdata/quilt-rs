use leptos::prelude::*;

use crate::commands;
use crate::components::Notification;
use crate::components::buttons;

// ── Account section ──

#[component]
pub(super) fn AccountSection(
    auth_hosts: Vec<String>,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
) -> impl IntoView {
    view! {
        <section class="settings-section">
            <h2 class="section-title">"Auth"</h2>
            {if auth_hosts.is_empty() {
                view! { <p class="empty-state">"No authenticated hosts"</p> }.into_any()
            } else {
                view! {
                    <dl class="settings-list">
                        {auth_hosts
                            .into_iter()
                            .map(|host| {
                                view! { <AuthHostRow host=host notification=notification refetch=refetch /> }
                            })
                            .collect_view()}
                    </dl>
                }
                    .into_any()
            }}
        </section>
    }
}

#[component]
fn AuthHostRow(
    host: String,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
) -> impl IntoView {
    let host_display = host.clone();
    let host_for_logout = host.clone();
    let back_encoded = urlencoding::encode("/settings");
    let login_href = format!(
        "/login?host={}&back={back_encoded}",
        urlencoding::encode(&host)
    );

    view! {
        <dt>{host_display}</dt>
        <dd>
            <buttons::ReLogin href=login_href />
            <div class="qui-popover">
                <buttons::Logout
                    on_click=move |_| {
                        let host = host_for_logout.clone();
                        leptos::task::spawn_local(async move {
                            match commands::erase_auth(host).await {
                                Ok(msg) => notification.set(Some(Notification::Success(msg))),
                                Err(e) => {
                                    notification
                                        .set(Some(Notification::Error(e)));
                                }
                            }
                            refetch.notify();
                        });
                    }
                    small=true
                />
                <div class="popover-wrapper">
                    <div class="popover">
                        "This will erase stored credentials for this host. You will need to log in again."
                    </div>
                </div>
            </div>
        </dd>
    }
}
