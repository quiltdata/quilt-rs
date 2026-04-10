use leptos::prelude::*;
use leptos_router::hooks::{use_navigate, use_query_map};
use wasm_bindgen::JsCast;

use crate::commands::{self, LoginData};
use crate::components::{Layout, Spinner};

// ── Login page ──

#[component]
pub fn Login() -> impl IntoView {
    let notification = RwSignal::new(String::new());

    let query = use_query_map();
    let data = LocalResource::new(move || {
        let host = query.read().get("host").unwrap_or_default();
        let back = query.read().get("back").unwrap_or_default();
        async move {
            commands::get_login_data(host, back).await
        }
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
fn LoginContent(data: LoginData, notification: RwSignal<String>) -> impl IntoView {
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
            let back_opt = if back.is_empty() { None } else { Some(back.clone()) };
            match commands::login(host, code.get_untracked(), back_opt.clone()).await {
                Ok(html) => {
                    notification.set(html);
                    // Rust's login command calls navigate_after_login when back is present;
                    // only navigate from JS when there is no back to avoid double navigation.
                    if back_opt.is_none() {
                        navigate("/installed-packages-list", Default::default());
                    }
                }
                Err(e) => {
                    notification.set(format!("<div class=\"error\">{e}</div>"));
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
                Ok(html) => notification.set(html),
                Err(e) => notification.set(format!("<div class=\"error\">{e}</div>")),
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
                    <button
                        class="qui-button primary large"
                        type="button"
                        prop:disabled=move || oauth_loading.get()
                        on:click=on_oauth
                    >
                        <img class="qui-icon" src="/assets/img/icons/open_in_browser.svg" />
                        <span>"Log in with browser"</span>
                    </button>
                </p>

                <hr class="divider" />

                <p class="message">{instructions}</p>
                <p class="message">
                    <button
                        class="qui-button"
                        type="button"
                        on:click=on_open_catalog
                    >
                        <img class="qui-icon" src="/assets/img/icons/open_in_browser.svg" />
                        <span>"Open browser"</span>
                    </button>
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
            <button
                class="qui-button primary large"
                type="button"
                prop:disabled=move || submitting.get() || code.get().is_empty()
                on:click=move |_| {
                    // Programmatically submit the form
                    if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                        if let Some(form) = doc.query_selector("form").ok().flatten() {
                            if let Ok(form_el) = form.dyn_into::<web_sys::HtmlFormElement>() {
                                let _ = form_el.request_submit();
                            }
                        }
                    }
                }
            >
                <img class="qui-icon" src="/assets/img/icons/done.svg" />
                <span>
                    {move || if submitting.get() { "Logging in\u{2026}" } else { "Submit code and Log in" }}
                </span>
            </button>
        </div>
    }
}
