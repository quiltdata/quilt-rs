use leptos::prelude::*;

use crate::commands;
use crate::components::buttons;
use crate::components::Notification;

// ── Ignore popup ──

#[derive(Clone, Debug)]
pub struct IgnorePopupData {
    pub namespace: String,
    pub path: String,
    pub suggested_pattern: String,
}

#[derive(Clone)]
enum IgnoreHint {
    WillBeIgnored(String),
    OnlyExact(String),
    NoMatch(String),
}

#[component]
pub fn IgnorePopup(
    data: IgnorePopupData,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
    on_close: impl Fn() + Clone + 'static,
) -> impl IntoView {
    let pattern = RwSignal::new(data.suggested_pattern.clone());
    let hint = RwSignal::new(Option::<IgnoreHint>::None);
    let submitting = RwSignal::new(false);

    let path = data.path.clone();
    let suggested = data.suggested_pattern.clone();
    let namespace = data.namespace.clone();

    let path_for_hint = path.clone();
    let suggested_for_hint = suggested.clone();
    let update_hint = move || {
        let current = pattern.get_untracked();
        let path = path_for_hint.clone();
        let suggested = suggested_for_hint.clone();
        leptos::task::spawn_local(async move {
            if current.trim().is_empty() {
                hint.set(None);
                return;
            }
            let matches = commands::test_quiltignore_pattern(current.clone(), path.clone())
                .await
                .unwrap_or(false);

            let is_suggested = current == suggested;
            let is_exact = current == path;

            let value = if (is_suggested || !is_exact) && matches {
                IgnoreHint::WillBeIgnored(path)
            } else if matches && is_exact {
                IgnoreHint::OnlyExact(path)
            } else {
                IgnoreHint::NoMatch(path)
            };
            hint.set(Some(value));
        });
    };

    update_hint();

    let update_hint_clone = update_hint.clone();
    let on_input = move |ev: leptos::ev::Event| {
        pattern.set(event_target_value(&ev));
        update_hint_clone();
    };

    let ns_for_submit = namespace.clone();
    let on_close_for_submit = on_close.clone();
    let on_submit = move || {
        let p = pattern.get_untracked();
        if p.trim().is_empty() || submitting.get_untracked() {
            return;
        }
        submitting.set(true);
        let ns = ns_for_submit.clone();
        let on_close = on_close_for_submit.clone();
        leptos::task::spawn_local(async move {
            match commands::add_to_quiltignore(ns, p).await {
                Ok(msg) => {
                    notification.set(Some(Notification::Success(msg)));
                    on_close();
                    refetch.notify();
                }
                Err(e) => {
                    notification.set(Some(Notification::Error(e)));
                    submitting.set(false);
                }
            }
        });
    };

    let on_submit_click = {
        let on_submit = on_submit.clone();
        move |_| on_submit()
    };

    let on_close_cancel = on_close.clone();
    let on_cancel = move |_: leptos::ev::MouseEvent| on_close_cancel();

    let on_submit_key = on_submit.clone();
    let on_close_key = on_close.clone();
    let on_keydown = move |ev: leptos::ev::KeyboardEvent| {
        if ev.key() == "Enter" {
            on_submit_key();
        } else if ev.key() == "Escape" {
            on_close_key();
        }
    };

    view! {
        <div class="popup-overlay" on:click={
            let on_close = on_close.clone();
            move |_| on_close()
        }>
            <div class="popup-content" on:click=|ev| ev.stop_propagation()>
                <div class="ignore-popup">
                    <label>"Pattern to ignore:"</label>
                    <input
                        class="ignore-input"
                        type="text"
                        prop:value=move || pattern.get()
                        on:input=on_input
                        on:keydown=on_keydown
                    />
                    <div class="ignore-hint">
                        {move || hint.get().map(|h| match h {
                            IgnoreHint::WillBeIgnored(path) => view! {
                                <code class="inactive">{path}</code>" will be ignored"
                            }.into_any(),
                            IgnoreHint::OnlyExact(path) => view! {
                                "Only "{path}" will be ignored"
                            }.into_any(),
                            IgnoreHint::NoMatch(path) => view! {
                                "Doesn't match "<code class="inactive">{path}</code>
                            }.into_any(),
                        })}
                    </div>
                    <div class="ignore-actions">
                        <buttons::FormPrimary on_click=on_submit_click disabled=submitting>
                            "Add to .quiltignore"
                        </buttons::FormPrimary>
                        <buttons::FormSecondary on_click=on_cancel />
                    </div>
                </div>
            </div>
        </div>
    }
}

// ── Unignore popup ──

#[derive(Clone, Debug)]
pub struct UnignorePopupData {
    pub namespace: String,
    pub pattern: String,
}

#[component]
pub fn UnignorePopup(
    data: UnignorePopupData,
    notification: RwSignal<Option<Notification>>,
    on_close: impl Fn() + Clone + 'static,
) -> impl IntoView {
    let ns = data.namespace.clone();
    let pattern_display = data.pattern.clone();

    let on_close_for_edit = on_close.clone();
    let on_edit = move |_| {
        let ns = ns.clone();
        let on_close = on_close_for_edit.clone();
        leptos::task::spawn_local(async move {
            match commands::open_in_default_application(ns, ".quiltignore".to_string()).await {
                Ok(msg) => notification.set(Some(Notification::Success(msg))),
                Err(e) => notification.set(Some(Notification::Error(e))),
            }
            on_close();
        });
    };

    view! {
        <div class="popup-overlay" on:click={
            let on_close = on_close.clone();
            move |_| on_close()
        }>
            <div class="popup-content" on:click=|ev| ev.stop_propagation()>
                <div class="unignore-popup">
                    <span>"Ignored by: "<span class="pattern-display">{pattern_display}</span></span>
                    <div>
                        <buttons::FormPrimary on_click=on_edit>
                            "Edit .quiltignore"
                        </buttons::FormPrimary>
                    </div>
                </div>
            </div>
        </div>
    }
}
