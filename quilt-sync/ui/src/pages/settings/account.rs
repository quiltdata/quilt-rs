use leptos::html::Select;
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
    let host_for_roles = host.clone();
    let back_encoded = urlencoding::encode("/settings");
    let login_href = format!(
        "/login?host={}&back={back_encoded}",
        urlencoding::encode(&host)
    );

    view! {
        <dt>{host_display}</dt>
        <dd>
            <RoleSwitcher host=host_for_roles notification=notification />
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

/// Per-host role control. Shown only when the user holds more than one role —
/// catalog parity, and a single-role user has nothing to choose. A load failure
/// renders nothing rather than an error row: a host whose stack predates the
/// role API is a normal condition, not a fault.
///
/// Unlike every other Settings section, this loads its own data instead of
/// receiving it as a prop from `get_settings_data`. That is deliberate: each
/// role fetch takes the host's credential single-flight lock — the same lock
/// credential vending uses, held across two round trips — so folding N hosts'
/// fetches into the page load would stall the whole Settings page behind a
/// control most users never touch. The row-local resource keeps the cost on
/// the row that needs it.
///
/// Switching needs no cache work here: the backend command expires the stored
/// credentials and clears the host's cached S3 clients before it returns. The
/// page's `refetch` trigger is deliberately not fired either — it reloads the
/// auth-host list, which roles have nothing to do with, and every reload would
/// take the per-host lock again.
#[component]
fn RoleSwitcher(host: String, notification: RwSignal<Option<Notification>>) -> impl IntoView {
    let host_for_load = host.clone();
    let roles = LocalResource::new(move || {
        let host = host_for_load.clone();
        async move { commands::get_roles(host).await.ok() }
    });

    // The switch is confirmed by the stack, so the control's displayed value is
    // driven back from the response rather than left wherever the click put it.
    let select_ref = NodeRef::<Select>::new();
    let show_role = move |role: &str| {
        if let Some(el) = select_ref.get_untracked() {
            el.set_value(role);
        }
    };

    view! {
        <Suspense fallback=|| ()>
            {move || {
                let host = host.clone();
                Suspend::new(async move {
                    let data = roles.await?;
                    (data.available.len() > 1)
                        .then(|| {
                            // Tracks the confirmed role so a later pick is compared
                            // against what the stack last acknowledged, not against
                            // the role this row happened to load with.
                            let initial = data.current.clone();
                            let current = RwSignal::new(data.current);
                            let options = data
                                .available
                                .into_iter()
                                .map(|role| {
                                    let selected = role == initial;
                                    let value = role.clone();
                                    view! {
                                        <option value=value selected=selected>
                                            {role}
                                        </option>
                                    }
                                })
                                .collect_view();
                            view! {
                                <select
                                    class="role-switcher"
                                    aria-label="Role"
                                    node_ref=select_ref
                                    on:change=move |ev| {
                                        let role = event_target_value(&ev);
                                        let previous = current.get_untracked();
                                        if role == previous {
                                            return;
                                        }
                                        let host = host.clone();
                                        leptos::task::spawn_local(async move {
                                            match commands::switch_role(host, role).await {
                                                Ok(d) => {
                                                    show_role(&d.current);
                                                    notification
                                                        .set(
                                                            Some(
                                                                Notification::Success(format!("Switched to {}", d.current)),
                                                            ),
                                                        );
                                                    current.set(d.current);
                                                }
                                                Err(e) => {
                                                    show_role(&previous);
                                                    notification.set(Some(Notification::Error(e)));
                                                }
                                            }
                                        });
                                    }
                                >
                                    {options}
                                </select>
                            }
                        })
                })
            }}
        </Suspense>
    }
}
