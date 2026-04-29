use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_query_map};
use wasm_bindgen::JsCast;

use crate::commands::{self, LoginData};
use crate::components::buttons;
use crate::components::{Layout, Notification, Spinner};

// ── Login page ──

#[component]
pub fn Login() -> impl IntoView {
    let notification = RwSignal::new(None);

    let query = use_query_map();
    let data = LocalResource::new(move || {
        let host = query.read().get("host").unwrap_or_default();
        let back = query.read().get("back").unwrap_or_default();
        async move { commands::get_login_data(host, back).await }
    });

    view! {
        <Layout breadcrumbs=vec![] notification=notification>
            <Suspense fallback=move || {
                view! { <Spinner /> }
            }>
                {move || Suspend::new(async move {
                    match data.await {
                        Ok(d) => {
                            view! {
                                <LoginContent data=d notification=notification />
                            }
                                .into_any()
                        }
                        Err(e) => {
                            let msg = format!("Failed to load login data: {e}");
                            view! {
                                <div class="qui-page-login container">
                                    <p>{msg}</p>
                                </div>
                            }
                                .into_any()
                        }
                    }
                })}
            </Suspense>
        </Layout>
    }
}

// ── Main content (rendered after data loads) ──

#[component]
fn LoginContent(data: LoginData, notification: RwSignal<Option<Notification>>) -> impl IntoView {
    let code = RwSignal::new(String::new());
    let submitting = RwSignal::new(false);
    let oauth_loading = RwSignal::new(false);

    let host = data.host.clone();
    let back = data.back.clone();
    let catalog_url = data.catalog_url.clone();

    let host_for_submit = host.clone();
    let back_for_submit = back.clone();

    let navigate = use_navigate();
    let on_submit = move |ev: leptos::ev::SubmitEvent| {
        ev.prevent_default();
        if submitting.get_untracked() {
            return;
        }
        submitting.set(true);
        let host = host_for_submit.clone();
        let back = back_for_submit.clone();
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            match commands::login(host, code.get_untracked()).await {
                Ok(msg) => {
                    notification.set(Some(Notification::Success(msg)));
                    let target = if back.is_empty() {
                        "/installed-packages-list".to_string()
                    } else {
                        back
                    };
                    navigate(&target, Default::default());
                }
                Err(e) => {
                    notification.set(Some(Notification::Error(e)));
                    submitting.set(false);
                }
            }
        });
    };

    let host_for_oauth = host.clone();
    let back_for_oauth = back.clone();

    let on_oauth = move |_| {
        if oauth_loading.get_untracked() {
            return;
        }
        oauth_loading.set(true);
        let host = host_for_oauth.clone();
        let back = back_for_oauth.clone();
        leptos::task::spawn_local(async move {
            let back_opt = if back.is_empty() { None } else { Some(back) };
            match commands::login_oauth(host, back_opt).await {
                Ok(msg) => notification.set(Some(Notification::Success(msg))),
                Err(e) => notification.set(Some(Notification::Error(e))),
            }
            oauth_loading.set(false);
        });
    };

    let on_open_catalog = {
        let catalog_url = catalog_url.clone();
        move |_| {
            let url = catalog_url.clone();
            leptos::task::spawn_local(async move {
                let _ = commands::open_in_web_browser(url).await;
            });
        }
    };

    let instructions = format!("Or visit {} to get your code:", catalog_url);

    view! {
        <div class="qui-page-login container">
            <div class="main">
                <p class="message">
                    <buttons::LogInWithBrowser on_click=on_oauth disabled=oauth_loading />
                </p>

                <hr class="divider" />

                <p class="message">{instructions}</p>
                <p class="message">
                    <buttons::OpenBrowser on_click=on_open_catalog />
                </p>

                <form class="form" on:submit=on_submit>
                    <p class="field">
                        <label class="label" for="code">"Code"</label>
                        <input
                            class="input"
                            id="code"
                            name="code"
                            required
                            prop:value=move || code.get()
                            on:input=move |ev| {
                                code.set(event_target_value(&ev));
                            }
                        />
                    </p>
                    <button type="submit" hidden></button>
                </form>
            </div>
        </div>

        // Action bar with Submit button
        <div class="qui-actionbar">
            <buttons::SubmitLogin
                on_click=move |_| {
                    // Programmatically submit the form
                    if let Some(doc) = web_sys::window().and_then(|w| w.document())
                        && let Some(form) = doc.query_selector("form").ok().flatten()
                        && let Ok(form_el) = form.dyn_into::<web_sys::HtmlFormElement>()
                    {
                        let _ = form_el.request_submit();
                    }
                }
                busy=submitting
                disabled=Signal::derive(move || submitting.get() || code.get().is_empty())
            />
        </div>
    }
}
