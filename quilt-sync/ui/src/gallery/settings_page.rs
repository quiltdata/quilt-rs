//! The settings page v2, one cell per state.
//!
//! Every card is `pages::settings_v2`'s own, over fixture props, so the port
//! and this scene cannot drift. Callbacks are dropped. Cards are drawn at the
//! page column's width; the last cell is the whole page at the app's 1024×560
//! floor. Its appbar has no actions: Refresh and Settings belong to the
//! pages that list packages.
//!
//! # Measured at 1024x560, in Chromium
//!
//! The header ends at 138px and the cards stack 16px apart: Syncing 154–461,
//! Publishing 477–968, App 984–1116, Storage 1132–1300, Experimental
//! 1316–1487, Help 1503–1826, About 1842–1953. The first screen shows the
//! header and Syncing; the page scrolls under the appbar, with no sideways
//! scroll.
//!
//! # Left for the port
//!
//! - The metadata editor repairs JSON it can (a missing `}`) and says so in
//!   its own green bar, above the page's *Not valid JSON* error. The two
//!   disagree until the reader picks the editor's repair.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::kit::Button;
use crate::kit::PageLayout;
use crate::pages::settings_v2::{
    AboutCard, AppCard, ExperimentalCard, HelpCard, PublishingCard, ReportBody, ReportDialog,
    ReportFooter, ReportState, Saved, SettingsColumn, SettingsHeader, StorageCard, StorageSize,
    SyncingCard, minutes_shown,
};
use quilt_sync_ui::commands::LogEnv;

const HOME_DIR: &str = "/Users/a-user/QuiltSync";
const DATA_DIR: &str = "/Users/a-user/Library/Application Support/com.quiltdata.quilt-sync";
const LOGS_DIR: &str = "/Users/a-user/Library/Logs/com.quiltdata.quilt-sync";
const TEMP_LOGS_DIR: &str = "/var/folders/7k/T/.tmpX2b9Qe";
const VERSION: &str = "0.22.8";

fn noop() -> Callback<()> {
    Callback::new(|()| ())
}

/// A card at the page column's width.
fn column(card: AnyView) -> AnyView {
    view! { <div class="g-stack" style="max-width:992px">{card}</div> }.into_any()
}

// ── Syncing ──

struct Syncing {
    pull: bool,
    pull_minutes: String,
    publish: bool,
    publish_minutes: String,
    saved: Option<Saved>,
}

impl Default for Syncing {
    /// The defaults: pull on every minute, publish off.
    fn default() -> Self {
        Self {
            pull: true,
            pull_minutes: "1".to_string(),
            publish: false,
            publish_minutes: "5".to_string(),
            saved: None,
        }
    }
}

fn syncing(s: Syncing) -> AnyView {
    view! {
        <SyncingCard
            pull=RwSignal::new(s.pull)
            pull_minutes=RwSignal::new(s.pull_minutes)
            publish=RwSignal::new(s.publish)
            publish_minutes=RwSignal::new(s.publish_minutes)
            watch=RwSignal::new(true)
            saved=Signal::stored(s.saved)
        />
    }
    .into_any()
}

// ── Publishing ──

#[derive(Clone, Copy, Default)]
struct Publishing {
    template: &'static str,
    workflow: &'static str,
    metadata: &'static str,
    dirty: bool,
}

fn publishing(p: Publishing) -> AnyView {
    view! {
        <PublishingCard
            template=RwSignal::new(p.template.to_string())
            workflow=RwSignal::new(p.workflow.to_string())
            metadata=RwSignal::new(p.metadata.to_string())
            dirty=p.dirty
            on_save=noop()
        />
    }
    .into_any()
}

// ── Storage ──

fn storage(size: StorageSize, freeing: bool, freed: Option<&'static str>) -> AnyView {
    match freed {
        Some(words) => view! {
            <StorageCard
                home_dir=Some(HOME_DIR.to_string())
                on_open_home=noop()
                size=size
                freeing=freeing
                freed=words
                on_free=noop()
                on_measure_again=noop()
            />
        }
        .into_any(),
        None => view! {
            <StorageCard
                home_dir=Some(HOME_DIR.to_string())
                on_open_home=noop()
                size=size
                freeing=freeing
                on_free=noop()
                on_measure_again=noop()
            />
        }
        .into_any(),
    }
}

