//! The settings page v2, one cell per state.
//!
//! Every card is `pages::settings_v2`'s own, over fixture props, so the port
//! and this scene cannot drift. Callbacks are dropped. Cards are drawn at the
//! page column's width; the last cells are the whole page at four window
//! sizes. Its appbar has no actions: Refresh and Settings belong to the
//! pages that list packages.
//!
//! # Measured in Chromium
//!
//! At 1024×560 the header ends at 138px and the cards are two to a row, 488px
//! wide and 16px apart; a row's two cards share its height. Rows start at
//! Syncing | Accounts 154, Storage | App 498, Publishing | Troubleshooting 694,
//! Experimental | About 1054. The first screen shows Syncing and Accounts.
//!
//! At 1000×560, just under the switch, one column of 968px in the same order:
//! Syncing 154, Accounts 477, Storage 713, App 909, Publishing 1057,
//! Troubleshooting 1354, Experimental 1693, About 1821. At 640, the same order
//! at 608px; only the Data folder's path is cut short, with its whole path in
//! the tooltip. No width scrolls sideways.
//!
//! The Publishing popup is a wide dialog, 640px, with the metadata editor
//! opening at 260px. With a workflow, metadata and so the warning, it is
//! 694px tall in a 1280×900 window. In a 640×560 one it is 606px wide, held
//! to 528px tall, and scrolls inside, the buttons under the fold. Scrolled
//! there so the editor's context menu reaches past the popup's lower edge,
//! the menu is not cropped: a point just past the edge lands on it. Without
//! the rule in `_base.scss` that frees the editor's menus from a modal's
//! clip, it does not.
//!
//! # Left open
//!
//! - The metadata editor repairs JSON it can (a missing `}`) and says so in
//!   its own green bar, above the page's *Not valid JSON* error. The two
//!   disagree until the reader picks the editor's repair.
//! - The editor's context menu cuts some of its own labels short (`Edit valu`),
//!   as on the commit page.

use leptos::prelude::*;

use crate::Cell;
use crate::Scene;
use crate::kit::Button;
use crate::kit::ButtonVariant;
use crate::kit::PageLayout;
use crate::pages::settings_v2::{
    AboutCard, AccountHost, AccountsCard, AppCard, ExperimentalCard, PublishingCard,
    PublishingDialog, PublishingForm, ReportBody, ReportDialog, ReportFooter, ReportState, Saved,
    SettingsHeader, SettingsLayout, SignOutDialog, StorageCard, StorageSize, SyncingCard,
    TroubleshootingCard, metadata_error, minutes_shown, sign_out_consequence,
};
use quilt_sync_ui::commands::{LogEnv, PublishSettingsData};

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

const LONG_TEMPLATE: &str = "Plate reads for {namespace}, {datetime}: {changes}. Instrument \
    SpectraMax iD5, read after recalibration; see the run sheet in the lab notebook for the \
    protocol version and the operator";
/// More than the folded line holds.
const LARGE_METADATA: &str = "{\"source\": \"desktop\", \"lab\": \"plate-reader-2\", \
    \"instrument\": \"SpectraMax iD5\", \"wavelength_nm\": 450, \"wells\": [\"A1\", \
    \"A2\", \"A3\", \"B1\"], \"protocol\": {\"version\": 3, \"incubation_min\": 60, \
    \"reagents\": [\"TMB\", \"stop solution\"]}, \"notes\": \"Second read after \
    recalibration\"}";
const METADATA: &str = "{\n  \"source\": \"desktop\",\n  \"lab\": \"plate-reader-2\"\n}";

#[derive(Clone, Copy, Default)]
struct Publishing {
    template: &'static str,
    workflow: &'static str,
    metadata: &'static str,
}

impl Publishing {
    fn stored(self) -> PublishSettingsData {
        PublishSettingsData {
            message_template: self.template.to_string(),
            default_workflow: self.workflow.to_string(),
            default_metadata: self.metadata.to_string(),
        }
    }
}

const CUSTOM: Publishing = Publishing {
    template: "Updated {namespace} on {date} ({changes})",
    workflow: "plate-reads",
    metadata: METADATA,
};

fn publishing(p: Publishing) -> AnyView {
    view! { <PublishingCard stored=p.stored() on_edit=noop() /> }.into_any()
}

/// The popup drawn inline at the modal's width, as the report is; the last
/// cell opens the real one.
fn publishing_popup(p: Publishing, saving: bool) -> AnyView {
    view! {
        <div class="g-bars g-dialog-inline g-dialog-inline--wide">
            <form>
                <fieldset class="g-bars g-fieldset" disabled=saving>
                    <PublishingForm
                        template=RwSignal::new(p.template.to_string())
                        workflow=RwSignal::new(p.workflow.to_string())
                        metadata=RwSignal::new(p.metadata.to_string())
                        fresh=RwSignal::new(0)
                        sealed=saving
                    />
                </fieldset>
            </form>
            <div class="g-inline g-inline--end">
                <span class="g-inline__start">
                    <Button disabled=saving on_click=move |_| ()>"Reset to defaults"</Button>
                </span>
                <Button disabled=saving on_click=move |_| ()>"Cancel"</Button>
                <Button
                    variant=ButtonVariant::Primary
                    disabled=metadata_error(p.metadata).is_some()
                    loading=saving
                    on_click=move |_| ()
                >
                    "Save"
                </Button>
            </div>
        </div>
    }
    .into_any()
}

