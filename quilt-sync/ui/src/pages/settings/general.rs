use leptos::prelude::*;

use crate::commands::{self, ChangelogEntry};
use crate::components::Notification;
use crate::components::buttons;

// ── General section ──

#[component]
pub(super) fn GeneralSection(
    version: String,
    home_dir: Option<String>,
    data_dir: String,
    changelog: Vec<ChangelogEntry>,
    notification: RwSignal<Option<Notification>>,
) -> impl IntoView {
    let home_display = home_dir.unwrap_or_else(|| "Not set".to_string());
    let home_title = home_display.clone();
    let data_title = data_dir.clone();
    let show_release_notes = RwSignal::new(false);

    let on_open_home = move |_| {
        leptos::task::spawn_local(async move {
            match commands::open_home_dir().await {
                Ok(msg) => notification.set(Some(Notification::Success(msg))),
                Err(e) => notification.set(Some(Notification::Error(e))),
            }
        });
    };

    let on_open_data = move |_| {
        leptos::task::spawn_local(async move {
            match commands::open_data_dir().await {
                Ok(msg) => notification.set(Some(Notification::Success(msg))),
                Err(e) => notification.set(Some(Notification::Error(e))),
            }
        });
    };

    view! {
        <section class="settings-section">
            <h2 class="section-title">"General"</h2>
            <dl class="settings-list">
                <dt>"Version"</dt>
                <dd>
                    <span>{version}</span>
                    <buttons::ReleaseNotes on_click=move |_| show_release_notes.set(true) />
                </dd>

                <dt>"Home directory"</dt>
                <dd>
                    <span class="path" title=home_title>{home_display}</span>
                    <buttons::OpenInFileBrowser on_click=on_open_home small=true link=true />
                </dd>

                <dt>"Data directory"</dt>
                <dd>
                    <span class="path" title=data_title>{data_dir}</span>
                    <buttons::OpenInFileBrowser on_click=on_open_data small=true link=true />
                </dd>
            </dl>
        </section>

        <Show when=move || show_release_notes.get()>
            <ReleaseNotesPopup
                changelog=changelog.clone()
                on_close=move || show_release_notes.set(false)
            />
        </Show>
    }
}

// ── Release notes popup ──

#[component]
fn ReleaseNotesPopup(
    changelog: Vec<ChangelogEntry>,
    on_close: impl Fn() + 'static,
) -> impl IntoView {
    view! {
        <div class="popup-overlay" on:click=move |_| on_close()>
            <div
                class="popup-content release-notes"
                on:click=|ev| ev.stop_propagation()
            >
                <h2 class="section-title">"Release Notes"</h2>
                {changelog
                    .into_iter()
                    .map(|entry| {
                        view! {
                            <div class="release-notes-entry">
                                <h3>{entry.version}</h3>
                                <p>{entry.date}</p>
                                <pre>{entry.body}</pre>
                            </div>
                        }
                    })
                    .collect_view()}
            </div>
        </div>
    }
}
