//! The settings page v2's cards, as views over props.
//!
//! The gallery draws them over fixtures; the port fills the same props from
//! `get_settings_data`. Every effect is a callback. [`SettingsColumn`] fixes
//! the order, most used first: Syncing, Publishing, App, Storage,
//! Experimental, Help, About.
//!
//! Toggles, selects and minute fields save as they change, and the row says
//! *Saved* for a moment ([`Saved`]). Publishing alone has a Save button,
//! because its metadata must be valid JSON before anything is written.

use leptos::prelude::*;

use crate::commands::LogEnv;
use crate::kit::BackLink;
use crate::kit::Banner;
use crate::kit::BannerVariant;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::Card;
use crate::kit::Dialog;
use crate::kit::FormControl;
use crate::kit::LoadFailure;
use crate::kit::Naming;
use crate::kit::NumberInput;
use crate::kit::PageHeader;
use crate::kit::Select;
use crate::kit::StateTone;
use crate::kit::TextArea;
use crate::kit::TextInput;
use crate::kit::ToggleRow;
use crate::util::format_size;

use super::commit_v2::metadata_editor;

stylance::import_crate_style!(style, "src/pages/settings_v2.module.scss");

/// A row that saves as it changes, so it can say it did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Saved {
    Pull,
    Publish,
    Watch,
    Tray,
    EntirePackageSync,
    DesignPreview,
    LogLevel,
}

/// The brief *Saved* beside a row, while the page says that row just saved.
fn saved_note(saved: Signal<Option<Saved>>, row: Saved) -> AnyView {
    view! {
        <span class=style::saved role="status">
            {move || (saved.get() == Some(row)).then_some("Saved")}
        </span>
    }
    .into_any()
}

// ── Minutes ──

/// A stored interval in the field's whole minutes: rounded, and never below
/// one, so a stored 45 s shows 1. The page writes it back only when the
/// reader edits the field.
#[must_use]
pub fn minutes_shown(secs: u64) -> u64 {
    secs.saturating_add(30).checked_div(60).unwrap_or(0).max(1)
}

/// What is wrong with the minutes typed, if anything.
#[must_use]
pub fn minutes_error(text: &str) -> Option<&'static str> {
    match text.trim().parse::<i64>() {
        Ok(n) if n >= 1 => None,
        Ok(_) => Some("At least 1 minute"),
        Err(_) if text.trim().is_empty() => Some("Enter a number of minutes"),
        Err(_) => Some("Whole minutes only"),
    }
}

/// The sentence around a minutes field: `Every [1] minute`, `After [5]
/// minutes of inactivity`. Its error, if any, goes under it.
#[component]
fn MinutesSentence(
    /// The words before the field.
    before: &'static str,
    /// The words after `minute(s)`, if any.
    #[prop(optional)]
    after: &'static str,
    /// The field's name for a reader: the sentence is not a label.
    name: &'static str,
    minutes: RwSignal<String>,
    on_commit: Option<Callback<()>>,
) -> impl IntoView {
    let error = Signal::derive(move || minutes.with(|text| minutes_error(text)));
    let error_id = crate::kit::unique_id("minutes-error");
    let unit = move || {
        if minutes.with(|text| text.trim() == "1") {
            "minute"
        } else {
            "minutes"
        }
    };
    view! {
        <span class=style::sentence>
            {before}
            <NumberInput
                naming=Naming::Hidden(name.to_string())
                value=minutes
                min=1
                invalid=Signal::derive(move || error.get().is_some())
                described_by={
                    let id = error_id.clone();
                    Signal::derive(move || error.get().map(|_| id.clone()))
                }
                on_commit=on_commit.unwrap_or_else(|| Callback::new(|()| ()))
            />
            {unit}
            {after}
        </span>
        <Show when=move || error.get().is_some()>
            <p class=style::error id=error_id.clone()>
                {StateTone::Danger.glyph()}
                <span>{move || error.get().unwrap_or_default()}</span>
            </p>
        </Show>
    }
}

// ── Header and column ──

