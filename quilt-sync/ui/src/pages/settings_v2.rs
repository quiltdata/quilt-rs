//! The settings page v2's cards, as views over props.
//!
//! The gallery draws them over fixtures; the port fills the same props from
//! `get_settings_data`. Every effect is a callback. [`SettingsLayout`] fixes
//! where each card goes.
//!
//! Toggles, selects and minute fields save as they change, and the row says
//! *Saved* for a moment ([`Saved`]). Publishing and Agent tasks are edited in
//! a popup with Save, because their fields must be valid before anything is
//! written.

use std::future::Future;

use leptos::prelude::*;

use crate::commands::AgentSettings;
use crate::commands::LogEnv;
use crate::commands::PublishSettingsData;
use crate::commands::TaskSettings;
use crate::commands::TaskSettingsData;
use crate::kit::BackLink;
use crate::kit::Banner;
use crate::kit::BannerVariant;
use crate::kit::Blankslate;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::Card;
use crate::kit::Checkbox;
use crate::kit::ConfirmDialog;
use crate::kit::Dialog;
use crate::kit::FormControl;
use crate::kit::FormDialog;
use crate::kit::HostRow;
use crate::kit::JsonDisplay;
use crate::kit::LoadFailure;
use crate::kit::Naming;
use crate::kit::NumberInput;
use crate::kit::PageHeader;
use crate::kit::Select;
use crate::kit::StateTone;
use crate::kit::Submit;
use crate::kit::TextArea;
use crate::kit::TextInput;
use crate::kit::ToggleRow;
use crate::kit::TwoColumns;
use crate::util::format_size;

use super::commit_v2::{MetadataEditorField, MetadataPreview};

stylance::import_crate_style!(style, "src/pages/settings_v2.module.scss");

mod page;
pub use page::{SettingsV2, SettingsV2Skeleton};

/// A row that saves as it changes, so it can say it did.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Saved {
    Pull,
    Publish,
    Watch,
    Tray,
    DesignPreview,
    AgentTasks,
    LogLevel,
}

/// Whether `row`'s own save is out: its control is held still until it
/// answers, so one setting never has two saves in flight.
fn saving(pending: Signal<Vec<Saved>>, row: Saved) -> Signal<bool> {
    Signal::derive(move || pending.with(|rows| rows.contains(&row)))
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
/// reader edits the field. The pull interval already arrives in whole
/// minutes, because the backend keeps it so; the quiet window may not.
#[must_use]
pub fn minutes_shown(secs: u64) -> u64 {
    secs.saturating_add(30).checked_div(60).unwrap_or(0).max(1)
}

/// The seconds the minutes typed stand for, or what is wrong with them. The
/// one reading of the field: its error and its save cannot disagree.
pub fn minutes_secs(text: &str) -> Result<u64, &'static str> {
    const TOO_MANY: &str = "Too many minutes";
    let text = text.trim();
    if text.is_empty() {
        return Err("Enter a number of minutes");
    }
    if !text
        .trim_start_matches(['-', '+'])
        .bytes()
        .all(|b| b.is_ascii_digit())
    {
        return Err("Whole minutes only");
    }
    match text.parse::<i64>() {
        Ok(n) if n < 1 => Err("At least 1 minute"),
        Ok(n) => u64::try_from(n)
            .ok()
            .and_then(|n| n.checked_mul(60))
            .ok_or(TOO_MANY),
        // All digits, so only too long to be a number.
        Err(_) if text.starts_with('-') => Err("At least 1 minute"),
        Err(_) => Err(TOO_MANY),
    }
}

/// What is wrong with the minutes typed, if anything.
#[must_use]
pub fn minutes_error(text: &str) -> Option<&'static str> {
    minutes_secs(text).err()
}

/// A minutes field for a toggle row's trailing slot, and its error for the
/// row's detail, under the words. `name` is the field's name for a reader, as
/// the sub-label says it. While `held` (its save is out) it takes no typing,
/// but the cursor stays. Only a valid value is saved; an invalid one stays in
/// the field with its error.
fn minutes_field(
    name: &'static str,
    minutes: RwSignal<String>,
    on_commit: Option<Callback<()>>,
    held: Signal<bool>,
) -> (AnyView, AnyView) {
    let error = Signal::derive(move || minutes.with(|text| minutes_error(text)));
    let error_id = crate::kit::unique_id("minutes-error");
    let field = view! {
        <NumberInput
            naming=Naming::Hidden(name.to_string())
            value=minutes
            min=1
            readonly=held
            invalid=Signal::derive(move || error.get().is_some())
            described_by={
                let id = error_id.clone();
                Signal::derive(move || error.get().map(|_| id.clone()))
            }
            on_commit=Callback::new(move |()| {
                if let Some(commit) = on_commit
                    && error.with_untracked(Option::is_none)
                {
                    commit.run(());
                }
            })
        />
    }
    .into_any();
    let message = view! {
        <Show when=move || error.get().is_some()>
            <p class=style::error id=error_id.clone()>
                {StateTone::Danger.glyph()}
                <span>{move || error.get().unwrap_or_default()}</span>
            </p>
        </Show>
    }
    .into_any();
    (field, message)
}