const MEASURED: StorageSize = StorageSize::Measured {
    total: 3_200_000_000,
    freeable: 1_100_000_000,
};

// ── Help ──

fn help(log_env: LogEnv, level: &str, temporary: bool) -> AnyView {
    let logs_dir = if temporary { TEMP_LOGS_DIR } else { LOGS_DIR };
    view! {
        <HelpCard
            on_report=noop()
            log_level=RwSignal::new(level.to_string())
            log_env=log_env
            logs_dir=logs_dir
            logs_dir_is_temporary=temporary
            on_open_logs=noop()
            data_dir=DATA_DIR
            on_open_data=noop()
        />
    }
    .into_any()
}

// ── Report a problem ──

/// The dialog drawn inline at the modal's width, as `forms.rs` draws its
/// forms; *Open the real one* opens the modal.
fn report(message: &'static str, state: ReportState) -> AnyView {
    let message = RwSignal::new(message.to_string());
    view! {
        <div class="g-bars g-dialog-inline">
            <ReportBody message=message state=state.clone() />
            <div class="g-inline g-inline--end">
                <ReportFooter state=state on_email=noop() on_send=noop() on_close=noop() />
            </div>
        </div>
    }
    .into_any()
}

// ── The whole page ──

fn whole_page() -> AnyView {
    let id = "settings-page";
    view! {
        <div id=id class="g-window" style="width:1024px; --q-frame-height:560px; max-width:100%">
            <PageLayout heading="QuiltSync" banner=().into_any()>
                <SettingsColumn
                    header=view! { <SettingsHeader home_href=format!("#{id}") /> }.into_any()
                    syncing=syncing(Syncing::default())
                    publishing=publishing(Publishing::default())
                    app=view! { <AppCard tray=RwSignal::new(false) /> }.into_any()
                    storage=storage(MEASURED, false, None)
                    experimental=view! {
                        <ExperimentalCard
                            entire_package_sync=RwSignal::new(false)
                            design_preview=RwSignal::new(true)
                        />
                    }
                        .into_any()
                    help=help(LogEnv::Unset, "Default", false)
                    about=view! { <AboutCard version=VERSION on_release_notes=noop() /> }.into_any()
                />
            </PageLayout>
        </div>
    }
    .into_any()
}

const NOTE: &str = "The settings page v2: one column of cards, most used first — Syncing, \
    Publishing, App, Storage, Experimental, Help, About. Toggles, selects and minute fields save \
    as they change and say Saved; Publishing alone has Save. Not routed yet.";