/// *‹ Packages*, then *Settings*. No actions: everything here is in a card.
#[component]
pub fn SettingsHeader(
    /// `/` by default.
    #[prop(optional, into)]
    home_href: Option<String>,
) -> impl IntoView {
    view! {
        <PageHeader
            trail=view! {
                <BackLink href=home_href.unwrap_or_else(|| "/".to_string()) label="Packages" />
            }
                .into_any()
            title="Settings"
            actions=().into_any()
        />
    }
}

/// The page's one column of cards, most used first.
#[component]
pub fn SettingsColumn(
    header: AnyView,
    syncing: AnyView,
    publishing: AnyView,
    app: AnyView,
    storage: AnyView,
    experimental: AnyView,
    help: AnyView,
    about: AnyView,
) -> impl IntoView {
    view! {
        <div class=style::column>
            {header}
            {syncing}
            {publishing}
            {app}
            {storage}
            {experimental}
            {help}
            {about}
        </div>
    }
}

// ── Syncing ──

pub const WATCH_SUBLABEL: &str = "Package statuses update as soon as you save, add or delete a \
     file in a package folder. When this is off, changes appear only after you press Refresh. \
     Turn it off if your system can't watch this many files.";

/// Autosync's two directions with their minutes, and the folder watcher.
///
/// A minute field writes only its own interval: the stored unfocused and
/// closed intervals stay as they are.
#[component]
pub fn SyncingCard(
    pull: RwSignal<bool>,
    pull_minutes: RwSignal<String>,
    publish: RwSignal<bool>,
    publish_minutes: RwSignal<String>,
    watch: RwSignal<bool>,
    #[prop(optional, into)] saved: Signal<Option<Saved>>,
    /// Enter or leaving the pull minutes, when they are valid.
    #[prop(optional)]
    on_pull_minutes: Option<Callback<()>>,
    #[prop(optional)] on_publish_minutes: Option<Callback<()>>,
) -> impl IntoView {
    view! {
        <Card title="Syncing">
            <ToggleRow
                label="Get new revisions"
                checked=pull
                trailing=saved_note(saved, Saved::Pull)
                detail=view! {
                    <MinutesSentence
                        before="Every "
                        name="Minutes between checks for new revisions"
                        minutes=pull_minutes
                        on_commit=on_pull_minutes
                    />
                }
                    .into_any()
            />
            <ToggleRow
                label="Publish your changes"
                checked=publish
                trailing=saved_note(saved, Saved::Publish)
                detail=view! {
                    <MinutesSentence
                        before="After "
                        after=" of inactivity"
                        name="Minutes of inactivity before publishing"
                        minutes=publish_minutes
                        on_commit=on_publish_minutes
                    />
                }
                    .into_any()
            />
            <ToggleRow
                label="Watch folders for changes"
                sublabel=WATCH_SUBLABEL
                checked=watch
                trailing=saved_note(saved, Saved::Watch)
            />
        </Card>
    }
}

// ── Publishing ──

/// Placeholders the message template fills in. In step with
/// `PUBLISH_PLACEHOLDERS` in `quilt-sync/src-tauri/src/commit_message.rs`.
pub const PUBLISH_PLACEHOLDERS: &[&str] =
    &["{date}", "{time}", "{datetime}", "{namespace}", "{changes}"];

pub const PUBLISHING_USE: &str = "Publishing and autosync use these. They also fill in a new \
     revision's form, where you can change them.";

/// Shown while a default could fail in some bucket.
pub const GLOBAL_SCOPE_WARNING: &str = "These settings apply to every bucket. Workflows and \
     metadata schemas are defined per bucket, so a default here can make publishing fail in a \
     bucket that doesn't define it.";

/// Whether to warn that the defaults apply to every bucket: while a workflow
/// is named, or there is metadata. The current values, not whether they
/// changed: a stored default warns too.
#[must_use]
pub fn show_global_scope_warning(workflow_named: bool, metadata: &str) -> bool {
    workflow_named || !metadata.trim().is_empty()
}

