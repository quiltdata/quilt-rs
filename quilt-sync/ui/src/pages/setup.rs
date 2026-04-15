use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

use crate::commands;
use crate::components::buttons;
use crate::components::{Layout, Spinner};

// ── Setup page ──

#[component]
pub fn Setup() -> impl IntoView {
    let notification = RwSignal::new(None);

    let data = LocalResource::new(move || async { commands::get_setup_data().await });

    view! {
        <Layout breadcrumbs=vec![] notification=notification>
            <Suspense fallback=move || {
                view! { <Spinner /> }
            }>
                {move || Suspend::new(async move {
                    match data.await {
                        Ok(d) => {
                            view! {
                                <SetupContent default_home=d.default_home />
                            }
                                .into_any()
                        }
                        Err(e) => {
                            let msg = format!("Failed to load setup data: {e}");
                            view! {
                                <div class="qui-page-setup container">
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
fn SetupContent(default_home: String) -> impl IntoView {
    let directory = RwSignal::new(default_home);
    let hint = RwSignal::new(String::new());
    let saving = RwSignal::new(false);
    let browsing = RwSignal::new(false);

    let on_browse = move |_| {
        if browsing.get_untracked() {
            return;
        }
        browsing.set(true);
        hint.set(String::new());
        leptos::task::spawn_local(async move {
            match commands::open_directory_picker().await {
                Ok(path) => {
                    directory.set(path);
                    hint.set(String::new());
                }
                Err(e) => hint.set(e),
            }
            browsing.set(false);
        });
    };

    let navigate = use_navigate();
    let on_save = move |_| {
        if saving.get_untracked() {
            return;
        }
        saving.set(true);
        hint.set(String::new());
        let navigate = navigate.clone();
        leptos::task::spawn_local(async move {
            match commands::setup(directory.get_untracked()).await {
                Ok(_) => {
                    navigate("/installed-packages-list", Default::default());
                }
                Err(e) => {
                    hint.set(e);
                    saving.set(false);
                }
            }
        });
    };

    view! {
        <div class="qui-page-setup container">
            <div class="main">
                <p class="message">
                    "Select a directory where QuiltSync will store your packages, ex. ~/QuiltSync"
                </p>

                <form class="form" on:submit=|ev| ev.prevent_default()>
                    <p class="field">
                        <label class="label" for="directory">
                            "Set home directory"
                        </label>
                        <input
                            class="input"
                            id="directory"
                            name="directory"
                            required
                            readonly
                            prop:value=move || directory.get()
                        />
                        <span class="hint">{move || hint.get()}</span>
                    </p>

                    <buttons::Browse on_click=on_browse disabled=browsing />
                </form>
            </div>
        </div>

        // Action bar with Save button
        <div class="qui-actionbar">
            <buttons::Save on_click=on_save busy=saving />
        </div>
    }
}