/// A row's control at its trailing end, with the row's *Saved* before it, so
/// the control stays at the card's edge when the note comes and goes.
fn trailing_control(saved: AnyView, control: AnyView) -> AnyView {
    view! { <span class=style::control>{saved}{control}</span> }.into_any()
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

/// The page's cards, in rows of two on a wide window: Syncing and Accounts,
/// the two the main page's strip shows, then Storage and App, Publishing and
/// Troubleshooting, Experimental and About, then Ignore rules, with Agent tasks
/// beside it when it is on. Narrow, the same order in one column.
#[component]
pub fn SettingsLayout(
    header: AnyView,
    syncing: AnyView,
    publishing: AnyView,
    ignore_rules: AnyView,
    accounts: AnyView,
    app: AnyView,
    storage: AnyView,
    experimental: AnyView,
    troubleshooting: AnyView,
    about: AnyView,
    /// Empty unless its experiment is on.
    #[prop(optional)]
    agent_tasks: Option<AnyView>,
) -> impl IntoView {
    view! {
        <div class=style::page>
            {header}
            // Two columns from about a 1024px window.
            <TwoColumns min_column=480>
                {syncing}
                {accounts}
                {storage}
                {app}
                {publishing}
                {troubleshooting}
                {experimental}
                {about}
                // Last, so its long list stretches no other card's row.
                {ignore_rules}
                {agent_tasks}
            </TwoColumns>
        </div>
    }
}

// ── Syncing ──

/// The pull minutes' sub-label, and the field's name for a reader.
pub const PULL_MINUTES: &str = "Minutes between checks for new revisions";
/// The publish minutes' sub-label, and the field's name for a reader.
pub const PUBLISH_MINUTES: &str = "Minutes to wait after your last edit before publishing";

pub const WATCH_SUBLABEL: &str = "Package statuses update as soon as you save, add or delete a \
     file in a package folder. When this is off, changes appear at the next autosync check, or \
     when you press Refresh. Turn it off if your system can't watch this many files.";

/// Autosync's two directions with their minutes, and the folder watcher.
///
/// A minute field writes only its own interval. The pull minutes are the one
/// check interval, whether the window is open, in the background or closed to
/// the tray.
#[component]
pub fn SyncingCard(
    pull: RwSignal<bool>,
    pull_minutes: RwSignal<String>,
    publish: RwSignal<bool>,
    publish_minutes: RwSignal<String>,
    watch: RwSignal<bool>,
    #[prop(optional, into)] saved: Signal<Option<Saved>>,
    /// The rows whose save is out.
    #[prop(optional, into)]
    pending: Signal<Vec<Saved>>,
    /// Enter or leaving the pull minutes, when they are valid.
    #[prop(optional)]
    on_pull_minutes: Option<Callback<()>>,
    #[prop(optional)] on_publish_minutes: Option<Callback<()>>,
) -> impl IntoView {
    let (pull_field, pull_error) = minutes_field(
        PULL_MINUTES,
        pull_minutes,
        on_pull_minutes,
        saving(pending, Saved::Pull),
    );
    let (publish_field, publish_error) = minutes_field(
        PUBLISH_MINUTES,
        publish_minutes,
        on_publish_minutes,
        saving(pending, Saved::Publish),
    );
    view! {
        <Card title="Syncing">
            <ToggleRow
                label="Get new revisions"
                sublabel=format!("{PULL_MINUTES}.")
                checked=pull
                disabled=saving(pending, Saved::Pull)
                trailing=trailing_control(saved_note(saved, Saved::Pull), pull_field)
                detail=pull_error
            />
            <ToggleRow
                label="Publish your changes"
                sublabel=format!("{PUBLISH_MINUTES}.")
                checked=publish
                disabled=saving(pending, Saved::Publish)
                trailing=trailing_control(saved_note(saved, Saved::Publish), publish_field)
                detail=publish_error
            />
            <ToggleRow
                label="Watch folders for changes"
                sublabel=WATCH_SUBLABEL
                checked=watch
                disabled=saving(pending, Saved::Watch)
                trailing=saved_note(saved, Saved::Watch)
            />
        </Card>
    }
}

// ── Publishing ──

/// Placeholders the message template fills in. In step with
/// `PUBLISH_PLACEHOLDERS` in `quilt-sync/src-tauri/src/engine/commit_message.rs`.
pub const PUBLISH_PLACEHOLDERS: &[&str] =
    &["{date}", "{time}", "{datetime}", "{namespace}", "{changes}"];

/// The template an empty one works as: `render_publish_message` in
/// `quilt-sync/src-tauri/src/engine/commit_message.rs` returns the changes summary.
pub const EMPTY_TEMPLATE_MEANS: &str = "{changes}";

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

/// The publish defaults as stored, read-only, and **Edit…**, which opens
/// [`PublishingDialog`]. Each empty value says what happens instead, muted.
#[component]
pub fn PublishingCard(
    #[prop(into)] stored: Signal<PublishSettingsData>,
    on_edit: Callback<()>,
) -> impl IntoView {
    let muted = |words: &'static str| view! { <span class=style::quiet>{words}</span> }.into_any();
    let template = move || {
        stored.with(|p| match p.message_template.as_str() {
            "" => view! {
                <span class=style::quiet>
                    "Generated from the changes: "
                    <code class=style::code>{EMPTY_TEMPLATE_MEANS}</code>
                </span>
            }
            .into_any(),
            text => view! { <span class=style::long>{text.to_string()}</span> }.into_any(),
        })
    };
    let workflow = move || {
        stored.with(|p| match p.default_workflow.as_str() {
            "" => muted("Each bucket's default"),
            id => view! { <span class=style::long>{id.to_string()}</span> }.into_any(),
        })
    };
    let metadata = move || {
        stored.with(|p| match MetadataPreview::of(&p.default_metadata) {
            MetadataPreview::None => muted("No metadata"),
            MetadataPreview::Json(value) => {
                view! { <div class=style::json><JsonDisplay value=value /></div> }.into_any()
            }
            // Saved only once valid, so only a file edited by hand.
            MetadataPreview::Invalid => muted("Not valid JSON"),
        })
    };
    view! {
        <Card title="Publishing">
            <p class=style::lead>{PUBLISHING_USE}</p>
            <ValueRow name="Message template" value=view! { {template} }.into_any() />
            <ValueRow name="Default workflow" value=view! { {workflow} }.into_any() />
            <ValueRow name="Default metadata" value=view! { {metadata} }.into_any() />
            <div class=style::card_actions>
                <Button on_click=move |_| on_edit.run(())>"Edit…"</Button>
            </div>
        </Card>
    }
}

/// The metadata editor's opening height in the popup: room for about a dozen
/// lines of JSON.
pub const PUBLISHING_EDITOR_HEIGHT: i32 = 260;

/// *Reset to defaults*: fills the fields with the defaults (no template, no
/// workflow, no metadata) and saves nothing, so Cancel undoes it.
pub fn reset_publishing(
    template: RwSignal<String>,
    workflow: RwSignal<String>,
    metadata: RwSignal<String>,
    fresh: RwSignal<u64>,
) {
    template.set(String::new());
    workflow.set(String::new());
    metadata.set(String::new());
    fresh.update(|n| *n += 1);
}

/// The popup's fields: the template, the workflow, the metadata editor and
/// the every-bucket warning. Apart from the dialog so the gallery can draw
/// them inline.
#[component]
pub fn PublishingForm(
    template: RwSignal<String>,
    /// A workflow id; empty uses each bucket's default.
    workflow: RwSignal<String>,
    metadata: RwSignal<String>,
    /// Changes when the metadata is set from outside its editor, so the
    /// editor is drawn again over the new value.
    fresh: RwSignal<u64>,
    /// While a save is out. The dialog's fieldset seals the other fields;
    /// the editor is no form control, so it is made `inert`.
    #[prop(optional, into)]
    sealed: Signal<bool>,
) -> impl IntoView {
    let error = Signal::derive(move || metadata.with(|text| metadata_error(text)));
    let warn = Signal::derive(move || {
        let named = workflow.with(|id| !id.trim().is_empty());
        metadata.with(|text| show_global_scope_warning(named, text))
    });
    let placeholders = format!("Placeholders: {}", PUBLISH_PLACEHOLDERS.join(" "));
    view! {
        <FormControl
            label="Message template"
            caption=placeholders
            control=move |id| {
                view! {
                    <TextInput id=id value=template placeholder="Generated from the changes" />
                }
                    .into_any()
            }
        />
        <FormControl
            label="Default workflow"
            caption="A workflow id. Empty uses each bucket's default."
            control=move |id| {
                view! { <TextInput id=id value=workflow placeholder="Each bucket's default" /> }
                    .into_any()
            }
        />
        <div inert=move || sealed.get()>
            <MetadataEditorField
                label="Default metadata"
                metadata=metadata
                error=error
                fresh=fresh
                height=PUBLISHING_EDITOR_HEIGHT
            />
        </div>
        <Show when=move || warn.get()>
            <Banner variant=BannerVariant::Warning>{GLOBAL_SCOPE_WARNING}</Banner>
        </Show>
    }
}

/// *Publishing*, as the modal **Edit…** opens: [`PublishingForm`], wide for
/// its editor, with *Reset to defaults* at the footer's start and Cancel and
/// Save at its end. Save is off while the metadata is not valid JSON. The fields
/// are the caller's draft; Cancel leaves them, and the caller fills them
/// again on the next open.
#[component]
pub fn PublishingDialog<F, Fut>(
    open: RwSignal<bool>,
    template: RwSignal<String>,
    workflow: RwSignal<String>,
    metadata: RwSignal<String>,
    fresh: RwSignal<u64>,
    /// Writes the draft. Its `Err` stays in the dialog.
    save: F,
    /// The page's lock, which Save holds too: the dialog seals while it is
    /// held.
    #[prop(optional, into)]
    running: Signal<bool>,
) -> impl IntoView
where
    F: Fn() -> Fut + 'static,
    Fut: Future<Output = Result<(), String>> + 'static,
{
    let blocked = Signal::derive(move || metadata.with(|text| metadata_error(text).is_some()));
    view! {
        <FormDialog
            open=open
            title="Publishing"
            submit=Submit::new("Save", save)
            running=running
            blocked=blocked
            wide=true
            footer_start=view! {
                <Button
                    disabled=running
                    on_click=move |_| reset_publishing(template, workflow, metadata, fresh)
                >
                    "Reset to defaults"
                </Button>
            }
                .into_any()
        >
            <PublishingForm
                template=template
                workflow=workflow
                metadata=metadata
                fresh=fresh
                sealed=running
            />
        </FormDialog>
    }
}

// ── Agent tasks ──

pub const AGENT_TASKS_USE: &str = "A new revision in one of these buckets can ask an agent on \
     this computer to run a task. Changes apply at the next check.";

pub const NO_BUCKETS: &str = "No buckets: tasks are off";

pub const TRUST_ALL: &str = "Approve every request the agent makes";

pub const TRUST_ALL_WARNING: &str = "Off, a request is approved only when every file it names \
     is in the package's folder; one that names no file, or a file outside the folder, is \
     refused, and the agent carries on without it. On, every request is approved, commands and \
     files outside the folder included.";

/// One agent in the popup's draft.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AgentDraft {
    pub name: RwSignal<String>,
    /// One argument per line, the program first.
    pub command: RwSignal<String>,
    pub trust_all: RwSignal<bool>,
    /// The agent as stored. Its command is saved as it is while its text is
    /// not edited, so an argument the box cannot show, such as an empty one,
    /// survives; its `env`, which the popup does not show, always is.
    pub stored: StoredValue<AgentSettings>,
}