/// Why the default metadata cannot be saved, if it cannot. Empty is fine.
#[must_use]
pub fn metadata_error(text: &str) -> Option<String> {
    if text.trim().is_empty() {
        return None;
    }
    serde_json::from_str::<serde_json::Value>(text)
        .err()
        .map(|err| format!("Not valid JSON: {err}"))
}

/// The publish defaults, as fields in place. The one card with Save.
#[component]
pub fn PublishingCard(
    template: RwSignal<String>,
    /// A workflow id; empty uses each bucket's default.
    workflow: RwSignal<String>,
    metadata: RwSignal<String>,
    /// The fields differ from what is stored.
    #[prop(into)]
    dirty: Signal<bool>,
    #[prop(optional, into)] saving: Signal<bool>,
    on_save: Callback<()>,
) -> impl IntoView {
    let error = Signal::derive(move || metadata.with(|text| metadata_error(text)));
    let warn = Signal::derive(move || {
        let named = workflow.with(|id| !id.trim().is_empty());
        metadata.with(|text| show_global_scope_warning(named, text))
    });
    let blocked = Signal::derive(move || !dirty.get() || error.with(Option::is_some));
    let metadata_id = crate::kit::unique_id("default-metadata");
    let error_id = format!("{metadata_id}-error");
    let placeholders = format!("Placeholders: {}", PUBLISH_PLACEHOLDERS.join(" "));

    view! {
        <Card title="Publishing">
            <div class=style::fields>
                <p class=style::quiet>{PUBLISHING_USE}</p>
                <FormControl
                    label="Message template"
                    caption=placeholders
                    control=move |id| {
                        view! {
                            <TextInput
                                id=id
                                value=template
                                placeholder="Generated from the changes"
                                disabled=saving
                            />
                        }
                            .into_any()
                    }
                />
                <FormControl
                    label="Default workflow"
                    caption="A workflow id. Empty uses each bucket's default."
                    control=move |id| {
                        view! {
                            <TextInput
                                id=id
                                value=workflow
                                placeholder="Each bucket's default"
                                disabled=saving
                            />
                        }
                            .into_any()
                    }
                />
                <div>
                    <label class=style::field_name for=metadata_id.clone()>
                        "Default metadata"
                    </label>
                    {metadata_editor(
                        metadata_id.clone(),
                        error_id.clone(),
                        metadata,
                        Signal::derive(move || error.with(Option::is_some)),
                    )}
                    <Show when=move || error.with(Option::is_some)>
                        <p class=style::error id=error_id.clone()>
                            {StateTone::Danger.glyph()}
                            <span>{move || error.get().unwrap_or_default()}</span>
                        </p>
                    </Show>
                </div>
                <Show when=move || warn.get()>
                    <Banner variant=BannerVariant::Warning>{GLOBAL_SCOPE_WARNING}</Banner>
                </Show>
                <div class=style::actions>
                    <Button
                        variant=ButtonVariant::Primary
                        disabled=blocked
                        loading=saving
                        on_click=move |_| on_save.run(())
                    >
                        "Save"
                    </Button>
                </div>
            </div>
        </Card>
    }
}

// ── App ──

pub const TRAY_SUBLABEL: &str = "Closing the window leaves QuiltSync running in the menu bar \
     (system tray), so autosync keeps working. Leave off if your desktop has no tray, such as \
     stock GNOME.";

/// The window and the tray. A card of its own: more tray settings will join it.
#[component]
pub fn AppCard(
    tray: RwSignal<bool>,
    #[prop(optional, into)] saved: Signal<Option<Saved>>,
) -> impl IntoView {
    view! {
        <Card title="App">
            <ToggleRow
                label="Keep running in the tray when the window is closed"
                sublabel=TRAY_SUBLABEL
                checked=tray
                trailing=saved_note(saved, Saved::Tray)
            />
        </Card>
    }
}

// ── Storage ──

/// What the app's own storage measured. Read after the page paints.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageSize {
    Measuring,
    /// `freeable` is an estimate; *Freed …* after the button is the answer.
    Measured {
        total: u64,
        freeable: u64,
    },
    Failed,
}

