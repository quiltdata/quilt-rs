use leptos::prelude::*;

use crate::commands::{self, PublishSettingsData};
use crate::components::Notification;
use crate::components::buttons;

// ── Publish section ──

/// Placeholders supported by the Publish message preview.
///
/// Kept in lockstep with `PUBLISH_PLACEHOLDERS` in
/// `quilt-sync/src-tauri/src/commit_message.rs`. When adding or renaming a
/// placeholder, update both sides and the positional values passed to
/// [`apply_placeholders`] below.
const PUBLISH_PLACEHOLDERS: &[&str] =
    &["{date}", "{time}", "{datetime}", "{namespace}", "{changes}"];

fn apply_placeholders(template: &str, values: &[&str]) -> String {
    debug_assert_eq!(PUBLISH_PLACEHOLDERS.len(), values.len());
    let mut rendered = template.to_string();
    for (placeholder, value) in PUBLISH_PLACEHOLDERS.iter().zip(values) {
        rendered = rendered.replace(placeholder, value);
    }
    rendered
}

fn render_publish_preview(template: &str) -> String {
    if template.trim().is_empty() {
        return "Auto-generated summary of changes".to_string();
    }
    let now = js_sys::Date::new_0();
    let date = format!(
        "{:04}-{:02}-{:02}",
        now.get_full_year(),
        now.get_month() + 1,
        now.get_date()
    );
    let time = format!("{:02}:{:02}", now.get_hours(), now.get_minutes());
    let datetime = format!("{date} {time}");
    apply_placeholders(
        template,
        &[
            &date,
            &time,
            &datetime,
            "example/package",
            "3 files modified",
        ],
    )
}

#[component]
pub(super) fn PublishSection(
    publish: PublishSettingsData,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
) -> impl IntoView {
    let show_popup = RwSignal::new(false);

    let template_is_default = publish.message_template.is_empty();
    let template_display = if template_is_default {
        "Default — auto-generated summary of changes".to_string()
    } else {
        publish.message_template.clone()
    };

    let workflow_is_default = publish.default_workflow.is_empty();
    let workflow_display = if workflow_is_default {
        "Default — bucket's workflow".to_string()
    } else {
        publish.default_workflow.clone()
    };

    let metadata_is_default = publish.default_metadata.is_empty();
    let metadata_display = if metadata_is_default {
        "Default — none".to_string()
    } else {
        publish.default_metadata.clone()
    };

    let current = publish;

    view! {
        <section class="settings-section qui-publish-settings">
            <h2 class="section-title">"Commit and Push"</h2>
            <dl class="settings-list">
                <dt>"Message template"</dt>
                <dd>
                    <span
                        class="value"
                        class:default=template_is_default
                    >{template_display}</span>
                </dd>

                <dt>"Default workflow"</dt>
                <dd>
                    <span
                        class="value"
                        class:default=workflow_is_default
                    >{workflow_display}</span>
                </dd>

                <dt>"Default metadata"</dt>
                <dd>
                    <span
                        class="value"
                        class:default=metadata_is_default
                    >{metadata_display}</span>
                </dd>
            </dl>
            <div class="settings-actions">
                <button
                    type="button"
                    class="qui-button"
                    on:click=move |_| show_popup.set(true)
                >
                    <span>"Edit"</span>
                </button>
            </div>
        </section>

        <Show when=move || show_popup.get()>
            <PublishSettingsPopup
                current=current.clone()
                notification=notification
                refetch=refetch
                on_close=move || show_popup.set(false)
            />
        </Show>
    }
}