impl AgentDraft {
    #[must_use]
    pub fn new(name: &str, agent: &AgentSettings) -> Self {
        Self {
            name: RwSignal::new(name.to_string()),
            command: RwSignal::new(agent.command.join("\n")),
            trust_all: RwSignal::new(agent.trust_all),
            stored: StoredValue::new(agent.clone()),
        }
    }

    /// Its fields, read where the caller tracks.
    #[must_use]
    pub fn fields(self) -> AgentFields {
        AgentFields {
            name: self.name.get(),
            command: self.command.get(),
            trust_all: self.trust_all.get(),
            stored: self.stored.get_value(),
        }
    }

    /// Every stored agent, as drafts.
    #[must_use]
    pub fn all(settings: &TaskSettings) -> Vec<Self> {
        settings
            .agents
            .iter()
            .map(|(name, agent)| Self::new(name, agent))
            .collect()
    }
}

/// An agent's fields as typed.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct AgentFields {
    pub name: String,
    /// One argument per line.
    pub command: String,
    pub trust_all: bool,
    /// The agent as stored; empty for a new one.
    pub stored: AgentSettings,
}

impl AgentFields {
    /// The command to save: as stored while its text is unchanged.
    fn args(&self) -> Vec<String> {
        let stored = &self.stored.command;
        if !stored.is_empty() && self.command == stored.join("\n") {
            stored.clone()
        } else {
            command_args(&self.command)
        }
    }

    /// What is wrong with the command Save would write, if anything: a
    /// stored one can start with a blank program.
    #[must_use]
    pub fn command_error(&self) -> Option<&'static str> {
        self.args()
            .first()
            .is_none_or(|program| program.trim().is_empty())
            .then_some("Enter the program to run, then its arguments")
    }
}

/// Each draft's fields, read where the caller tracks.
#[must_use]
pub fn agent_fields(agents: &[AgentDraft]) -> Vec<AgentFields> {
    agents.iter().map(|a| a.fields()).collect()
}

/// A command as typed, one argument per line; blank lines are dropped.
#[must_use]
pub fn command_args(text: &str) -> Vec<String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(ToString::to_string)
        .collect()
}

/// An S3 bucket's name: 3 to 63 lowercase letters, digits, dots and hyphens,
/// starting and ending with a letter or digit, with no two dots in a row, and
/// not shaped like an IP address.
fn is_bucket_name(name: &str) -> bool {
    let ends = |b: Option<u8>| b.is_some_and(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    (3..=63).contains(&name.len())
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-')
        && ends(name.bytes().next())
        && ends(name.bytes().last())
        && !name.contains("..")
        && name.parse::<std::net::Ipv4Addr>().is_err()
}

/// The buckets typed, one per line as `s3://name` or `name`, as stored:
/// `s3://name`. Or the first line that is not a bucket.
pub fn task_buckets(text: &str) -> Result<Vec<String>, String> {
    text.lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(|line| {
            let name = line.strip_prefix("s3://").unwrap_or(line);
            let name = name.trim_end_matches('/');
            if is_bucket_name(name) {
                Ok(format!("s3://{name}"))
            } else {
                Err(format!("Not a bucket name: {line}"))
            }
        })
        .collect()
}

/// What is wrong with the name of the agent `at`, if anything. A name taken
/// by an earlier agent is wrong on the later one.
#[must_use]
pub fn agent_name_error(names: &[String], at: usize) -> Option<&'static str> {
    let name = names.get(at)?.trim();
    if name.is_empty() {
        Some("Enter a name")
    } else if names[..at].iter().any(|n| n.trim() == name) {
        Some("Another agent has this name")
    } else {
        None
    }
}

/// What Save writes, or `None` while a field is wrong.
#[must_use]
pub fn task_settings_of(buckets: &str, agents: &[AgentFields]) -> Option<TaskSettings> {
    let blessed_buckets = task_buckets(buckets).ok()?;
    let names: Vec<String> = agents.iter().map(|a| a.name.clone()).collect();
    let mut stored = std::collections::BTreeMap::new();
    for (at, agent) in agents.iter().enumerate() {
        if agent_name_error(&names, at).is_some() || agent.command_error().is_some() {
            return None;
        }
        stored.insert(
            agent.name.trim().to_string(),
            AgentSettings {
                command: agent.args(),
                trust_all: agent.trust_all,
                env: agent.stored.env.clone(),
            },
        );
    }
    Some(TaskSettings {
        blessed_buckets,
        agents: stored,
    })
}

/// What `task_settings.json` holds, read-only, and **Edit…**, which opens
/// [`AgentTasksDialog`]. A file that could not be read says why, and is not
/// offered for editing: the user fixes it by hand.
#[component]
pub fn AgentTasksCard(
    #[prop(into)] stored: Signal<TaskSettingsData>,
    on_edit: Callback<()>,
) -> impl IntoView {
    let muted = |words: &'static str| view! { <span class=style::quiet>{words}</span> }.into_any();
    let buckets = move || {
        stored.with(|t| match t.settings.blessed_buckets.as_slice() {
            [] => muted(NO_BUCKETS),
            buckets => buckets
                .iter()
                .map(|b| view! { <span class=style::long>{b.clone()}</span> })
                .collect_view()
                .into_any(),
        })
    };
    let agents = move || {
        stored.with(|t| {
            if t.settings.agents.is_empty() {
                return muted("No agents");
            }
            let rows = t
                .settings
                .agents
                .iter()
                .map(|(name, agent)| {
                    view! {
                        <li class=style::agent>
                            <span>{name.clone()}</span>
                            <span class=style::command>{agent.command.join(" ")}</span>
                            {agent
                                .trust_all
                                .then(|| view! { <span class=style::quiet>"Approves every request"</span> })}
                        </li>
                    }
                })
                .collect_view();
            // `list-style: none` drops the list semantics in WebKit, so the
            // role is explicit.
            view! { <ul class=style::agents role="list">{rows}</ul> }.into_any()
        })
    };
    let unreadable = move || stored.with(|t| t.error.clone());
    view! {
        <div class=style::slot data-agent-tasks>
        <Card title="Agent tasks">
            <p class=style::lead>{AGENT_TASKS_USE}</p>
            {move || match unreadable() {
                Some(err) => view! {
                    <p class=style::error>
                        {StateTone::Danger.glyph()}
                        <span>{format!("Can't read task_settings.json: {err}")}</span>
                    </p>
                }
                .into_any(),
                None => view! {
                    <ValueRow name="Run tasks from" value=view! { {buckets} }.into_any() />
                    <ValueRow name="Agents" value=view! { {agents} }.into_any() />
                }
                .into_any(),
            }}
            <div class=style::card_actions>
                <Button
                    disabled=Signal::derive(move || unreadable().is_some())
                    on_click=move |_| on_edit.run(())
                >
                    "Edit…"
                </Button>
            </div>
        </Card>
        </div>
    }
}