impl StorageSize {
    /// `3.2 GB · 1.1 GB can be freed`, or `… · nothing to free`.
    #[must_use]
    pub fn words(&self) -> Option<String> {
        match self {
            Self::Measuring => Some("Measuring…".to_string()),
            Self::Measured { total, freeable: 0 } => {
                Some(format!("{} · nothing to free", format_size(*total)))
            }
            Self::Measured { total, freeable } => Some(format!(
                "{} · {} can be freed",
                format_size(*total),
                format_size(*freeable)
            )),
            Self::Failed => None,
        }
    }
}

/// A name, a value, and what to do with it: one line of a card.
#[component]
fn ValueRow(
    #[prop(into)] name: String,
    value: AnyView,
    #[prop(optional)] action: Option<AnyView>,
    /// A line under the value.
    #[prop(optional, into)]
    note: Option<String>,
) -> impl IntoView {
    view! {
        <div class=style::row>
            <div class=style::row_text>
                <span class=style::row_name>{name}</span>
                <span class=style::row_value>{value}</span>
                {note.map(|note| view! { <span class=style::row_note>{note}</span> })}
            </div>
            {action.map(|action| view! { <div class=style::row_action>{action}</div> })}
        </div>
    }
}

fn path_value(path: String) -> AnyView {
    let title = path.clone();
    view! { <span class=style::path title=title>{path}</span> }.into_any()
}

/// Where packages live, and the app's own storage.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads them from there"
)]
pub fn StorageCard(
    /// `None` until the reader picks one.
    home_dir: Option<String>,
    on_open_home: Callback<()>,
    size: StorageSize,
    #[prop(optional, into)] freeing: Signal<bool>,
    /// The sweep's own answer, such as `Freed 1.1 GB`.
    #[prop(optional, into)]
    freed: Option<String>,
    on_free: Callback<()>,
    on_measure_again: Callback<()>,
) -> impl IntoView {
    let nothing = matches!(size, StorageSize::Measured { freeable: 0, .. });
    let home = match home_dir {
        Some(path) => path_value(path),
        None => view! { <span class=style::quiet>"Not set"</span> }.into_any(),
    };
    let measured = match size.words() {
        Some(words) => view! { <span>{words}</span> }.into_any(),
        None => view! {
            <LoadFailure words="Could not measure it." on_retry=on_measure_again />
        }
        .into_any(),
    };
    view! {
        <Card title="Storage">
            <ValueRow
                name="Where packages live"
                value=home
                action=view! { <Button on_click=move |_| on_open_home.run(())>"Open"</Button> }
                    .into_any()
            />
            <ValueRow
                name="App storage"
                value=view! {
                    {measured}
                    {freed.map(|words| view! { <span class=style::freed role="status">{words}</span> })}
                }
                    .into_any()
                action=view! {
                    <Button
                        disabled=nothing
                        loading=freeing
                        on_click=move |_| on_free.run(())
                    >
                        "Free up space"
                    </Button>
                }
                    .into_any()
            />
        </Card>
    }
}

// ── Experimental ──

pub const ENTIRE_PACKAGE_SYNC_SUBLABEL: &str =
    "Adds a per-package option to sync everything in a package, instead of picking files.";
pub const DESIGN_PREVIEW_SUBLABEL: &str = "The redesigned QuiltSync, wherever it is ready.";

/// Opt-ins for what is still being designed.
#[component]
pub fn ExperimentalCard(
    entire_package_sync: RwSignal<bool>,
    design_preview: RwSignal<bool>,
    #[prop(optional, into)] saved: Signal<Option<Saved>>,
) -> impl IntoView {
    view! {
        <Card title="Experimental">
            <ToggleRow
                label="Enable entire-package sync"
                sublabel=ENTIRE_PACKAGE_SYNC_SUBLABEL
                checked=entire_package_sync
                trailing=saved_note(saved, Saved::EntirePackageSync)
            />
            <ToggleRow
                label="New design preview"
                sublabel=DESIGN_PREVIEW_SUBLABEL
                checked=design_preview
                trailing=saved_note(saved, Saved::DesignPreview)
            />
        </Card>
    }
}