#[component]
#[allow(clippy::too_many_lines, reason = "one cell per state, read as a list")]
pub fn SettingsPageScene() -> impl IntoView {
    let report_open = RwSignal::new(false);

    view! {
        <Scene title="The settings page v2" note=NOTE>
            <Cell full=true label="Syncing — the defaults: Get new revisions on, every minute; Publish off">
                {column(syncing(Syncing::default()))}
            </Cell>
            <Cell full=true label="Syncing — both on, custom minutes">
                {column(syncing(Syncing {
                    publish: true,
                    pull_minutes: "15".to_string(),
                    publish_minutes: "10".to_string(),
                    ..Syncing::default()
                }))}
            </Cell>
            <Cell full=true label="Syncing — a minute field below 1, with its error">
                {column(syncing(Syncing { pull_minutes: "0".to_string(), ..Syncing::default() }))}
            </Cell>
            <Cell full=true label="Syncing — a stored 45 s shows 1, and is written only if edited">
                {column(syncing(Syncing {
                    pull_minutes: minutes_shown(45).to_string(),
                    ..Syncing::default()
                }))}
            </Cell>
            <Cell full=true label="Syncing — just saved: the row says so for a moment">
                {column(syncing(Syncing { saved: Some(Saved::Pull), ..Syncing::default() }))}
            </Cell>
            <Cell full=true label="Publishing — at rest, nothing set, Save disabled">
                {column(publishing(Publishing::default()))}
            </Cell>
            <Cell full=true label="Publishing — edited, Save enabled">
                {column(publishing(Publishing {
                    template: "Updated {namespace} on {date} ({changes})",
                    dirty: true,
                    ..Publishing::default()
                }))}
            </Cell>
            <Cell full=true label="Publishing — metadata that is not valid JSON: the error, Save disabled">
                {column(publishing(Publishing {
                    metadata: "{\"source\": \"desktop\",",
                    dirty: true,
                    ..Publishing::default()
                }))}
            </Cell>
            <Cell full=true label="Publishing — a workflow and metadata set: the every-bucket warning">
                {column(publishing(Publishing {
                    workflow: "plate-reads",
                    metadata: "{\"source\": \"desktop\"}",
                    ..Publishing::default()
                }))}
            </Cell>
            <Cell full=true label="App">
                {column(view! { <AppCard tray=RwSignal::new(true) /> }.into_any())}
            </Cell>
            <Cell full=true label="Storage — measuring, after the page paints">
                {column(storage(StorageSize::Measuring, false, None))}
            </Cell>
            <Cell full=true label="Storage — measured, some can be freed">
                {column(storage(MEASURED, false, None))}
            </Cell>
            <Cell full=true label="Storage — nothing to free, so the button is disabled">
                {column(storage(
                    StorageSize::Measured { total: 1_500_000_000, freeable: 0 },
                    false,
                    None,
                ))}
            </Cell>
            <Cell full=true label="Storage — freeing">
                {column(storage(MEASURED, true, None))}
            </Cell>
            <Cell full=true label="Storage — freed: the sweep's own answer beside the estimate">
                {column(storage(
                    StorageSize::Measured { total: 2_100_000_000, freeable: 0 },
                    false,
                    Some("Freed 1.1\u{a0}GB"),
                ))}
            </Cell>
            <Cell full=true label="Storage — the measure failed">
                {column(storage(StorageSize::Failed, false, None))}
            </Cell>
            <Cell full=true label="Experimental">
                {column(view! {
                    <ExperimentalCard
                        entire_package_sync=RwSignal::new(false)
                        design_preview=RwSignal::new(true)
                    />
                }
                    .into_any())}
            </Cell>
            <Cell full=true label="Help — at rest">
                {column(help(LogEnv::Unset, "Default", false))}
            </Cell>
            <Cell full=true label="Help — the log level set by QUILT_LOG: disabled, with the hint">
                {column(help(LogEnv::Overrides("quilt_rs=trace".to_string()), "Info", false))}
            </Cell>
            <Cell full=true label="Help — the logs in a temporary folder">
                {column(help(LogEnv::Unset, "Default", true))}
            </Cell>
            <Cell full=true label="About">
                {column(view! { <AboutCard version=VERSION on_release_notes=noop() /> }.into_any())}
            </Cell>
            <Cell wide=true label="Report a problem — empty">
                {report("", ReportState::Writing)}
            </Cell>
            <Cell wide=true label="Report a problem — with text">
                {report(
                    "Autosync stopped publishing my plate reads after I renamed the folder.",
                    ReportState::Writing,
                )}
            </Cell>
            <Cell wide=true label="Report a problem — sending: collecting the logs, then sending">
                {report(
                    "Autosync stopped publishing my plate reads after I renamed the folder.",
                    ReportState::Sending,
                )}
            </Cell>
            <Cell wide=true label="Report a problem — sent">
                {report(
                    "Autosync stopped publishing my plate reads after I renamed the folder.",
                    ReportState::Sent,
                )}
            </Cell>
            <Cell wide=true label="Report a problem — the real modal">
                <div class="g-inline">
                    <Button on_click=move |_| report_open.set(true)>
                        "Open the real one"
                    </Button>
                </div>
                <ReportDialog
                    open=report_open
                    message=RwSignal::new(String::new())
                    state=ReportState::Writing
                    on_email=noop()
                    on_send=noop()
                />
            </Cell>
            <Cell full=true label="The whole page at 1024×560, the app's smallest window">
                {whole_page()}
            </Cell>
        </Scene>
    }
}