/// The popup's fields: the buckets, then each agent with its name, command
/// and trust, *Remove*, and *Add agent*. Apart from the dialog so the gallery
/// can draw them inline.
#[component]
pub fn AgentTasksForm(
    /// One per line.
    buckets: RwSignal<String>,
    agents: RwSignal<Vec<AgentDraft>>,
) -> impl IntoView {
    let bucket_error = Signal::derive(move || buckets.with(|text| task_buckets(text).err()));
    // Drawn again only when an agent is added or removed, so typing keeps
    // its place.
    let rows = move || {
        agents
            .get()
            .into_iter()
            .enumerate()
            .map(|(at, draft)| {
                let name_error = Signal::derive(move || {
                    let names: Vec<String> =
                        agents.with(|all| all.iter().map(|a| a.name.get()).collect());
                    agent_name_error(&names, at).map(ToString::to_string)
                });
                let command_error =
                    Signal::derive(move || draft.fields().command_error().map(ToString::to_string));
                view! {
                    <fieldset class=style::agent_fields>
                        <legend class=style::agent_legend>"Agent"</legend>
                        <FormControl
                            label="Name"
                            error=name_error
                            control=move |id| {
                                view! {
                                    <TextInput
                                        id=id
                                        value=draft.name
                                        invalid=Signal::derive(move || name_error.with(Option::is_some))
                                    />
                                }
                                    .into_any()
                            }
                        />
                        <FormControl
                            label="Command"
                            caption="One argument per line, the program first."
                            error=command_error
                            control=move |id| {
                                view! { <TextArea id=id value=draft.command rows=3 /> }.into_any()
                            }
                        />
                        <label class=style::rule>
                            <Checkbox
                                state=Signal::derive(move || draft.trust_all.get().into())
                                on_toggle=move |next: bool| draft.trust_all.set(next)
                            />
                            <span>{TRUST_ALL}</span>
                        </label>
                        <p class=style::quiet>{TRUST_ALL_WARNING}</p>
                        <div class=style::card_actions>
                            <Button on_click=move |_| {
                                agents.update(|all| {
                                    all.retain(|a| *a != draft);
                                });
                            }>"Remove"</Button>
                        </div>
                    </fieldset>
                }
            })
            .collect_view()
    };
    view! {
        <FormControl
            label="Run tasks from"
            caption="One bucket per line, as s3://name or name. With none, tasks are off."
            error=bucket_error
            control=move |id| {
                view! { <TextArea id=id value=buckets placeholder="s3://my-bucket" /> }.into_any()
            }
        />
        {rows}
        <div>
            <Button on_click=move |_| {
                agents.update(|all| all.push(AgentDraft::new("", &AgentSettings::default())));
            }>"Add agent"</Button>
        </div>
    }
}

/// *Agent tasks*, as the modal **Edit…** opens: [`AgentTasksForm`], saved by
/// Save, which is off while a field is wrong. The fields are the caller's
/// draft; Cancel leaves them, and the caller fills them again on the next
/// open.
#[component]
pub fn AgentTasksDialog<F, Fut>(
    open: RwSignal<bool>,
    buckets: RwSignal<String>,
    agents: RwSignal<Vec<AgentDraft>>,
    /// Writes the draft. Its `Err` stays in the dialog.
    save: F,
    /// The page's lock, which Save holds too.
    #[prop(optional, into)]
    running: Signal<bool>,
) -> impl IntoView
where
    F: Fn() -> Fut + 'static,
    Fut: Future<Output = Result<(), String>> + 'static,
{
    let blocked = Signal::derive(move || {
        let fields = agents.with(|all| agent_fields(all));
        buckets.with(|text| task_settings_of(text, &fields).is_none())
    });
    view! {
        <FormDialog
            open=open
            title="Agent tasks"
            submit=Submit::new("Save", save)
            running=running
            blocked=blocked
            wide=true
        >
            <AgentTasksForm buckets=buckets agents=agents />
        </FormDialog>
    }
}

// ── Ignore rules ──

pub const IGNORE_RULES_USE: &str = "New files that match a built-in rule stay out of every \
     package. Your rules apply to every package too, and a package's own .quiltignore wins over \
     both.";

pub const IGNORE_RULES_UNREADABLE: &str =
    "Could not read your rules file, so the built-in rules can't be reviewed. Open it to see why.";

/// One built-in ignore rule as the popup lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltInRule {
    pub rule: String,
    /// The heading it is listed under.
    pub group: String,
    /// A few words on what it keeps out.
    pub about: String,
}

/// A built-in rule as the popup names it: a control character as its escape,
/// so the macOS `Icon\r` rule reads as written.
#[must_use]
pub fn rule_shown(rule: &str) -> String {
    rule.chars()
        .map(|c| {
            if c.is_control() {
                c.escape_default().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// What keeps new files out: a short description, the reader's rules file
/// with **Open file**, and the built-in rules with **Review…**, which opens
/// [`IgnoreRulesDialog`].
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads them from there"
)]
pub fn IgnoreRulesCard(
    /// The reader's rules file.
    #[prop(into)]
    file: String,
    /// Whether the file could be read; if not, there is nothing to review.
    readable: bool,
    /// Opens the file, creating it if missing.
    on_open: Callback<()>,
    on_review: Callback<()>,
) -> impl IntoView {
    let note = (!readable).then_some(IGNORE_RULES_UNREADABLE);
    let review = view! {
        <Button disabled=!readable on_click=move |_| on_review.run(())>"Review…"</Button>
    }
    .into_any();
    view! {
        <Card title="Ignore rules">
            <p class=style::lead>{IGNORE_RULES_USE}</p>
            <ValueRow
                name="Your rules"
                value=path_value(file)
                action=view! { <Button on_click=move |_| on_open.run(())>"Open file"</Button> }
                    .into_any()
            />
            {match note {
                Some(note) => view! {
                    <ValueRow name="Built-in rules" note=note action=review />
                }
                .into_any(),
                None => view! { <ValueRow name="Built-in rules" action=review /> }.into_any(),
            }}
        </Card>
    }
}

/// The popup's checklist: each group under its heading, each rule with a
/// box and its words. A box off is the rule turned off. Apart from the
/// dialog so the gallery can draw it inline.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads them from there"
)]
pub fn IgnoreRulesForm(
    /// In the order listed; a group's rules are next to each other.
    rules: Vec<BuiltInRule>,
    /// The rules turned off, as the draft holds them.
    off: RwSignal<std::collections::BTreeSet<String>>,
    /// While a save is out.
    #[prop(optional, into)]
    sealed: Signal<bool>,
) -> impl IntoView {
    let mut groups: Vec<(String, Vec<BuiltInRule>)> = Vec::new();
    for rule in rules {
        match groups.last_mut() {
            Some((group, rows)) if *group == rule.group => rows.push(rule),
            _ => groups.push((rule.group.clone(), vec![rule])),
        }
    }
    groups
        .into_iter()
        .map(|(group, rows)| {
            let items = rows
                .into_iter()
                .map(|BuiltInRule { rule, about, .. }| {
                    let on = {
                        let rule = rule.clone();
                        Signal::derive(move || off.with(|off| !off.contains(&rule)).into())
                    };
                    let shown = rule_shown(&rule);
                    view! {
                        <li>
                            <label class=style::rule>
                                <Checkbox
                                    state=on
                                    on_toggle=move |next: bool| {
                                        off.update(|off| {
                                            if next {
                                                off.remove(&rule);
                                            } else {
                                                off.insert(rule.clone());
                                            }
                                        });
                                    }
                                    disabled=sealed
                                />
                                <code class=style::code>{shown}</code>
                                <span class=style::quiet>{about}</span>
                            </label>
                        </li>
                    }
                })
                .collect_view();
            view! {
                <fieldset class=style::rule_group>
                    <legend class=style::field_name>{group}</legend>
                    <ul class=style::rules>{items}</ul>
                </fieldset>
            }
        })
        .collect_view()
}

/// *Built-in rules*, as the modal **Review…** opens: [`IgnoreRulesForm`],
/// saved by **Confirm**. The boxes are the caller's draft; Cancel or ✕ leaves
/// it, and the caller fills it again on the next open.
#[component]
pub fn IgnoreRulesDialog<F, Fut>(
    open: RwSignal<bool>,
    #[prop(into)] rules: Signal<Vec<BuiltInRule>>,
    off: RwSignal<std::collections::BTreeSet<String>>,
    /// Writes the draft. Its `Err` stays in the dialog.
    save: F,
    /// The page's lock, which Confirm holds too.
    #[prop(optional, into)]
    running: Signal<bool>,
) -> impl IntoView
where
    F: Fn() -> Fut + 'static,
    Fut: Future<Output = Result<(), String>> + 'static,
{
    view! {
        <FormDialog
            open=open
            title="Built-in rules"
            submit=Submit::new("Confirm", save)
            running=running
            wide=true
        >
            {move || view! { <IgnoreRulesForm rules=rules.get() off=off sealed=running /> }}
        </FormDialog>
    }
}

// ── Accounts ──