/// *Edit…* and the real popup behind it.
fn publishing_real(p: Publishing) -> AnyView {
    let open = RwSignal::new(false);
    let stored = p.stored();
    view! {
        <div class="g-inline">
            <Button on_click=move |_| open.set(true)>"Open the real one"</Button>
        </div>
        <PublishingDialog
            open=open
            template=RwSignal::new(stored.message_template)
            workflow=RwSignal::new(stored.default_workflow)
            metadata=RwSignal::new(stored.default_metadata)
            fresh=RwSignal::new(0)
            save=|| async { Ok(()) }
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
                on_measure=noop()
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
                on_measure=noop()
            />
        }
        .into_any(),
    }
}

// ── Accounts ──

fn host(host: &str, role: &str, roles: &[&str], signed_out: bool) -> AccountHost {
    AccountHost {
        host: host.to_string(),
        role: role.to_string(),
        roles: roles.iter().map(ToString::to_string).collect(),
        signed_out,
        provisional: false,
    }
}

/// The card, with the real confirmation behind its Sign out.
fn accounts(hosts: Vec<AccountHost>) -> AnyView {
    let asked = RwSignal::new(None::<String>);
    view! {
        <AccountsCard
            hosts=hosts
            on_sign_in=Callback::new(|_: String| ())
            on_role=Callback::new(|_: (String, String)| ())
            on_sign_out=Callback::new(move |host| asked.set(Some(host)))
        />
        <SignOutDialog host=asked sign_out=|_: String| async { Ok(()) } />
    }
    .into_any()
}

fn several_hosts() -> Vec<AccountHost> {
    vec![
        host(
            "open.quiltdata.com",
            "analyst",
            &["analyst", "bench-scientist", "admin"],
            false,
        ),
        host("demo.quiltdata.com", "ReadWriteQuiltV2", &[], false),
        host("custom.registry.io", "", &[], true),
    ]
}

/// The Sign out confirmation drawn inline, as `confirm_dialog.rs` draws its own.
fn sign_out_inline() -> AnyView {
    view! {
        <div class="g-bars g-dialog-inline">
            <p class="g-consequence">{sign_out_consequence("open.quiltdata.com")}</p>
            <div class="g-inline g-inline--end">
                <Button on_click=move |_| ()>"Cancel"</Button>
                <Button variant=ButtonVariant::Danger on_click=move |_| ()>"Sign out"</Button>
            </div>
        </div>
    }
    .into_any()
}

const MEASURED: StorageSize = StorageSize::Measured {
    total: 3_200_000_000,
    freeable: 1_100_000_000,
};

// ── Troubleshooting ──