#[component]
fn PublishSettingsPopup(
    current: PublishSettingsData,
    notification: RwSignal<Option<Notification>>,
    refetch: Trigger,
    on_close: impl Fn() + Clone + 'static,
) -> impl IntoView {
    let message_template = RwSignal::new(current.message_template.clone());
    let workflow_override = RwSignal::new(current.default_workflow.clone());
    let metadata = RwSignal::new(current.default_metadata.clone());
    let use_bucket_default = RwSignal::new(current.default_workflow.is_empty());
    let metadata_error = RwSignal::new(None::<String>);
    let saving = RwSignal::new(false);

    let on_close_save = on_close.clone();
    let on_save = move |_: leptos::ev::MouseEvent| {
        if saving.get_untracked() {
            return;
        }
        let template = message_template.get_untracked();
        let wf = if use_bucket_default.get_untracked() {
            String::new()
        } else {
            workflow_override.get_untracked()
        };
        let meta = metadata.get_untracked();
        if !meta.trim().is_empty()
            && let Err(err) = serde_json::from_str::<serde_json::Value>(&meta)
        {
            metadata_error.set(Some(format!("Invalid JSON: {err}")));
            return;
        }
        metadata_error.set(None);
        saving.set(true);
        let on_close = on_close_save.clone();
        leptos::task::spawn_local(async move {
            match commands::update_publish_settings(template, wf, meta).await {
                Ok(()) => {
                    notification.set(Some(Notification::Success(
                        "Commit and Push settings saved".into(),
                    )));
                    on_close();
                    refetch.notify();
                }
                Err(e) => notification.set(Some(Notification::Error(e))),
            }
            saving.set(false);
        });
    };

    let on_reset = move |_: leptos::ev::MouseEvent| {
        message_template.set(String::new());
        workflow_override.set(String::new());
        use_bucket_default.set(true);
        metadata.set(String::new());
        metadata_error.set(None);
    };

    let on_close_cancel = on_close.clone();
    let on_cancel = move |_: leptos::ev::MouseEvent| on_close_cancel();

    view! {
        <div class="popup-overlay" on:click={
            let on_close = on_close.clone();
            move |_| on_close()
        }>
            <div class="popup-content publish-settings-form" on:click=|ev| ev.stop_propagation()>
                <h2 class="section-title">"Edit commit defaults"</h2>

                <div class="field">
                    <label for="publish-message-template">"Message template"</label>
                    <input
                        class="input"
                        id="publish-message-template"
                        placeholder="Auto-publish {date} ({changes})"
                        prop:value=move || message_template.get()
                        on:input=move |ev| message_template.set(event_target_value(&ev))
                    />
                    <p class="field-description">
                        "Placeholders: "
                        {PUBLISH_PLACEHOLDERS
                            .iter()
                            .map(|p| view! { <code>{*p}</code>" " })
                            .collect_view()}
                    </p>
                    <p class="field-description">
                        "Preview: "
                        <em>{move || render_publish_preview(&message_template.get())}</em>
                    </p>
                </div>

                <div class="field">
                    <label>"Default workflow"</label>
                    <label class="radio-option">
                        <input
                            type="radio"
                            name="publish-workflow-mode"
                            prop:checked=move || use_bucket_default.get()
                            on:change=move |_| use_bucket_default.set(true)
                        />
                        "Use the bucket's default workflow"
                    </label>
                    <label class="radio-option">
                        <input
                            type="radio"
                            name="publish-workflow-mode"
                            prop:checked=move || !use_bucket_default.get()
                            on:change=move |_| use_bucket_default.set(false)
                        />
                        "Override"
                    </label>
                    <Show when=move || !use_bucket_default.get()>
                        <input
                            class="input"
                            id="publish-workflow-override"
                            placeholder="workflow-id"
                            prop:value=move || workflow_override.get()
                            on:input=move |ev| workflow_override.set(event_target_value(&ev))
                        />
                    </Show>
                </div>

                <div class="field">
                    <label for="publish-default-metadata">"Default metadata"</label>
                    <textarea
                        class="textarea"
                        id="publish-default-metadata"
                        placeholder="{ \"source\": \"desktop\" }"
                        prop:value=move || metadata.get()
                        on:input=move |ev| metadata.set(event_target_value(&ev))
                    ></textarea>
                    <Show when=move || metadata_error.get().is_some()>
                        <span class="error">
                            {move || metadata_error.get().unwrap_or_default()}
                        </span>
                    </Show>
                </div>

                <div class="popup-actions">
                    <buttons::FormPrimary on_click=on_save disabled=saving>
                        "Save"
                    </buttons::FormPrimary>
                    <buttons::FormSecondary on_click=on_cancel />
                    <button type="button" class="qui-button link" on:click=on_reset>
                        <span>"Reset to defaults"</span>
                    </button>
                </div>
            </div>
        </div>
    }
}