/// One host the reader has a session at, or had.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AccountHost {
    pub host: String,
    /// The active role; empty when the role query failed.
    pub role: String,
    /// Every role held; more than one draws the switcher.
    pub roles: Vec<String>,
    pub signed_out: bool,
    /// The role is still being read: the row says *Checking role…*.
    pub provisional: bool,
}

pub const NO_ACCOUNTS: &str = "Accounts appear here for each Quilt catalog you sign in to. To \
     sign in, choose Open in QuiltSync on a package in your catalog.";

/// The sentence in the Sign out confirmation.
#[must_use]
pub fn sign_out_consequence(host: &str) -> String {
    format!(
        "This erases the credentials stored for {host}. To use its packages again, you sign in \
         again."
    )
}

/// Every host, with its role, *Sign in* when signed out, and *Sign out*,
/// which asks [`SignOutDialog`] first. Settings holds Sign out; the main
/// page's card does not.
#[component]
#[allow(
    clippy::needless_pass_by_value,
    reason = "a component's props are owned; the body reads them from there"
)]
pub fn AccountsCard(
    hosts: Vec<AccountHost>,
    /// With the host to sign in to.
    on_sign_in: Callback<String>,
    /// `(host, role)` when the reader picks another role. Not on the first
    /// read: only a choice calls it.
    on_role: Callback<(String, String)>,
    /// With the host whose *Sign out* was pressed: open the confirmation.
    on_sign_out: Callback<String>,
    /// The page's lock: no role pick or Sign out while another command runs.
    #[prop(optional, into)]
    locked: Signal<bool>,
) -> impl IntoView {
    if hosts.is_empty() {
        return view! {
            <Card title="Accounts">
                <Blankslate heading="No accounts yet" description=NO_ACCOUNTS />
            </Card>
        }
        .into_any();
    }
    let rows: Vec<AnyView> = hosts
        .into_iter()
        .map(|account| {
            let role = RwSignal::new(account.role);
            let role_host = account.host.clone();
            Effect::watch(
                move || role.get(),
                move |picked, _, _| on_role.run((role_host.clone(), picked.clone())),
                false,
            );
            let sign_in_host = account.host.clone();
            let sign_out_host = account.host.clone();
            view! {
                <HostRow
                    host=account.host
                    role=role
                    roles=account.roles
                    signed_out=account.signed_out
                    provisional=account.provisional
                    locked=locked
                    on_sign_in=move |_| on_sign_in.run(sign_in_host.clone())
                    on_sign_out=Callback::new(move |()| on_sign_out.run(sign_out_host.clone()))
                />
            }
            .into_any()
        })
        .collect();
    view! { <Card title="Accounts">{rows}</Card> }.into_any()
}

/// *Sign out of …*, for the host [`AccountsCard`] asked about.
///
/// Apart from the card, so the page can hold it above what it rebuilds: a
/// re-read redraws the card, and an open confirmation must not close under
/// the reader.
#[component]
pub fn SignOutDialog<F, Fut>(
    /// The host asked about; `None` while nothing is. Closing the dialog sets
    /// it back to `None`.
    host: RwSignal<Option<String>>,
    /// Erases the host's credentials. Its `Err` stays in the confirmation.
    sign_out: F,
    /// The page's lock: the dialog seals while another command runs.
    #[prop(optional, into)]
    running: MaybeProp<bool>,
) -> impl IntoView
where
    F: Fn(String) -> Fut + Clone + Send + Sync + 'static,
    Fut: Future<Output = Result<(), String>> + 'static,
{
    let open = RwSignal::new(false);
    Effect::new(move |_| {
        if host.with(Option::is_some) {
            open.set(true);
        }
    });
    Effect::new(move |was: Option<bool>| {
        let now = open.get();
        if was == Some(true) && !now {
            host.set(None);
        }
        now
    });
    move || {
        host.get().map(|asked| {
            let sign_out = sign_out.clone();
            let target = asked.clone();
            view! {
                <ConfirmDialog
                    open=open
                    title=format!("Sign out of {asked}")
                    consequence=sign_out_consequence(&asked)
                    confirm=Submit::new("Sign out", move || sign_out(target.clone()))
                    running=running
                />
            }
        })
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
    #[prop(optional, into)] pending: Signal<Vec<Saved>>,
) -> impl IntoView {
    view! {
        <Card title="App">
            <ToggleRow
                label="Keep running in the tray when the window is closed"
                sublabel=TRAY_SUBLABEL
                checked=tray
                disabled=saving(pending, Saved::Tray)
                trailing=saved_note(saved, Saved::Tray)
            />
        </Card>
    }
}

// ── Storage ──

/// What the app's own storage measured. Measured only when asked, and not
/// remembered: it lives on the page until the reader leaves it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum StorageSize {
    /// Not asked yet: *Check size*.
    NotMeasured,
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
            Self::NotMeasured | Self::Failed => None,
        }
    }

    /// The line after *Free up space* freed `freed` bytes, from the sweep's
    /// own report rather than a second measure. Unmeasured stays unmeasured.
    #[must_use]
    pub fn after_freeing(self, freed: u64) -> Self {
        match self {
            Self::Measured { total, .. } => Self::Measured {
                total: total.saturating_sub(freed),
                freeable: 0,
            },
            other => other,
        }
    }
}