fn troubleshooting(log_env: LogEnv, level: &str, temporary: bool) -> AnyView {
    let logs_dir = if temporary { TEMP_LOGS_DIR } else { LOGS_DIR };
    view! {
        <TroubleshootingCard
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

fn whole_page(width: u32, height: u32) -> AnyView {
    let id = format!("settings-page-{width}");
    let home = format!("#{id}");
    view! {
        <div
            id=id
            class="g-window"
            style=format!("width:{width}px; --q-frame-height:{height}px; max-width:100%")
        >
            <PageLayout heading="QuiltSync" banner=().into_any()>
                <SettingsLayout
                    header=view! { <SettingsHeader home_href=home /> }.into_any()
                    syncing=syncing(Syncing::default())
                    publishing=publishing(Publishing::default())
                    accounts=accounts(several_hosts())
                    app=view! { <AppCard tray=RwSignal::new(false) /> }.into_any()
                    storage=storage(StorageSize::NotMeasured, false, None)
                    experimental=view! {
                        <ExperimentalCard
                            design_preview=RwSignal::new(true)
                        />
                    }
                        .into_any()
                    troubleshooting=troubleshooting(LogEnv::Unset, "Default", false)
                    about=view! { <AboutCard version=VERSION on_release_notes=noop() /> }.into_any()
                />
            </PageLayout>
        </div>
    }
    .into_any()
}

const NOTE: &str = "The settings page v2. From a 1024px window, two cards to a row: Syncing and \
    Accounts, Storage and App, Publishing and Troubleshooting, Experimental and About. Narrower, \
    the same order in one column. Toggles, selects and minute fields save as they change and say \
    Saved; Publishing is read-only, edited in a popup with Save. Routed at /settings under New \
    design preview.";

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
            <Cell full=true label="Publishing — all defaults: each value says what happens instead">
                {column(publishing(Publishing::default()))}
            </Cell>
            <Cell full=true label="Publishing — custom values: the template, the workflow, the metadata as a tree">
                {column(publishing(CUSTOM))}
            </Cell>
            <Cell full=true label="Publishing — a long template wraps in the card">
                {column(publishing(Publishing { template: LONG_TEMPLATE, ..Publishing::default() }))}
            </Cell>
            <Cell full=true label="Publishing — large metadata folds to one line; open it to read it">
                {column(publishing(Publishing { metadata: LARGE_METADATA, ..Publishing::default() }))}
            </Cell>
            <Cell wide=true label="The Publishing popup — at rest, nothing stored">
                {publishing_popup(Publishing::default(), false)}
            </Cell>
            <Cell wide=true label="The Publishing popup — edited: a workflow and metadata, so the every-bucket warning">
                {publishing_popup(CUSTOM, false)}
            </Cell>
            <Cell wide=true label="The Publishing popup — after Reset to defaults: the fields empty, nothing saved until Save">
                {publishing_popup(Publishing::default(), false)}
            </Cell>
            <Cell wide=true label="The Publishing popup — metadata that is not valid JSON: the error under the editor, Save disabled">
                {publishing_popup(
                    Publishing { metadata: "{\"source\": \"desktop\",", ..Publishing::default() },
                    false,
                )}
            </Cell>
            <Cell wide=true label="The Publishing popup — saving: the fields sealed, Save busy">
                {publishing_popup(CUSTOM, true)}
            </Cell>
            <Cell
                wide=true
                label="The Publishing popup, the real one — right-click a key in the editor: its menu \
                       is not cropped by the popup"
            >
                {publishing_real(CUSTOM)}
            </Cell>
            <Cell full=true label="Accounts — several hosts: a role switcher, one role, and one signed out">
                {column(accounts(several_hosts()))}
            </Cell>
            <Cell full=true label="Accounts — one host, one role">
                {column(accounts(vec![host("open.quiltdata.com", "analyst", &[], false)]))}
            </Cell>
            <Cell full=true label="Accounts — signed out: Sign in, and no Sign out">
                {column(accounts(vec![host("open.quiltdata.com", "", &[], true)]))}
            </Cell>
            <Cell full=true label="Accounts — no accounts">
                {column(accounts(Vec::new()))}
            </Cell>
            <Cell wide=true label="Accounts — Sign out asks first (press Sign out in the cells above for the real one)">
                {sign_out_inline()}
            </Cell>
            <Cell full=true label="App">
                {column(view! { <AppCard tray=RwSignal::new(true) /> }.into_any())}
            </Cell>
            <Cell full=true label="Storage — not measured: Check size, and Free up space works anyway">
                {column(storage(StorageSize::NotMeasured, false, None))}
            </Cell>
            <Cell full=true label="Storage — measuring, after Check size">
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
            <Cell full=true label="Storage — freed after a measure: the line updated from the sweep's report, not measured again">
                {column(storage(
                    MEASURED.after_freeing(1_100_000_000),
                    false,
                    Some("Freed 1.1\u{a0}GB"),
                ))}
            </Cell>
            <Cell full=true label="Storage — freed without a measure">
                {column(storage(StorageSize::NotMeasured, false, Some("Freed 1.1\u{a0}GB")))}
            </Cell>
            <Cell full=true label="Storage — the measure failed">
                {column(storage(StorageSize::Failed, false, None))}
            </Cell>
            <Cell full=true label="Experimental">
                {column(view! {
                    <ExperimentalCard
                        design_preview=RwSignal::new(true)
                    />
                }
                    .into_any())}
            </Cell>
            <Cell full=true label="Troubleshooting — at rest">
                {column(troubleshooting(LogEnv::Unset, "Default", false))}
            </Cell>
            <Cell full=true label="Troubleshooting — the log level set by QUILT_LOG: disabled, with the hint">
                {column(troubleshooting(LogEnv::Overrides("quilt_rs=trace".to_string()), "Info", false))}
            </Cell>
            <Cell full=true label="Troubleshooting — the logs in a temporary folder">
                {column(troubleshooting(LogEnv::Unset, "Default", true))}
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
            <Cell full=true label="The whole page at 1280×900, the window's default size: rows of two">
                {whole_page(1280, 900)}
            </Cell>
            <Cell full=true label="At 1024×560, where it turns to rows of two">
                {whole_page(1024, 560)}
            </Cell>
            <Cell full=true label="At 1000×560, just under: one column, in the same order">
                {whole_page(1000, 560)}
            </Cell>
            <Cell full=true label="At 640×560, the app's smallest window">
                {whole_page(640, 560)}
            </Cell>
        </Scene>
    }
}