// ── Help ──

pub const LOG_LEVELS: [&str; 6] = ["Default", "Trace", "Debug", "Info", "Warn", "Error"];

/// The dropdown's label for a saved level (`debug` → `Debug`).
#[must_use]
pub fn log_level_label(mut level: String) -> String {
    if let Some(first) = level.get_mut(0..1) {
        first.make_ascii_uppercase();
    }
    if LOG_LEVELS.contains(&level.as_str()) {
        level
    } else {
        LOG_LEVELS[0].to_string()
    }
}

/// The line under the dropdown when `QUILT_LOG` is set.
#[must_use]
pub fn log_env_hint(env: &LogEnv) -> Option<String> {
    match env {
        LogEnv::Unset => None,
        LogEnv::Overrides(value) => Some(format!(
            "Set by the QUILT_LOG environment variable ({value})"
        )),
        LogEnv::Ignored(value) => Some(format!(
            "QUILT_LOG={value:?} is not a log level or a list of directives, so it's ignored"
        )),
    }
}

pub const TEMPORARY_LOGS: &str = "The usual logs folder could not be created, so logs go to a \
     temporary folder that is deleted when QuiltSync quits.";

/// Reporting a problem, and the logs and folders behind one.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads them from there"
)]
pub fn HelpCard(
    on_report: Callback<()>,
    /// One of [`LOG_LEVELS`].
    log_level: RwSignal<String>,
    log_env: LogEnv,
    #[prop(into)] logs_dir: String,
    #[prop(optional)] logs_dir_is_temporary: bool,
    on_open_logs: Callback<()>,
    #[prop(into)] data_dir: String,
    on_open_data: Callback<()>,
    #[prop(optional, into)] saved: Signal<Option<Saved>>,
) -> impl IntoView {
    // A valid `QUILT_LOG` replaces the saved level, so the choice is moot.
    let overridden = matches!(log_env, LogEnv::Overrides(_));
    let caption = match log_env_hint(&log_env) {
        Some(hint) => format!("Applies after a restart. {hint}."),
        None => "Applies after a restart.".to_string(),
    };
    view! {
        <Card title="Help">
            <ValueRow
                name="Report a problem"
                value=view! {
                    <span class=style::quiet>
                        "Sends your logs to Quilt, with what went wrong if you say."
                    </span>
                }
                    .into_any()
                action=view! { <Button on_click=move |_| on_report.run(())>"Report a problem…"</Button> }
                    .into_any()
            />
            <div class=style::row>
                <div class=style::row_text>
                    <FormControl
                        label="Log level"
                        caption=caption
                        control=move |id| {
                            view! {
                                <span class=style::select>
                                    <Select
                                        naming=Naming::FormControl(id)
                                        options=LOG_LEVELS.iter().map(ToString::to_string).collect()
                                        selected=log_level
                                        disabled=overridden
                                    />
                                </span>
                            }
                                .into_any()
                        }
                    />
                </div>
                <div class=style::row_action>{saved_note(saved, Saved::LogLevel)}</div>
            </div>
            {
                let logs_note = logs_dir_is_temporary.then_some(TEMPORARY_LOGS);
                match logs_note {
                    Some(note) => view! {
                        <ValueRow
                            name="Logs folder"
                            value=path_value(logs_dir)
                            note=note
                            action=view! {
                                <Button on_click=move |_| on_open_logs.run(())>"Open"</Button>
                            }
                                .into_any()
                        />
                    }
                    .into_any(),
                    None => view! {
                        <ValueRow
                            name="Logs folder"
                            value=path_value(logs_dir)
                            action=view! {
                                <Button on_click=move |_| on_open_logs.run(())>"Open"</Button>
                            }
                                .into_any()
                        />
                    }
                    .into_any(),
                }
            }
            <ValueRow
                name="Data folder"
                value=path_value(data_dir)
                action=view! { <Button on_click=move |_| on_open_data.run(())>"Open"</Button> }
                    .into_any()
            />
        </Card>
    }
}