/// A name, a value, and what to do with it: one line of a card.
#[component]
fn ValueRow(
    #[prop(into)] name: String,
    /// None for a row whose control is its action, such as Log level.
    #[prop(optional)]
    value: Option<AnyView>,
    #[prop(optional)] action: Option<AnyView>,
    /// A line under the value.
    #[prop(optional, into)]
    note: Option<String>,
    /// The note's id, for a control in the action that it describes.
    #[prop(optional, into)]
    note_id: Option<String>,
) -> impl IntoView {
    view! {
        <div class=style::row>
            <div class=style::row_text>
                <span class=style::row_name>{name}</span>
                {value.map(|value| view! { <span class=style::row_value>{value}</span> })}
                {note.map(|note| view! { <span class=style::row_note id=note_id>{note}</span> })}
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
    /// *Check size*, and *Try again* after a failed measure.
    on_measure: Callback<()>,
) -> impl IntoView {
    let nothing = matches!(size, StorageSize::Measured { freeable: 0, .. });
    let home = match home_dir {
        Some(path) => path_value(path),
        None => view! { <span class=style::quiet>"Not set"</span> }.into_any(),
    };
    // Free up space works unmeasured too; it is disabled only when a measure
    // said there is nothing to free.
    let measured = match (&size, size.words()) {
        (_, Some(words)) => view! { <span>{words}</span> }.into_any(),
        (StorageSize::Failed, None) => view! {
            <LoadFailure words="Could not measure it." on_retry=on_measure />
        }
        .into_any(),
        (_, None) => view! {
            <Button disabled=freeing on_click=move |_| on_measure.run(())>"Check size"</Button>
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

pub const DESIGN_PREVIEW_SUBLABEL: &str = "The redesigned QuiltSync.";

pub const AGENT_TASKS_SUBLABEL: &str = "Shows the Agent tasks card, where you choose which \
     buckets can ask an agent on this computer to run a task. Turning it off only hides the card.";

/// Opt-ins for what is still being designed. Entire-package sync is not one
/// of them here: the v2 pages offer it whatever v1's flag says, so a row for
/// it would change nothing a reader of this page can see.
#[component]
pub fn ExperimentalCard(
    design_preview: RwSignal<bool>,
    agent_tasks: RwSignal<bool>,
    #[prop(optional, into)] saved: Signal<Option<Saved>>,
    #[prop(optional, into)] pending: Signal<Vec<Saved>>,
) -> impl IntoView {
    view! {
        <Card title="Experimental">
            <ToggleRow
                label="New design"
                sublabel=DESIGN_PREVIEW_SUBLABEL
                checked=design_preview
                disabled=saving(pending, Saved::DesignPreview)
                trailing=saved_note(saved, Saved::DesignPreview)
            />
            <ToggleRow
                label="Agent tasks"
                sublabel=AGENT_TASKS_SUBLABEL
                checked=agent_tasks
                disabled=saving(pending, Saved::AgentTasks)
                trailing=saved_note(saved, Saved::AgentTasks)
            />
        </Card>
    }
}

// ── Troubleshooting ──

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
pub fn TroubleshootingCard(
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
    #[prop(optional, into)] pending: Signal<Vec<Saved>>,
) -> impl IntoView {
    // A valid `QUILT_LOG` replaces the saved level, so the choice is moot.
    let overridden = matches!(log_env, LogEnv::Overrides(_));
    let caption = match log_env_hint(&log_env) {
        Some(hint) => format!("Applies after a restart. {hint}."),
        None => "Applies after a restart.".to_string(),
    };
    let note_id = crate::kit::unique_id("log-level-note");
    view! {
        <Card title="Troubleshooting">
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
            <ValueRow
                name="Log level"
                note=caption
                note_id=note_id.clone()
                action=trailing_control(
                    saved_note(saved, Saved::LogLevel),
                    view! {
                        <Select
                            naming=Naming::Hidden("Log level".to_string())
                            described_by=note_id
                            options=LOG_LEVELS.iter().map(ToString::to_string).collect()
                            selected=log_level
                            disabled=Signal::derive(move || {
                                overridden || saving(pending, Saved::LogLevel).get()
                            })
                        />
                    }
                        .into_any(),
                )
            />
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

    /// The field's error and its save read the text the same way: a value
    /// too large to save says so instead of being dropped.
    #[test]
    fn the_error_and_the_save_agree() {
        assert_eq!(minutes_secs("3"), Ok(180));
        assert_eq!(
            minutes_secs(&(u64::MAX / 60).to_string()),
            Ok(u64::MAX / 60 * 60)
        );
        assert_eq!(minutes_secs(&i64::MAX.to_string()), Err("Too many minutes"));
        assert_eq!(
            minutes_secs("99999999999999999999999"),
            Err("Too many minutes")
        );
        assert_eq!(
            minutes_secs("-99999999999999999999999"),
            Err("At least 1 minute")
        );
        assert_eq!(
            minutes_error(&i64::MAX.to_string()),
            Some("Too many minutes")
        );
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
        assert_eq!(StorageSize::NotMeasured.words(), None);
    }

    #[test]
    fn freeing_updates_the_line_without_measuring_again() {
        assert_eq!(
            StorageSize::Measured {
                total: 3_200_000_000,
                freeable: 1_100_000_000
            }
            .after_freeing(1_100_000_000),
            StorageSize::Measured {
                total: 2_100_000_000,
                freeable: 0
            }
        );
        assert_eq!(
            StorageSize::NotMeasured.after_freeing(1_000),
            StorageSize::NotMeasured
        );
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

    /// The minutes sit at the row's end, outside its label, named as the
    /// sub-label says; an invalid value's error is under the row and
    /// describes the field.
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn the_minutes_are_at_the_rows_end_with_the_error_under_it() {
        use wasm_bindgen::JsCast;

        let el = crate::test_support::mount(move || {
            view! {
                <SyncingCard
                    pull=RwSignal::new(true)
                    pull_minutes=RwSignal::new("0".to_string())
                    publish=RwSignal::new(false)
                    publish_minutes=RwSignal::new("5".to_string())
                    watch=RwSignal::new(true)
                />
            }
        });
        let text = el.text_content().unwrap_or_default();
        assert!(!text.contains("Every"), "no sentence: {text}");
        assert!(text.contains(&format!("{PULL_MINUTES}.")), "got: {text}");
        assert!(text.contains(&format!("{PUBLISH_MINUTES}.")), "got: {text}");

        let field: web_sys::HtmlInputElement = el
            .query_selector("input[type=number]")
            .unwrap()
            .expect("the pull minutes")
            .dyn_into()
            .unwrap();
        assert_eq!(
            field.get_attribute("aria-label").as_deref(),
            Some(PULL_MINUTES)
        );
        assert!(
            field.closest("label").unwrap().is_none(),
            "outside the label"
        );

        let error_id = field.get_attribute("aria-describedby").expect("described");
        let error = el
            .query_selector(&format!("#{error_id}"))
            .unwrap()
            .expect("the error");
        assert!(
            error
                .text_content()
                .unwrap_or_default()
                .contains("At least 1 minute"),
            "got: {:?}",
            error.text_content()
        );
        assert!(
            error.closest("label").unwrap().is_none(),
            "under the row, not in it"
        );
    }

    /// Log level is a row like Report a problem: the select is named *Log
    /// level* and described by the note under the name.
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn the_log_level_select_is_named_and_described_by_its_note() {
        let el = crate::test_support::mount(move || {
            view! {
                <TroubleshootingCard
                    on_report=Callback::new(|()| ())
                    log_level=RwSignal::new("Default".to_string())
                    log_env=LogEnv::Overrides("quilt_rs=trace".into())
                    logs_dir="/logs"
                    on_open_logs=Callback::new(|()| ())
                    data_dir="/data"
                    on_open_data=Callback::new(|()| ())
                />
            }
        });
        let select = el.query_selector("select").unwrap().expect("the select");
        assert_eq!(
            select.get_attribute("aria-label").as_deref(),
            Some("Log level")
        );
        let note_id = select.get_attribute("aria-describedby").expect("described");
        let note = el
            .query_selector(&format!("#{note_id}"))
            .unwrap()
            .expect("the note")
            .text_content()
            .unwrap_or_default();
        assert_eq!(
            note,
            "Applies after a restart. Set by the QUILT_LOG environment variable \
             (quilt_rs=trace)."
        );
        assert!(select.has_attribute("disabled"), "QUILT_LOG decides");
    }

    /// A minute field saves on Enter or leaving it, and only a valid value.
    #[wasm_bindgen_test::wasm_bindgen_test]
    fn only_valid_minutes_are_saved() {
        use wasm_bindgen::JsCast;

        let minutes = RwSignal::new("1".to_string());
        let saves = RwSignal::new(0);
        let el = crate::test_support::mount(move || {
            view! {
                <SyncingCard
                    pull=RwSignal::new(true)
                    pull_minutes=minutes
                    publish=RwSignal::new(false)
                    publish_minutes=RwSignal::new("5".to_string())
                    watch=RwSignal::new(true)
                    on_pull_minutes=Callback::new(move |()| saves.update(|n| *n += 1))
                />
            }
        });
        let field: web_sys::HtmlInputElement = el
            .query_selector("input[type=number]")
            .unwrap()
            .expect("the pull minutes")
            .dyn_into()
            .unwrap();
        let change = || {
            field
                .dispatch_event(&web_sys::Event::new("change").unwrap())
                .unwrap();
        };
        minutes.set("0".to_string());
        change();
        assert_eq!(saves.get_untracked(), 0, "0 is not saved");
        minutes.set("3".to_string());
        change();
        assert_eq!(saves.get_untracked(), 1, "3 is");
    }

    /// Picking a role reaches the page with its host; drawing the rows does not.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn a_picked_role_reaches_the_page() {
        let picked = RwSignal::new(Vec::<(String, String)>::new());
        let el = crate::test_support::mount(move || {
            view! {
                <AccountsCard
                    hosts=vec![AccountHost {
                        host: "quilt.test".to_string(),
                        role: "analyst".to_string(),
                        roles: vec!["analyst".to_string(), "admin".to_string()],
                        signed_out: false,
                        provisional: false,
                    }]
                    on_sign_in=Callback::new(|_: String| ())
                    on_role=Callback::new(move |pick| picked.update(|all| all.push(pick)))
                    on_sign_out=Callback::new(|_: String| ())
                />
            }
        });
        leptos::task::tick().await;
        assert!(picked.get_untracked().is_empty(), "no call before a choice");
        let select: web_sys::HtmlSelectElement = wasm_bindgen::JsCast::dyn_into(
            el.query_selector("select")
                .unwrap()
                .expect("the role switcher"),
        )
        .unwrap();
        select.set_value("admin");
        select
            .dispatch_event(&web_sys::Event::new("change").unwrap())
            .unwrap();
        leptos::task::tick().await;
        assert_eq!(
            picked.get_untracked(),
            vec![("quilt.test".to_string(), "admin".to_string())]
        );
    }

    fn form(metadata: &str) -> impl IntoView + use<> {
        let metadata = RwSignal::new(metadata.to_string());
        view! {
            <PublishingForm
                template=RwSignal::new(String::new())
                workflow=RwSignal::new(String::new())
                metadata=metadata
                fresh=RwSignal::new(0)
            />
        }
    }

    /// The editor a reader sees is named by the field's label and, while the
    /// JSON fails, described by its error: the glue hides the textarea that
    /// the label points at.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn the_visible_metadata_editor_is_named_and_described() {
        let el = crate::test_support::mount(|| form("{\"source\": "));
        leptos::task::tick().await;
        let group = el
            .query_selector("[role=group]")
            .unwrap()
            .expect("the editor");
        let named_by = group.get_attribute("aria-labelledby").expect("a name");
        let label = el
            .query_selector(&format!("#{named_by}"))
            .unwrap()
            .expect("the label it points at");
        assert_eq!(label.text_content().as_deref(), Some("Default metadata"));
        let described_by = group.get_attribute("aria-describedby").expect("the error");
        let error = el
            .query_selector(&format!("#{described_by}"))
            .unwrap()
            .expect("the error it points at")
            .text_content()
            .unwrap_or_default();
        assert!(error.contains("Not valid JSON"), "{error}");
    }

    /// A click on the open editor's label lands in the editor. Here, with no
    /// editor bundle, that is the textarea the label names; in the app the
    /// label hands focus to the editor that hides it.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn a_click_on_the_metadata_label_focuses_the_editor() {
        // An earlier test's modal, left open, would make this page inert.
        crate::test_support::unmount_earlier();
        let el = crate::test_support::mount(|| form("{}"));
        leptos::task::tick().await;
        crate::test_support::element_saying(&el, "Default metadata").click();
        let focused = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .active_element()
            .expect("something has focus");
        assert_eq!(focused.tag_name(), "TEXTAREA", "{}", el.inner_html());
    }

    /// *Reset to defaults*, at the footer's start, empties the three fields
    /// and draws the editor again over no metadata; it saves nothing.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn reset_fills_the_defaults() {
        crate::test_support::unmount_earlier();
        let template = RwSignal::new("Plate {date}".to_string());
        let workflow = RwSignal::new("release".to_string());
        let metadata = RwSignal::new("{\"a\": 1}".to_string());
        let fresh = RwSignal::new(0);
        let saves = RwSignal::new(0);
        let el = crate::test_support::mount(move || {
            view! {
                <PublishingDialog
                    open=RwSignal::new(true)
                    template=template
                    workflow=workflow
                    metadata=metadata
                    fresh=fresh
                    save=move || async move {
                        saves.update(|n| *n += 1);
                        Ok(())
                    }
                />
            }
        });
        leptos::task::tick().await;
        let reset = crate::test_support::button_saying(&el, "Reset to defaults");
        assert!(
            reset.closest("form").unwrap().is_none(),
            "in the footer, not the form"
        );
        assert_eq!(reset.get_attribute("type").as_deref(), Some("button"));
        reset.click();
        leptos::task::tick().await;
        assert_eq!(template.get_untracked(), "");
        assert_eq!(workflow.get_untracked(), "");
        assert_eq!(metadata.get_untracked(), "");
        assert_eq!(fresh.get_untracked(), 1, "the editor is drawn again");
        assert_eq!(saves.get_untracked(), 0, "nothing saved");
        let textarea: web_sys::HtmlTextAreaElement = wasm_bindgen::JsCast::dyn_into(
            el.query_selector("textarea")
                .unwrap()
                .expect("the editor's field"),
        )
        .unwrap();
        assert_eq!(textarea.value(), "");
        assert!(
            !el.text_content()
                .unwrap_or_default()
                .contains("every bucket"),
            "and the every-bucket warning is gone"
        );
    }

    /// The card says what each empty value means, muted, and has one button.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn the_card_reads_the_stored_values() {
        crate::test_support::unmount_earlier();
        let stored = RwSignal::new(PublishSettingsData::default());
        let el = crate::test_support::mount(move || {
            view! { <PublishingCard stored=stored on_edit=Callback::new(|()| ()) /> }
        });
        leptos::task::tick().await;
        let text = || el.text_content().unwrap_or_default();
        for words in [
            "Generated from the changes: {changes}",
            "Each bucket's default",
            "No metadata",
        ] {
            assert!(text().contains(words), "{words}: {}", text());
        }
        stored.set(PublishSettingsData {
            message_template: "Plate {date}".to_string(),
            default_workflow: "release".to_string(),
            default_metadata: "{\"source\": \"desktop\"}".to_string(),
        });
        leptos::task::tick().await;
        assert!(text().contains("Plate {date}"), "{}", text());
        assert!(text().contains("release"));
        assert!(!text().contains("No metadata"));
        crate::test_support::button_saying(&el, "Edit…");
        assert!(
            el.query_selector("input, textarea").unwrap().is_none(),
            "read-only"
        );
    }

    #[test]
    fn buckets_are_stored_as_s3_uris() {
        assert_eq!(
            task_buckets("s3://quilt-dev\n\n  plate-reads/ \n"),
            Ok(vec![
                "s3://quilt-dev".to_string(),
                "s3://plate-reads".to_string()
            ])
        );
        assert_eq!(task_buckets(""), Ok(Vec::new()));
        assert_eq!(
            task_buckets("quilt-dev\nQuilt Dev"),
            Err("Not a bucket name: Quilt Dev".to_string())
        );
        assert!(task_buckets("s3://quilt-dev/data").is_err());
        assert!(task_buckets("ab").is_err());
        assert!(task_buckets("-quilt").is_err());
        assert!(task_buckets("quilt..dev").is_err(), "two dots in a row");
        assert!(task_buckets("192.168.5.4").is_err(), "an IP address");
        assert!(task_buckets("quilt.dev.2").is_ok());
    }

    #[test]
    fn a_command_is_one_argument_per_line() {
        assert_eq!(
            command_args("/Applications/Kiro CLI.app/kiro\n\n acp \n"),
            vec!["/Applications/Kiro CLI.app/kiro", "acp"]
        );
        let typed = |command: &str| AgentFields {
            command: command.to_string(),
            ..AgentFields::default()
        };
        assert_eq!(
            typed(" \n").command_error(),
            Some("Enter the program to run, then its arguments")
        );
        assert_eq!(typed("kiro").command_error(), None);
    }

    /// A stored command whose program is blank cannot be saved as it is,
    /// and says so under Command; edited, its blank line is dropped.
    #[test]
    fn a_stored_blank_program_is_an_error() {
        for program in ["", "  "] {
            let stored = vec![program.to_string(), "acp".to_string()];
            let mut fields = AgentFields {
                name: "kiro".to_string(),
                command: stored.join("\n"),
                stored: AgentSettings {
                    command: stored,
                    ..AgentSettings::default()
                },
                ..AgentFields::default()
            };
            assert_eq!(
                fields.command_error(),
                Some("Enter the program to run, then its arguments"),
                "{program:?}"
            );
            assert_eq!(task_settings_of("", std::slice::from_ref(&fields)), None);
            fields.command = "kiro\nacp".to_string();
            assert_eq!(fields.command_error(), None);
        }
    }

    /// The popup does not show an agent's variables, and a save keeps them,
    /// whatever else changed.
    #[test]
    fn a_save_keeps_each_agents_env() {
        let env: std::collections::BTreeMap<String, String> =
            [("DENO_DIR".to_string(), "/tmp/deno".to_string())].into();
        let fields = AgentFields {
            name: "kiro".to_string(),
            command: "kiro\nacp\n--verbose".to_string(),
            trust_all: true,
            stored: AgentSettings {
                command: vec!["kiro".to_string(), "acp".to_string()],
                trust_all: false,
                env: env.clone(),
            },
        };
        let saved = task_settings_of("", &[fields]).expect("valid");
        assert_eq!(saved.agents["kiro"].env, env);
    }

    #[test]
    fn an_agent_needs_a_name_of_its_own() {
        let names = ["kiro".to_string(), " ".to_string(), "kiro ".to_string()];
        assert_eq!(agent_name_error(&names, 0), None);
        assert_eq!(agent_name_error(&names, 1), Some("Enter a name"));
        assert_eq!(
            agent_name_error(&names, 2),
            Some("Another agent has this name")
        );
    }

    #[test]
    fn save_writes_only_a_valid_draft() {
        let agent = |name: &str, command: &str| AgentFields {
            name: name.to_string(),
            command: command.to_string(),
            ..AgentFields::default()
        };
        assert_eq!(
            task_settings_of("quilt-dev", &[agent(" kiro ", "kiro\nacp")]),
            Some(TaskSettings {
                blessed_buckets: vec!["s3://quilt-dev".to_string()],
                agents: [(
                    "kiro".to_string(),
                    AgentSettings {
                        command: vec!["kiro".to_string(), "acp".to_string()],
                        ..AgentSettings::default()
                    },
                )]
                .into_iter()
                .collect(),
            })
        );
        assert_eq!(task_settings_of("Quilt Dev", &[]), None);
        assert_eq!(task_settings_of("", &[agent("kiro", "")]), None);
        assert_eq!(
            task_settings_of("", &[agent("kiro", "a"), agent("kiro", "b")]),
            None
        );
    }

    /// A stored command the box cannot show, such as one with an empty
    /// argument, is saved as it is until its text is edited.
    #[test]
    fn an_unedited_command_is_saved_as_stored() {
        let stored: Vec<String> = ["sh", "-c", "exec agent \"$@\"", "", "--acp"]
            .iter()
            .map(ToString::to_string)
            .collect();
        let mut fields = AgentFields {
            name: "wrapped".to_string(),
            command: stored.join("\n"),
            trust_all: false,
            stored: AgentSettings {
                command: stored.clone(),
                ..AgentSettings::default()
            },
        };
        let saved = |fields: &AgentFields| {
            task_settings_of("", std::slice::from_ref(fields))
                .unwrap()
                .agents["wrapped"]
                .command
                .clone()
        };
        assert_eq!(saved(&fields), stored);
        fields.command.push_str("\n--verbose");
        assert_eq!(
            saved(&fields),
            ["sh", "-c", "exec agent \"$@\"", "--acp", "--verbose"]
        );
    }

    /// The grid's styles and the card rows', for a layout test.
    fn load_layout_styles() {
        stylance::import_crate_style!(grid, "src/kit/two_columns.module.scss");
        crate::test_support::load_module_style(
            include_str!("../kit/two_columns.module.scss"),
            &[
                ("grid", grid::grid),
                ("tight", grid::tight),
                ("fill", grid::fill),
            ],
        );
        // The rules alone: the file's `//` comments are not CSS.
        let scss = include_str!("settings_v2.module.scss");
        let rule = |name: &str| {
            let start = scss.find(&format!(".{name} {{")).expect("the rule");
            let end = start + scss[start..].find('}').expect("its end") + 1;
            scss[start..end].to_string()
        };
        let css = [
            "slot",
            "row",
            "row_text",
            "row_value",
            "agents",
            "agent",
            "command",
        ]
        .map(rule)
        .join("\n");
        crate::test_support::load_module_style(
            &css,
            &[
                ("slot", style::slot),
                ("row", style::row),
                ("row_text", style::row_text),
                ("row_value", style::row_value),
                ("agents", style::agents),
                ("agent", style::agent),
                ("command", style::command),
            ],
        );
    }

    /// Agent tasks shares Ignore rules' row: wide, it is beside it; narrow,
    /// it is next in the one column. A long command wraps inside the card.
    #[wasm_bindgen_test::wasm_bindgen_test]
    #[allow(clippy::float_cmp, reason = "box edges here are whole pixels")]
    async fn agent_tasks_sits_beside_ignore_rules() {
        load_layout_styles();
        let long = TaskSettingsData {
            settings: TaskSettings {
                blessed_buckets: Vec::new(),
                agents: [(
                    "codex".to_string(),
                    AgentSettings {
                        command: vec![
                            "/Users/a/.nvm/versions/node/v24.12.0/lib/node_modules/\
                             @agentclientprotocol/codex-acp/dist/index.js"
                                .to_string(),
                            // No place a browser breaks on its own.
                            format!("--key={}", "x".repeat(150)),
                        ],
                        ..AgentSettings::default()
                    },
                )]
                .into_iter()
                .collect(),
            },
            error: None,
        };
        for width in [1024, 640] {
            crate::test_support::unmount_earlier();
            let card =
                |name: &'static str| view! { <section data-card=name>{name}</section> }.into_any();
            let stored = long.clone();
            let el = crate::test_support::mount(move || {
                view! {
                    <div style=format!("--q-space-4:16px; width:{width}px")>
                        <SettingsLayout
                            header=().into_any()
                            syncing=card("syncing")
                            publishing=card("publishing")
                            ignore_rules=card("ignore")
                            accounts=card("accounts")
                            app=card("app")
                            storage=card("storage")
                            experimental=card("experimental")
                            troubleshooting=card("troubleshooting")
                            about=card("about")
                            agent_tasks=view! {
                                <AgentTasksCard stored=stored on_edit=Callback::new(|()| ()) />
                            }
                                .into_any()
                        />
                    </div>
                }
            });
            crate::test_support::sleep_ms(0).await;
            let rect = |selector: &str| {
                el.query_selector(selector)
                    .unwrap()
                    .expect(selector)
                    .get_bounding_client_rect()
            };
            let ignore = rect("[data-card=ignore]");
            let tasks = rect("[data-agent-tasks] > *");
            if width == 1024 {
                assert_eq!(tasks.top(), ignore.top(), "{width}: on Ignore rules' row");
                assert!(tasks.left() > ignore.right(), "{width}: to its right");
            } else {
                assert_eq!(tasks.left(), ignore.left(), "{width}: one column");
                assert!(tasks.top() > ignore.bottom(), "{width}: under it");
            }
            let command = el
                .query_selector("[data-agent-tasks] li > :nth-child(2)")
                .unwrap()
                .expect("the command");
            assert!(
                command.scroll_width() <= command.client_width(),
                "{width}: the command wraps inside its box"
            );
            assert!(
                command.get_bounding_client_rect().right() <= tasks.right() + 0.5,
                "{width}: and the box inside the card"
            );
        }
    }

    /// An agent's legend breaks only part of its fieldset's border.
    #[wasm_bindgen_test::wasm_bindgen_test]
    async fn the_agent_legend_is_narrow() {
        crate::test_support::unmount_earlier();
        let scss = include_str!("settings_v2.module.scss");
        let start = scss.find(".agent_fields {").expect("the rule");
        // Without the file's `//` comments, which are not CSS.
        let css: String = scss[start..]
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
            .collect::<Vec<_>>()
            .join("\n");
        // As the app's normalize sets every box.
        let css = format!("{css}\n[data-legend-test] legend {{ box-sizing: border-box; }}");
        crate::test_support::load_module_style(
            &css,
            &[
                ("agent_fields", style::agent_fields),
                ("agent_legend", style::agent_legend),
            ],
        );
        let el = crate::test_support::mount(|| {
            let agents = RwSignal::new(vec![AgentDraft::new("kiro", &AgentSettings::default())]);
            view! {
                <div data-legend-test style="width:600px">
                    <AgentTasksForm buckets=RwSignal::new(String::new()) agents=agents />
                </div>
            }
        });
        crate::test_support::sleep_ms(0).await;
        let legend = el.query_selector("legend").unwrap().expect("the legend");
        assert_eq!(legend.get_bounding_client_rect().width(), 48.0);
    }

    /// v2 vocabulary: no commit, push, pull or remote in the page's words.
    #[test]
    fn no_fixed_string_uses_a_banned_word() {
        let mut all = vec![
            WATCH_SUBLABEL,
            PUBLISHING_USE,
            GLOBAL_SCOPE_WARNING,
            TRAY_SUBLABEL,
            DESIGN_PREVIEW_SUBLABEL,
            AGENT_TASKS_SUBLABEL,
            AGENT_TASKS_USE,
            NO_BUCKETS,
            TRUST_ALL,
            TRUST_ALL_WARNING,
            TEMPORARY_LOGS,
            EMAIL_INSTEAD,
            REPORT_SENT,
            NO_ACCOUNTS,
        ];
        let consequence = sign_out_consequence("quilt.test");
        all.push(&consequence);
        all.extend(REPORT_CONTENTS);
        for words in all {
            assert_eq!(banned_in(words), None, "{words:?}");
        }
    }
}