// ── About ──

#[component]
pub fn AboutCard(#[prop(into)] version: String, on_release_notes: Callback<()>) -> impl IntoView {
    view! {
        <Card title="About">
            <ValueRow
                name="Version"
                value=view! { <span>{version}</span> }.into_any()
                action=view! {
                    <Button on_click=move |_| on_release_notes.run(())>"Release notes"</Button>
                }
                    .into_any()
            />
        </Card>
    }
}

// ── Report a problem ──

/// Where a problem report is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ReportState {
    Writing,
    /// The dialog collects the logs, then sends them.
    Sending,
    Sent,
    /// The send's own error.
    Failed(String),
}

/// What a report carries, in the order a reader would worry about it.
pub const REPORT_CONTENTS: &[&str] = &[
    "What you wrote above, if anything",
    "The app's version and your operating system",
    "Log files",
    "Folder paths",
    "Names of the hosts you are signed in to",
    "OAuth client IDs",
    "This installation's anonymous ID",
];

pub const EMAIL_INSTEAD: &str =
    "Email instead opens your mail app with a message that names the zip of your logs to attach.";

pub const REPORT_SENT: &str = "Sent to Quilt. Thank you.";

/// The dialog's body: what went wrong, and what is sent.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads them from there"
)]
pub fn ReportBody(message: RwSignal<String>, state: ReportState) -> impl IntoView {
    // Sealed while it goes, and after: what was sent is what it says.
    let busy = matches!(state, ReportState::Sending | ReportState::Sent);
    let outcome = match state {
        ReportState::Sent => {
            Some(view! { <Banner variant=BannerVariant::Success>{REPORT_SENT}</Banner> }.into_any())
        }
        ReportState::Failed(err) => {
            Some(view! { <Banner variant=BannerVariant::Critical>{err}</Banner> }.into_any())
        }
        ReportState::Writing | ReportState::Sending => None,
    };
    view! {
        {outcome}
        <FormControl
            label="What went wrong?"
            caption="Optional."
            control=move |id| {
                view! { <TextArea id=id value=message disabled=busy /> }.into_any()
            }
        />
        <div>
            <p class=style::field_name>"What is sent"</p>
            <ul class=style::sent_list>
                {REPORT_CONTENTS.iter().map(|item| view! { <li>{*item}</li> }).collect_view()}
            </ul>
        </div>
        <p class=style::quiet>{EMAIL_INSTEAD}</p>
    }
}

/// The dialog's buttons, primary last. Once sent, only Close.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads them from there"
)]
pub fn ReportFooter(
    state: ReportState,
    on_email: Callback<()>,
    on_send: Callback<()>,
    on_close: Callback<()>,
) -> impl IntoView {
    let busy = state == ReportState::Sending;
    if state == ReportState::Sent {
        return view! {
            <Button variant=ButtonVariant::Primary on_click=move |_| on_close.run(())>
                "Close"
            </Button>
        }
        .into_any();
    }
    view! {
        <Button disabled=busy on_click=move |_| on_close.run(())>"Cancel"</Button>
        <Button disabled=busy on_click=move |_| on_email.run(())>"Email instead"</Button>
        <Button variant=ButtonVariant::Primary loading=busy on_click=move |_| on_send.run(())>
            "Send to Quilt"
        </Button>
    }
    .into_any()
}

/// *Report a problem…*, as the modal the button opens.
#[component]
pub fn ReportDialog(
    open: RwSignal<bool>,
    message: RwSignal<String>,
    #[prop(into)] state: Signal<ReportState>,
    on_email: Callback<()>,
    on_send: Callback<()>,
) -> impl IntoView {
    let on_close = Callback::new(move |()| open.set(false));
    view! {
        <Dialog
            open=open
            title="Report a problem"
            held=Signal::derive(move || state.get() == ReportState::Sending)
            footer=view! {
                {move || view! {
                    <ReportFooter
                        state=state.get()
                        on_email=on_email
                        on_send=on_send
                        on_close=on_close
                    />
                }}
            }
                .into_any()
        >
            {move || view! { <ReportBody message=message state=state.get() /> }}
        </Dialog>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pages::commit_v2::tests::banned_in;

    #[test]
    fn a_stored_interval_shows_in_whole_minutes() {
        assert_eq!(minutes_shown(45), 1);
        assert_eq!(minutes_shown(30), 1);
        assert_eq!(minutes_shown(0), 1);
        assert_eq!(minutes_shown(60), 1);
        assert_eq!(minutes_shown(89), 1);
        assert_eq!(minutes_shown(90), 2);
        assert_eq!(minutes_shown(300), 5);
        assert_eq!(minutes_shown(u64::MAX), u64::MAX / 60);
    }

    #[test]
    fn minutes_below_one_say_so() {
        assert_eq!(minutes_error("1"), None);
        assert_eq!(minutes_error(" 15 "), None);
        assert_eq!(minutes_error("0"), Some("At least 1 minute"));
        assert_eq!(minutes_error("-3"), Some("At least 1 minute"));
        assert_eq!(minutes_error(""), Some("Enter a number of minutes"));
        assert_eq!(minutes_error("1.5"), Some("Whole minutes only"));
    }

    #[test]
    fn storage_reads_as_its_two_numbers() {
        assert_eq!(
            StorageSize::Measured {
                total: 3_200_000_000,
                freeable: 1_100_000_000
            }
            .words()
            .as_deref(),
            Some("3.2\u{a0}GB · 1.1\u{a0}GB can be freed")
        );
        assert_eq!(
            StorageSize::Measured {
                total: 1_500_000_000,
                freeable: 0
            }
            .words()
            .as_deref(),
            Some("1.5\u{a0}GB · nothing to free")
        );
        assert_eq!(StorageSize::Failed.words(), None);
    }

    #[test]
    fn empty_metadata_is_valid_and_broken_metadata_says_why() {
        assert_eq!(metadata_error("  "), None);
        assert_eq!(metadata_error("{\"a\": 1}"), None);
        assert!(metadata_error("{\"a\": ").is_some_and(|err| err.starts_with("Not valid JSON: ")),);
    }

    #[test]
    fn the_every_bucket_warning_follows_the_values() {
        assert!(show_global_scope_warning(true, ""));
        assert!(show_global_scope_warning(
            false,
            "{\"source\": \"desktop\"}"
        ));
        assert!(!show_global_scope_warning(false, "  \n\t "));
        assert!(!show_global_scope_warning(false, ""));
    }

    #[test]
    fn a_saved_level_maps_to_its_label() {
        assert_eq!(log_level_label("debug".into()), "Debug");
        assert_eq!(log_level_label("default".into()), "Default");
        assert_eq!(log_level_label("nonsense".into()), "Default");
    }

    #[test]
    fn the_hint_names_the_variable_and_its_value() {
        assert_eq!(log_env_hint(&LogEnv::Unset), None);
        assert_eq!(
            log_env_hint(&LogEnv::Overrides("quilt_rs=trace".into())).as_deref(),
            Some("Set by the QUILT_LOG environment variable (quilt_rs=trace)")
        );
        assert_eq!(
            log_env_hint(&LogEnv::Ignored("debgu".into())).as_deref(),
            Some("QUILT_LOG=\"debgu\" is not a log level or a list of directives, so it's ignored")
        );
    }

    /// v2 vocabulary: no commit, push, pull or remote in the page's words.
    #[test]
    fn no_fixed_string_uses_a_banned_word() {
        let mut all = vec![
            WATCH_SUBLABEL,
            PUBLISHING_USE,
            GLOBAL_SCOPE_WARNING,
            TRAY_SUBLABEL,
            ENTIRE_PACKAGE_SYNC_SUBLABEL,
            DESIGN_PREVIEW_SUBLABEL,
            TEMPORARY_LOGS,
            EMAIL_INSTEAD,
            REPORT_SENT,
        ];
        all.extend(REPORT_CONTENTS);
        for words in all {
            assert_eq!(banned_in(words), None, "{words:?}");
        }
    }
}
