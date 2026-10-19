//! The routed settings page v2: `get_settings_data` mapped onto the cards
//! above, each control saving as it changes.
//!
//! # Above the rebuild line
//!
//! The body is rebuilt only when what it draws as fixed text changes: the
//! version, the folders, the log variable, the release notes ([`Fixed`]).
//! Everything a reader changes is the page's, made before [`V2Page`], so no
//! re-read takes it away:
//!
//! - every control's value ([`Fields`]): the toggles, the minute fields as
//!   typed, the log level;
//! - the Publishing popup: open or not, and its draft;
//! - which built-in ignore rules are off, and whether the reader has opened
//!   their rules file to edit, so coming back to the window reads it again;
//! - what each control last wrote ([`Written`]), so a value the page seeds
//!   from a read is never written back;
//! - which row just saved (*Saved*);
//! - the storage line: not measured, measuring, measured, freed. Not
//!   remembered past the visit;
//! - the accounts, read on their own, and the host whose Sign out is being
//!   confirmed;
//! - the report dialog: open, its text, where the send is;
//! - the release notes dialog.
//!
//! # Writes
//!
//! A control's save goes on one queue and the queue runs in order, so two
//! quick changes land as chosen. The queue is the app's, not the visit's: a
//! save queued before the reader leaves still runs, before the next visit's. When it empties the page reads again, and
//! only that read, landed, seeds the controls: a read that went out before a
//! write would put back the value the write replaced. A seeded value equal to
//! what the control holds changes nothing. Whenever the queue empties, every
//! open visit reads again, so one opened while an earlier visit's saves ran
//! is not left with the values from before them.
//!
//! Saves are quick and each writes its own setting, so they do not take the
//! page's lock. A control is held still while its own save is out — a toggle
//! or the log level disabled, a minutes field read-only so the cursor stays —
//! so one setting never has two saves in flight, and a failure is never about
//! a value the reader has since replaced. The commands that run longer do — Free up space, a role
//! switch, Sign out — and so does the Publishing popup's Save, which waits for its
//! own write on the queue; the dialogs read it.

use std::collections::BTreeSet;
use std::collections::VecDeque;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::commands::{
    self, AccountHostData, AutosyncPatch, ChangelogEntry, FreedSpace, LogEnv, PublishSettingsData,
    SettingsData,
};
use crate::kit::{
    Button, ButtonVariant, Card, Dialog, HostRowSkeleton, LoadFailure, PageLayout,
    ToggleRowSkeleton,
};

use super::super::v2_page::{PageHandle, PageRead, V2Page, critical, hold};
use super::{
    AboutCard, AccountHost, AccountsCard, AppCard, ExperimentalCard, IgnoreRulesCard,
    PublishingCard, PublishingDialog, ReportDialog, ReportState, Saved, SettingsHeader,
    SettingsLayout, SignOutDialog, StorageCard, StorageSize, SyncingCard, TroubleshootingCard,
    log_level_label, minutes_secs, minutes_shown,
};

use super::style;

/// One command's answer.
pub(crate) type Answer<T> = Pin<Box<dyn Future<Output = Result<T, String>>>>;

/// One setting, written.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Write {
    Autosync(AutosyncPatch),
    Watch(bool),
    DesignPreview(bool),
    /// Lowercase, as the backend names it.
    LogLevel(String),
    Publishing(PublishSettingsData),
    /// A built-in ignore rule turned on or off for every package.
    IgnoreRule {
        rule: String,
        on: bool,
    },
}

/// What the page reads and writes, as parameters so a test can answer
/// without a Tauri host. The rest of its commands are called directly.
#[derive(Clone, Copy)]
pub(crate) struct SettingsIo {
    pub read: PageRead<SettingsData>,
    pub write: fn(Write) -> Answer<()>,
    pub measure: fn() -> Answer<commands::StorageSize>,
    pub free: fn() -> Answer<FreedSpace>,
    /// The accounts' light read: every host, roles not yet asked.
    pub accounts: fn() -> Answer<Vec<AccountHostData>>,
    /// One host's role, asked.
    pub settle_account: fn(String) -> Answer<AccountHostData>,
    /// Erase one host's credentials: `erase_auth`, which also drops the
    /// host's cached clients.
    pub sign_out: fn(String) -> Answer<String>,
    /// Open the reader's ignore rules file, creating it if missing.
    pub edit_rules: fn() -> Answer<()>,
}

fn read_settings(_: String) -> Answer<SettingsData> {
    Box::pin(commands::get_settings_data())
}

fn write_setting(write: Write) -> Answer<()> {
    Box::pin(async move {
        match write {
            Write::Autosync(patch) => commands::patch_autosync_settings(patch).await,
            Write::Watch(on) => commands::update_fswatcher_settings(on).await,
            Write::DesignPreview(on) => {
                commands::update_experimental_settings(None, Some(on)).await
            }
            Write::LogLevel(level) => commands::update_log_settings(level).await,
            Write::Publishing(p) => {
                commands::update_publish_settings(
                    p.message_template,
                    p.default_workflow,
                    p.default_metadata,
                )
                .await
            }
            Write::IgnoreRule { rule, on } => commands::set_default_ignore_rule(rule, on).await,
        }
    })
}

fn measure() -> Answer<commands::StorageSize> {
    Box::pin(commands::measure_storage())
}

fn free() -> Answer<FreedSpace> {
    Box::pin(commands::run_gc())
}

fn read_accounts() -> Answer<Vec<AccountHostData>> {
    Box::pin(async { commands::get_main_page_accounts().await.map(|d| d.hosts) })
}

fn settle_account(host: String) -> Answer<AccountHostData> {
    Box::pin(commands::refresh_main_page_account(host))
}

fn sign_out(host: String) -> Answer<String> {
    Box::pin(commands::erase_auth(host))
}

fn edit_rules() -> Answer<()> {
    Box::pin(commands::open_user_ignore_rules())
}

const TAURI: SettingsIo = SettingsIo {
    read: read_settings,
    write: write_setting,
    measure,
    free,
    accounts: read_accounts,
    settle_account,
    sign_out,
    edit_rules,
};

/// What `/settings` renders when *New design preview* is on.
#[component]
pub fn SettingsV2() -> impl IntoView {
    view! { <SettingsScreen io=TAURI /> }
}

/// What `/settings` draws while it reads which page it is, for a v2 reader:
/// the page's own first paint.
#[component]
pub fn SettingsV2Skeleton() -> impl IntoView {
    view! {
        <PageLayout heading="Settings" banner=().into_any() actions=().into_any()>
            {skeleton()}
        </PageLayout>
    }
}

/// The header, then the first card's rows still being read.
fn skeleton() -> AnyView {
    view! {
        <SettingsLayout
            header=view! { <SettingsHeader /> }.into_any()
            syncing=view! {
                <Card title="Syncing" busy=true>
                    <ToggleRowSkeleton />
                    <ToggleRowSkeleton />
                    <ToggleRowSkeleton />
                </Card>
            }
                .into_any()
            publishing=().into_any()
            ignore_rules=().into_any()
            accounts=().into_any()
            app=().into_any()
            storage=().into_any()
            experimental=().into_any()
            troubleshooting=().into_any()
            about=().into_any()
        />
    }
    .into_any()
}

// ── Mapping ──

/// What the body draws as fixed text. The body is rebuilt when this changes,
/// and only then.
#[derive(Clone, Debug, PartialEq)]
struct Fixed {
    version: String,
    os: String,
    home_dir: Option<String>,
    data_dir: String,
    log_env: LogEnv,
    logs_dir: String,
    logs_dir_is_temporary: bool,
    changelog: Vec<ChangelogEntry>,
    ignore_file: String,
    /// The built-in rules, in order; `None` when the file could not be read.
    ignore_rules: Option<Vec<String>>,
}

impl Fixed {
    fn of(d: &SettingsData) -> Self {
        Self {
            version: d.version.clone(),
            os: d.os.clone(),
            home_dir: d.home_dir.clone(),
            data_dir: d.data_dir.clone(),
            log_env: d.log_env.clone(),
            logs_dir: d.logs_dir.clone(),
            logs_dir_is_temporary: d.logs_dir_is_temporary,
            changelog: d.changelog.clone(),
            ignore_file: d.ignore_rules.file.clone(),
            ignore_rules: d
                .ignore_rules
                .rules
                .as_ref()
                .map(|rules| rules.iter().map(|r| r.rule.clone()).collect()),
        }
    }
}

/// An account as the card draws it.
pub(crate) fn account_host(d: AccountHostData) -> AccountHost {
    AccountHost {
        host: d.host,
        role: d.current_role.unwrap_or_default(),
        roles: d.roles,
        signed_out: !d.signed_in,
        provisional: d.provisional,
    }
}

/// *Sign in* from Settings comes back to Settings.
pub(crate) fn sign_in_href(host: &str) -> String {
    format!(
        "/login?host={}&back={}",
        urlencoding::encode(host),
        urlencoding::encode("/settings")
    )
}

/// The mail *Email instead* opens: the version and system in the subject,
/// what the reader wrote, and the zip to attach.
pub(crate) fn report_mailto(version: &str, os: &str, message: &str, zip: &str) -> String {
    let subject = format!("Quilt issue report (v{version}, {os})");
    let what = match message.trim() {
        "" => "Please describe the issue:\n...",
        said => said,
    };
    let body = format!(
        "{what}\n\nDiagnostic logs saved to:\n{zip}\nPlease attach this file to this email."
    );
    format!(
        "mailto:support@quilt.bio?subject={}&body={}",
        urlencoding::encode(&subject),
        urlencoding::encode(&body),
    )
}

// ── The page's state ──

/// Every control's value. The page's, so a re-read keeps it.
#[derive(Clone, Copy)]
struct Fields {
    pull: RwSignal<bool>,
    pull_minutes: RwSignal<String>,
    publish: RwSignal<bool>,
    publish_minutes: RwSignal<String>,
    watch: RwSignal<bool>,
    tray: RwSignal<bool>,
    design_preview: RwSignal<bool>,
    /// One of `LOG_LEVELS`.
    log_level: RwSignal<String>,
    /// The built-in ignore rules turned off.
    rules_off: RwSignal<BTreeSet<String>>,
    /// The Publishing popup's draft, filled from what is stored as it opens.
    template: RwSignal<String>,
    workflow: RwSignal<String>,
    metadata: RwSignal<String>,
}

impl Fields {
    fn new() -> Self {
        Self {
            pull: RwSignal::new(false),
            pull_minutes: RwSignal::new(String::new()),
            publish: RwSignal::new(false),
            publish_minutes: RwSignal::new(String::new()),
            watch: RwSignal::new(false),
            tray: RwSignal::new(false),
            design_preview: RwSignal::new(false),
            log_level: RwSignal::new(String::new()),
            rules_off: RwSignal::new(BTreeSet::new()),
            template: RwSignal::new(String::new()),
            workflow: RwSignal::new(String::new()),
            metadata: RwSignal::new(String::new()),
        }
    }

    fn draft(self) -> PublishSettingsData {
        PublishSettingsData {
            message_template: self.template.get_untracked(),
            default_workflow: self.workflow.get_untracked(),
            default_metadata: self.metadata.get_untracked(),
        }
    }
}

/// What each control last wrote, or was last seeded with: the stored value
/// as far as the page knows. A control whose value equals this has nothing
/// to save.
#[derive(Clone, Copy)]
struct Written {
    pull: StoredValue<bool>,
    pull_secs: StoredValue<u64>,
    publish: StoredValue<bool>,
    publish_secs: StoredValue<u64>,
    watch: StoredValue<bool>,
    tray: StoredValue<bool>,
    design_preview: StoredValue<bool>,
    log_level: StoredValue<String>,
    rules_off: StoredValue<BTreeSet<String>>,
    /// Reactive, because the Publishing card draws it.
    publishing: RwSignal<PublishSettingsData>,
    /// Whether the first read has seeded the fields.
    seeded: StoredValue<bool>,
}

impl Written {
    fn new() -> Self {
        Self {
            pull: StoredValue::new(false),
            pull_secs: StoredValue::new(0),
            publish: StoredValue::new(false),
            publish_secs: StoredValue::new(0),
            watch: StoredValue::new(false),
            tray: StoredValue::new(false),
            design_preview: StoredValue::new(false),
            log_level: StoredValue::new(String::new()),
            rules_off: StoredValue::new(BTreeSet::new()),
            publishing: RwSignal::new(PublishSettingsData::default()),
            seeded: StoredValue::new(false),
        }
    }
}

/// Seed `field` with `stored`, unless it is the same: the control's save
/// sees its value equal to what was written and writes nothing.
fn seed<T: Clone + PartialEq + Send + Sync + 'static>(
    field: RwSignal<T>,
    written: StoredValue<T>,
    stored: T,
) {
    written.set_value(stored.clone());
    if field.with_untracked(|now| *now != stored) {
        field.set(stored);
    }
}

/// Bring the fields to `d`. A minutes field the reader has edited stays as
/// it is, and so does the Publishing draft: the popup fills it as it opens.
fn seed_all(fields: Fields, written: Written, d: &SettingsData) {
    let first = !written.seeded.get_value();
    written.seeded.set_value(true);
    let autosync = &d.autosync;
    seed(fields.pull, written.pull, autosync.pull_enabled);
    seed(fields.publish, written.publish, autosync.push_enabled);
    seed(fields.watch, written.watch, d.fswatcher.enabled);
    seed(fields.tray, written.tray, autosync.close_to_tray);
    seed(
        fields.design_preview,
        written.design_preview,
        d.experimental.main_page_v2,
    );
    seed(
        fields.log_level,
        written.log_level,
        log_level_label(d.log_level.clone()),
    );
    if let Some(rules) = &d.ignore_rules.rules {
        let off = rules
            .iter()
            .filter(|r| !r.on)
            .map(|r| r.rule.clone())
            .collect();
        seed(fields.rules_off, written.rules_off, off);
    }
    for (field, was, stored) in [
        (
            fields.pull_minutes,
            written.pull_secs,
            autosync.pull_interval_secs,
        ),
        (
            fields.publish_minutes,
            written.publish_secs,
            autosync.idle_timeout_secs,
        ),
    ] {
        let untouched = first
            || field.with_untracked(|text| *text == minutes_shown(was.get_value()).to_string());
        was.set_value(stored);
        let shown = minutes_shown(stored).to_string();
        if untouched && field.with_untracked(|text| *text != shown) {
            field.set(shown);
        }
    }
    if written.publishing.with_untracked(|was| *was != d.publish) {
        written.publishing.set(d.publish.clone());
    }
}

/// A toggle: its value, what it last wrote, its row, and its write.
type Toggle = (RwSignal<bool>, StoredValue<bool>, Saved, fn(bool) -> Write);

/// Where a save's answer goes when its caller waits for it, instead of the
/// row's *Saved* or the band.
type Reply = Box<dyn FnOnce(Result<(), String>)>;

/// One save waiting its turn, with the page that asked for it.
struct Queued {
    from: Saves,
    page: PageHandle,
    io: SettingsIo,
    row: Option<Saved>,
    write: Write,
    reply: Option<Reply>,
}

/// What [`Answered`] waits for.
#[derive(Default)]
struct Slot {
    answer: Option<Result<(), String>>,
    waker: Option<std::task::Waker>,
}

/// A save's answer, awaited: the other half of a [`Reply`].
struct Answered(std::rc::Rc<std::cell::RefCell<Slot>>);

impl Future for Answered {
    type Output = Result<(), String>;

    fn poll(
        self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> std::task::Poll<Self::Output> {
        let mut slot = self.0.borrow_mut();
        if let Some(answer) = slot.answer.take() {
            return std::task::Poll::Ready(answer);
        }
        slot.waker = Some(cx.waker().clone());
        std::task::Poll::Pending
    }
}

/// A reply for the queue to call, and the answer it gives.
fn reply() -> (Reply, Answered) {
    let slot = std::rc::Rc::new(std::cell::RefCell::new(Slot::default()));
    let filled = std::rc::Rc::clone(&slot);
    let reply: Reply = Box::new(move |answer| {
        let mut slot = filled.borrow_mut();
        slot.answer = Some(answer);
        if let Some(waker) = slot.waker.take() {
            waker.wake();
        }
    });
    (reply, Answered(slot))
}

thread_local! {
    /// Every settings save, from any visit, in the order it was made. The
    /// app's, not a page's: a save queued before the reader leaves still runs,
    /// and the next visit's saves run after it, so an old one never lands on
    /// top of a newer choice.
    static QUEUE: std::cell::RefCell<VecDeque<Queued>> =
        const { std::cell::RefCell::new(VecDeque::new()) };
    /// Whether a task is running the queue.
    static DRAINING: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
    /// Counts the times the queue has emptied. Every open visit watches it and
    /// reads again, so one opened while an earlier visit's saves ran is not
    /// left with the values from before them. An arc signal: no page owns it.
    static EMPTIED: ArcRwSignal<u64> = ArcRwSignal::new(0);
    /// Tells one visit's [`Saves`] from another's.
    static VISITS: std::cell::Cell<u64> = const { std::cell::Cell::new(0) };
}

/// One visit's side of the queue every save goes on.
#[derive(Clone, Copy)]
struct Saves {
    visit: u64,
    /// The preview's off was saved: time to leave for `/`.
    leave: RwSignal<bool>,
    /// A save of this visit's is out, or waiting.
    writing: RwSignal<bool>,
    /// The row that just saved, for its *Saved*.
    saved: RwSignal<Option<Saved>>,
    /// Rows whose save is out. Their controls are held still meanwhile, so a
    /// setting never has two saves in flight and a failure is never about a
    /// value the reader has since replaced.
    pending: RwSignal<Vec<Saved>>,
    saved_timer: StoredValue<Option<TimeoutHandle>>,
    /// The next re-read is the queue's own, which is not news about accounts.
    quiet: StoredValue<bool>,
}

/// How long *Saved* stays beside a row.
const SAVED_FOR: Duration = Duration::from_millis(2_000);

const SAVE_FAILED: &str = "Could not save this setting.";

/// The outcome band's key: one page, one address.
const HERE: &str = "settings";

impl Saves {
    fn new() -> Self {
        Self {
            visit: VISITS.with(|v| {
                v.set(v.get() + 1);
                v.get()
            }),
            leave: RwSignal::new(false),
            writing: RwSignal::new(false),
            saved: RwSignal::new(None),
            pending: RwSignal::new(Vec::new()),
            saved_timer: StoredValue::new(None),
            quiet: StoredValue::new(false),
        }
    }

    /// Queue `write`, and run the queue unless it is running.
    ///
    /// A new save is now the last command, so an earlier failure in the band
    /// is not news any more, as with [`hold`].
    fn push(
        self,
        row: Option<Saved>,
        write: Write,
        reply: Option<Reply>,
        io: SettingsIo,
        page: PageHandle,
    ) {
        page.outcome.set(None);
        if let Some(row) = row {
            self.pending.update(|rows| rows.push(row));
        }
        QUEUE.with(|q| {
            q.borrow_mut().push_back(Queued {
                from: self,
                page,
                io,
                row,
                write,
                reply,
            });
        });
        self.writing.set(true);
        if DRAINING.with(|d| d.replace(true)) {
            return;
        }
        leptos::task::spawn_local(drain());
    }

    /// One of this visit's saves has run.
    ///
    /// `preview` is the *New design preview* value it wrote, if it wrote one:
    /// the last one saved decides whether the page leaves for `/`. A save
    /// with a `reply` gets its answer there, failure included.
    fn ran(
        self,
        page: PageHandle,
        row: Option<Saved>,
        preview: Option<bool>,
        reply: Option<Reply>,
        result: Result<(), String>,
    ) {
        if let Some(row) = row {
            self.pending.try_update(|rows| {
                if let Some(at) = rows.iter().position(|r| *r == row) {
                    rows.remove(at);
                }
            });
        }
        if let Some(reply) = reply {
            reply(result);
            return;
        }
        match result {
            Ok(()) => {
                if let Some(row) = row {
                    self.say_saved(row);
                }
                if let Some(on) = preview {
                    self.leave.try_set(!on);
                }
            }
            Err(detail) => {
                page.outcome
                    .try_set(Some(critical(HERE.into(), SAVE_FAILED, Some(detail))));
            }
        }
    }

    /// This visit's last save has run. The read after it comes when the
    /// queue empties ([`EMPTIED`]), and only that read may seed the controls,
    /// so mark it out before saying the saves are done.
    fn done(self, page: PageHandle) {
        page.in_flight.try_set(true);
        self.writing.try_set(false);
    }

    fn say_saved(self, row: Saved) {
        if self.saved.try_set(Some(row)).is_some() {
            return;
        }
        if let Some(Some(handle)) = self.saved_timer.try_get_value() {
            handle.clear();
        }
        let saved = self.saved;
        if let Ok(handle) = set_timeout_with_handle(
            move || {
                if saved.try_get_untracked() == Some(Some(row)) {
                    saved.try_set(None);
                }
            },
            SAVED_FOR,
        ) {
            self.saved_timer.try_set_value(Some(handle));
        }
    }
}

/// Run the queue until it is empty. A page's signals are reached with
/// `try_`: the page can be gone, and its saves still run.
async fn drain() {
    while let Some(next) = QUEUE.with(|q| q.borrow_mut().pop_front()) {
        let Queued {
            from,
            page,
            io,
            row,
            write,
            reply,
        } = next;
        let preview = match write {
            Write::DesignPreview(on) => Some(on),
            _ => None,
        };
        let result = (io.write)(write).await;
        from.ran(page, row, preview, reply, result);
        if !QUEUE.with(|q| q.borrow().iter().any(|q| q.from.visit == from.visit)) {
            from.done(page);
        }
    }
    DRAINING.with(|d| d.set(false));
    EMPTIED.with(|e| e.update(|n| *n += 1));
}

/// The accounts, read apart from the settings: asking a role takes the host's
/// credential lock, so the rest of the page does not wait on it.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Accounts {
    Reading,
    Failed,
    Hosts(Vec<AccountHost>),
}

// ── The page ──

#[component]
#[allow(
    clippy::too_many_lines,
    reason = "the page's wiring in one place: the read, the fields, the saves and the commands"
)]
pub(crate) fn SettingsScreen(io: SettingsIo) -> impl IntoView {
    let key = Memo::new(|_| HERE.to_string());
    let page = PageHandle::new();
    let PageHandle {
        busy,
        outcome,
        reload,
        in_flight,
        ..
    } = page;
    let answer = page.read(key, io.read);
    let fixed = Memo::new(move |_| {
        answer.with(|a| match a {
            Some(Ok(d)) => Some(Ok(Fixed::of(d))),
            Some(Err(e)) => Some(Err(e.clone())),
            None => None,
        })
    });

    let fields = Fields::new();
    let written = Written::new();
    let saves = Saves::new();

    // The queue has emptied: read again, after this visit's saves or another
    // visit's. Not on the first run, which is the page's own first read.
    let emptied = EMPTIED.with(Clone::clone);
    Effect::new(move |seen: Option<()>| {
        emptied.track();
        if seen.is_some() && in_flight.try_set(true).is_none() {
            saves.quiet.set_value(true);
            reload.notify();
        }
    });

    // Seed from a read that has landed, and only while no save is out or
    // waiting: see the module's *Writes*.
    Effect::new(move |_| {
        let writing = saves.writing.get();
        let reading = in_flight.get();
        answer.with(|a| {
            if let Some(Ok(d)) = a
                && !writing
                && !reading
            {
                untrack(|| seed_all(fields, written, d));
            }
        });
    });

    // ── Saving as each control changes ──
    let save = move |row: Saved, write: Write| saves.push(Some(row), write, None, io, page);
    let navigate = use_navigate();
    let toggles: [Toggle; 5] = [
        (fields.pull, written.pull, Saved::Pull, |on| {
            Write::Autosync(AutosyncPatch {
                pull_enabled: Some(on),
                ..AutosyncPatch::default()
            })
        }),
        (fields.publish, written.publish, Saved::Publish, |on| {
            Write::Autosync(AutosyncPatch {
                push_enabled: Some(on),
                ..AutosyncPatch::default()
            })
        }),
        (fields.watch, written.watch, Saved::Watch, Write::Watch),
        (fields.tray, written.tray, Saved::Tray, |on| {
            Write::Autosync(AutosyncPatch {
                close_to_tray: Some(on),
                ..AutosyncPatch::default()
            })
        }),
        (
            fields.design_preview,
            written.design_preview,
            Saved::DesignPreview,
            Write::DesignPreview,
        ),
    ];
    for (field, was, row, write) in toggles {
        Effect::new(move |_| {
            let now = field.get();
            if was.get_value() == now {
                return;
            }
            was.set_value(now);
            save(row, write(now));
        });
    }
    Effect::new(move |_| {
        let label = fields.log_level.get();
        if written.log_level.with_value(|was| *was == label) {
            return;
        }
        written.log_level.set_value(label.clone());
        save(Saved::LogLevel, Write::LogLevel(label.to_lowercase()));
    });
    // A box saves the one rule it changed. Its index names the row, for the
    // row's *Saved* and to hold the box while the save is out.
    Effect::new(move |_| {
        let now = fields.rules_off.get();
        let was = written.rules_off.get_value();
        if was == now {
            return;
        }
        written.rules_off.set_value(now.clone());
        let rules = fixed.with_untracked(|f| match f {
            Some(Ok(f)) => f.ignore_rules.clone().unwrap_or_default(),
            _ => Vec::new(),
        });
        for rule in now.symmetric_difference(&was) {
            let Some(at) = rules.iter().position(|r| r == rule) else {
                continue;
            };
            let write = Write::IgnoreRule {
                rule: rule.clone(),
                on: !now.contains(rule),
            };
            save(Saved::IgnoreRule(at), write);
        }
    });
    // Off this page once the preview's off is saved and the queue has run:
    // `/` asks again which design is on, and this page would stay the one it
    // is until the reader moved. A failed save stays here, in the band.
    Effect::new(move |_| {
        if saves.leave.get() && !saves.writing.get() {
            saves.leave.set(false);
            navigate("/", NavigateOptions::default());
        }
    });

    // A minutes field saves on Enter or leaving it, when it is valid and
    // differs from what is stored: a stored 45 s shows 1 and stays 45 s
    // until the reader changes it.
    let minutes = move |field: RwSignal<String>,
                        was: StoredValue<u64>,
                        row: Saved,
                        write: fn(u64) -> Write| {
        Callback::new(move |()| {
            let Some(secs) = field.with_untracked(|text| minutes_secs(text).ok()) else {
                return;
            };
            // The field already shows the stored value rounded: equal to that
            // is no change.
            if secs == was.get_value() || secs == minutes_shown(was.get_value()) * 60 {
                return;
            }
            was.set_value(secs);
            save(row, write(secs));
        })
    };
    let on_pull_minutes = minutes(fields.pull_minutes, written.pull_secs, Saved::Pull, |s| {
        Write::Autosync(AutosyncPatch {
            pull_interval_secs: Some(s),
            ..AutosyncPatch::default()
        })
    });
    let on_publish_minutes = minutes(
        fields.publish_minutes,
        written.publish_secs,
        Saved::Publish,
        |s| {
            Write::Autosync(AutosyncPatch {
                idle_timeout_secs: Some(s),
                ..AutosyncPatch::default()
            })
        },
    );

    // ── Publishing ──
    // The popup and its draft are the page's: a re-read that rebuilds the
    // body leaves them as they are.
    let publishing_open = RwSignal::new(false);
    let fresh = RwSignal::new(0_u64);
    let on_edit = Callback::new(move |()| {
        let stored = written.publishing.get_untracked();
        fields.template.set(stored.message_template);
        fields.workflow.set(stored.default_workflow);
        fields.metadata.set(stored.default_metadata);
        fresh.update(|n| *n += 1);
        publishing_open.set(true);
    });
    // Save waits for its own write on the queue, holding the page's lock.
    // What is sent is what the page takes as stored, so the card shows it at
    // once and the re-read's value replaces it where the backend trims it. A
    // failure puts the old one back, and the popup stays open with the draft.
    let save_publishing = move || {
        let draft = fields.draft();
        async move {
            let before = written.publishing.get_untracked();
            if before == draft {
                return Ok(());
            }
            written.publishing.set(draft.clone());
            let (reply, answer) = reply();
            saves.push(None, Write::Publishing(draft), Some(reply), io, page);
            let answer = hold(busy, outcome, answer).await;
            answer.map_err(|detail| {
                written.publishing.try_set(before);
                format!("{SAVE_FAILED} {detail}")
            })
        }
    };

    // ── Accounts ──
    let accounts = RwSignal::new(Accounts::Reading);
    let accounts_reload = Trigger::new();
    let account_reads = StoredValue::new(0_u64);
    // A re-read of the page reads the accounts too, unless it is a save's
    // own: that is not news about accounts, and asking roles again takes each
    // host's credential lock. Apart from the accounts' own trigger below, so
    // a sign-out's or a role switch's read is never swallowed by a save's
    // quiet one landing in the same moment. Not on the first run: the read
    // below makes the first.
    Effect::new(move |seen: Option<()>| {
        reload.track();
        let quiet = saves.quiet.get_value();
        saves.quiet.set_value(false);
        if seen.is_some() && !quiet {
            accounts_reload.notify();
        }
    });
    Effect::new(move |_| {
        accounts_reload.track();
        account_reads.update_value(|n| *n += 1);
        let this = account_reads.get_value();
        let current = move || account_reads.try_get_value() == Some(this);
        leptos::task::spawn_local(async move {
            let Ok(hosts) = (io.accounts)().await else {
                if current() {
                    accounts.try_set(Accounts::Failed);
                }
                return;
            };
            if !current() {
                return;
            }
            let asking: Vec<String> = hosts
                .iter()
                .filter(|h| h.provisional)
                .map(|h| h.host.clone())
                .collect();
            accounts.try_set(Accounts::Hosts(
                hosts.into_iter().map(account_host).collect(),
            ));
            for host in asking {
                leptos::task::spawn_local(async move {
                    let settled = (io.settle_account)(host.clone()).await;
                    if !current() {
                        return;
                    }
                    let settled = match settled {
                        Ok(d) => account_host(d),
                        // Nothing confirmed the role: the row says it is
                        // unavailable rather than checking for ever.
                        Err(_) => AccountHost {
                            host: host.clone(),
                            role: String::new(),
                            roles: Vec::new(),
                            signed_out: false,
                            provisional: false,
                        },
                    };
                    accounts.try_update(|a| {
                        if let Accounts::Hosts(hosts) = a
                            && let Some(row) = hosts.iter_mut().find(|h| h.host == host)
                        {
                            *row = settled;
                        }
                    });
                });
            }
        });
    });
    let asked_sign_out = RwSignal::new(None::<String>);
    let navigate_sign_in = use_navigate();
    let on_sign_in = Callback::new(move |host: String| {
        navigate_sign_in(&sign_in_href(&host), NavigateOptions::default());
    });
    let on_role = Callback::new(move |(host, role): (String, String)| {
        if busy.get_untracked() {
            accounts_reload.notify();
            return;
        }
        leptos::task::spawn_local(async move {
            if let Err(detail) = hold(busy, outcome, commands::switch_role(host, role)).await {
                outcome.try_set(Some(critical(
                    HERE.into(),
                    "Could not switch the role.",
                    Some(detail),
                )));
            }
            // On a failure too: the read puts the true role back.
            accounts_reload.notify();
        });
    });
    let sign_out = move |host: String| async move {
        hold(busy, outcome, (io.sign_out)(host)).await?;
        accounts_reload.notify();
        Ok(())
    };

    // ── Storage ──
    let size = RwSignal::new(StorageSize::NotMeasured);
    let freeing = RwSignal::new(false);
    let freed = RwSignal::new(None::<String>);
    let measures = StoredValue::new(0_u64);
    let on_measure = Callback::new(move |()| {
        // A measure beside a sweep would count what it is deleting.
        if freeing.get_untracked() {
            return;
        }
        measures.update_value(|n| *n += 1);
        let this = measures.get_value();
        size.set(StorageSize::Measuring);
        freed.set(None);
        leptos::task::spawn_local(async move {
            let answer = (io.measure)().await;
            if measures.try_get_value() != Some(this) {
                return;
            }
            size.try_set(match answer {
                Ok(s) => StorageSize::Measured {
                    total: s.total_bytes,
                    freeable: s.freeable_bytes,
                },
                Err(_) => StorageSize::Failed,
            });
        });
    });
    let on_free = Callback::new(move |()| {
        if busy.get_untracked() {
            return;
        }
        // A measure still out counted what this is about to delete.
        measures.update_value(|n| *n += 1);
        if size.with_untracked(|s| *s == StorageSize::Measuring) {
            size.set(StorageSize::NotMeasured);
        }
        freeing.set(true);
        freed.set(None);
        leptos::task::spawn_local(async move {
            match hold(busy, outcome, (io.free)()).await {
                Ok(done) => {
                    size.try_update(|s| *s = s.clone().after_freeing(done.freed_bytes));
                    freed.try_set(Some(done.message));
                }
                Err(detail) => {
                    // The sweep deletes as it goes, so a failure can come
                    // after some of it: a measured line may be stale now.
                    size.try_update(|s| {
                        if matches!(s, StorageSize::Measured { .. }) {
                            *s = StorageSize::NotMeasured;
                        }
                    });
                    outcome.try_set(Some(critical(
                        HERE.into(),
                        "Could not free up space.",
                        Some(detail),
                    )));
                }
            }
            freeing.try_set(false);
        });
    });

    // ── Report a problem ──
    let report_open = RwSignal::new(false);
    let report_message = RwSignal::new(String::new());
    let report_state = RwSignal::new(ReportState::Writing);
    // A sent report is done with: the next open starts a new one.
    Effect::new(move |_| {
        if report_open.get() && report_state.get_untracked() == ReportState::Sent {
            report_state.set(ReportState::Writing);
            report_message.set(String::new());
        }
    });
    let on_report = Callback::new(move |()| report_open.set(true));
    let on_send = Callback::new(move |()| {
        if report_state.get_untracked() == ReportState::Sending {
            return;
        }
        report_state.set(ReportState::Sending);
        let message = report_message.get_untracked();
        leptos::task::spawn_local(async move {
            let sent = async {
                let zip = commands::collect_diagnostic_logs().await?;
                commands::send_crash_report(zip, Some(message)).await
            };
            report_state.try_set(match sent.await {
                Ok(_) => ReportState::Sent,
                Err(e) => ReportState::Failed(e),
            });
        });
    });
    let on_email = Callback::new(move |()| {
        if report_state.get_untracked() == ReportState::Sending {
            return;
        }
        let (version, os) = fixed.with_untracked(|f| match f {
            Some(Ok(f)) => (f.version.clone(), f.os.clone()),
            _ => (String::new(), String::new()),
        });
        report_state.set(ReportState::Sending);
        let message = report_message.get_untracked();
        leptos::task::spawn_local(async move {
            let opened = async {
                let zip = commands::collect_diagnostic_logs().await?;
                commands::open_in_web_browser(report_mailto(&version, &os, &message, &zip)).await
            };
            report_state.try_set(match opened.await {
                // The mail app has it now; the dialog's work is done.
                Ok(_) => {
                    report_open.try_set(false);
                    ReportState::Writing
                }
                Err(e) => ReportState::Failed(e),
            });
        });
    });

    // ── Folders and release notes ──
    let open_folder = move |open: fn() -> Answer<String>| {
        Callback::new(move |()| {
            // This command is now the last one, so an earlier failure is not news.
            outcome.set(None);
            leptos::task::spawn_local(async move {
                if let Err(detail) = open().await {
                    outcome.try_set(Some(critical(
                        HERE.into(),
                        "Could not open the folder.",
                        Some(detail),
                    )));
                }
            });
        })
    };
    let on_open_home = open_folder(|| Box::pin(commands::open_home_dir()));
    let on_open_logs = open_folder(|| Box::pin(commands::debug_logs()));
    let on_open_data = open_folder(|| Box::pin(commands::open_data_dir()));
    let notes_open = RwSignal::new(false);
    let on_release_notes = Callback::new(move |()| notes_open.set(true));

    // ── Ignore rules ──
    // The file opens in another app. When the reader comes back to the
    // window, the page reads again, so the boxes show what they wrote there.
    let editing_rules = RwSignal::new(false);
    let on_edit_rules = Callback::new(move |()| {
        outcome.set(None);
        leptos::task::spawn_local(async move {
            match (io.edit_rules)().await {
                Ok(()) => {
                    editing_rules.try_set(true);
                }
                Err(detail) => {
                    outcome.try_set(Some(critical(
                        HERE.into(),
                        "Could not open your rules.",
                        Some(detail),
                    )));
                }
            }
        });
    });
    let focus = window_event_listener(leptos::ev::focus, move |_| {
        if editing_rules.get_untracked() && in_flight.try_set(true).is_none() {
            editing_rules.set(false);
            saves.quiet.set_value(true);
            reload.notify();
        }
    });
    on_cleanup(move || focus.remove());

    // ── The body, rebuilt only when the fixed text changes ──
    let just_saved: Signal<Option<Saved>> = saves.saved.into();
    let pending: Signal<Vec<Saved>> = saves.pending.into();
    let body = move |f: Fixed| {
        let accounts_card = move || match accounts.get() {
            Accounts::Reading => view! {
                <Card title="Accounts" busy=true>
                    <HostRowSkeleton />
                </Card>
            }
            .into_any(),
            Accounts::Failed => view! {
                <Card title="Accounts">
                    <LoadFailure
                        words="Could not load your accounts."
                        on_retry=Callback::new(move |()| accounts_reload.notify())
                    />
                </Card>
            }
            .into_any(),
            Accounts::Hosts(hosts) => view! {
                <AccountsCard
                    hosts=hosts
                    on_sign_in=on_sign_in
                    on_role=on_role
                    on_sign_out=Callback::new(move |host| asked_sign_out.set(Some(host)))
                    locked=busy
                />
            }
            .into_any(),
        };
        let home_dir = f.home_dir.clone();
        let storage_card = move || {
            let home_dir = home_dir.clone();
            match freed.get() {
                Some(words) => view! {
                    <StorageCard
                        home_dir=home_dir
                        on_open_home=on_open_home
                        size=size.get()
                        freeing=freeing
                        freed=words
                        on_free=on_free
                        on_measure=on_measure
                    />
                }
                .into_any(),
                None => view! {
                    <StorageCard
                        home_dir=home_dir
                        on_open_home=on_open_home
                        size=size.get()
                        freeing=freeing
                        on_free=on_free
                        on_measure=on_measure
                    />
                }
                .into_any(),
            }
        };
        view! {
            <SettingsLayout
                header=view! { <SettingsHeader /> }.into_any()
                syncing=view! {
                    <SyncingCard
                        pull=fields.pull
                        pull_minutes=fields.pull_minutes
                        publish=fields.publish
                        publish_minutes=fields.publish_minutes
                        watch=fields.watch
                        saved=just_saved
                        pending=pending
                        on_pull_minutes=on_pull_minutes
                        on_publish_minutes=on_publish_minutes
                    />
                }
                    .into_any()
                publishing=view! {
                    <div class=style::slot data-publishing>
                        <PublishingCard stored=written.publishing on_edit=on_edit />
                    </div>
                }
                    .into_any()
                ignore_rules=view! {
                    <div class=style::slot data-ignore-rules>
                        <IgnoreRulesCard
                            rules=f.ignore_rules
                            off=fields.rules_off
                            file=f.ignore_file
                            on_edit=on_edit_rules
                            saved=just_saved
                            pending=pending
                        />
                    </div>
                }
                    .into_any()
                accounts=view! { <div class=style::slot data-accounts>{accounts_card}</div> }.into_any()
                app=view! { <AppCard tray=fields.tray saved=just_saved pending=pending /> }.into_any()
                storage=view! { <div class=style::slot data-storage>{storage_card}</div> }.into_any()
                experimental=view! {
                    <ExperimentalCard
                        design_preview=fields.design_preview
                        saved=just_saved
                        pending=pending
                    />
                }
                    .into_any()
                troubleshooting=view! {
                    <TroubleshootingCard
                        on_report=on_report
                        log_level=fields.log_level
                        log_env=f.log_env
                        logs_dir=f.logs_dir
                        logs_dir_is_temporary=f.logs_dir_is_temporary
                        on_open_logs=on_open_logs
                        data_dir=f.data_dir
                        on_open_data=on_open_data
                        saved=just_saved
                        pending=pending
                    />
                }
                    .into_any()
                about=view! { <AboutCard version=f.version on_release_notes=on_release_notes /> }
                    .into_any()
            />
        }
        .into_any()
    };

    let notes = move || {
        fixed.with(|f| match f {
            Some(Ok(f)) => f.changelog.clone(),
            _ => Vec::new(),
        })
    };

    view! {
        <SignOutDialog host=asked_sign_out sign_out=sign_out running=busy />
        <ReportDialog
            open=report_open
            message=report_message
            state=report_state
            on_email=on_email
            on_send=on_send
        />
        <ReleaseNotes open=notes_open entries=Signal::derive(notes) />
        <PublishingDialog
            open=publishing_open
            template=fields.template
            workflow=fields.workflow
            metadata=fields.metadata
            fresh=fresh
            save=save_publishing
            running=busy
        />
        <V2Page
            heading="Settings"
            page=page
            answer=fixed
            showing=key
            skeleton=skeleton
            bands=|_| ().into_any()
            body=body
            actions=().into_any()
            failure=move || {
                view! {
                    <LoadFailure
                        words="Could not load your settings."
                        on_retry=Callback::new(move |()| reload.notify())
                    />
                }
                .into_any()
            }
        />
    }
}

/// *Release notes*: the changelog's latest entries, newest first.
#[component]
fn ReleaseNotes(open: RwSignal<bool>, entries: Signal<Vec<ChangelogEntry>>) -> impl IntoView {
    view! {
        <Dialog
            open=open
            title="Release notes"
            footer=view! {
                <Button variant=ButtonVariant::Primary on_click=move |_| open.set(false)>
                    "Close"
                </Button>
            }
                .into_any()
        >
            {move || {
                entries
                    .get()
                    .into_iter()
                    .map(|entry| {
                        view! {
                            <section class=style::release>
                                <h3 class=style::field_name>{entry.version}</h3>
                                // An unreleased `-dev` section has no date.
                                {(!entry.date.is_empty())
                                    .then(|| view! { <p class=style::quiet>{entry.date}</p> })}
                                <pre class=style::release_body>{entry.body}</pre>
                            </section>
                        }
                    })
                    .collect_view()
            }}
        </Dialog>
    }
}

/// The page's life, mounted as `/settings` mounts it: under `ByDesign`'s
/// `Suspense`, over stubbed commands.
#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::{
        AutosyncSettingsData, DefaultRuleData, ExperimentalSettingsData, FsWatcherSettingsData,
        IgnoreRulesData,
    };
    use crate::test_support::{button_saying, element_saying, mount, sleep_ms, unmount_earlier};
    use std::cell::{Cell, RefCell};
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    thread_local! {
        /// What each read answers, by read number from 1; the last repeats.
        static ANSWERS: RefCell<Vec<SettingsData>> = const { RefCell::new(Vec::new()) };
        static READS: Cell<usize> = const { Cell::new(0) };
        static WRITES: RefCell<Vec<Write>> = const { RefCell::new(Vec::new()) };
        static WRITE_DELAY: Cell<i32> = const { Cell::new(0) };
        static WRITE_FAILS: Cell<bool> = const { Cell::new(false) };
        static ACCOUNT_READS: Cell<usize> = const { Cell::new(0) };
        static FREE_DELAY: Cell<i32> = const { Cell::new(0) };
        static FREE_FAILS: Cell<bool> = const { Cell::new(false) };
        static MEASURES: Cell<usize> = const { Cell::new(0) };
        static SIGNED_OUT: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
        static RULE_EDITS: Cell<usize> = const { Cell::new(0) };
    }

    fn rule(rule: &str, on: bool) -> DefaultRuleData {
        DefaultRuleData {
            rule: rule.to_string(),
            on,
        }
    }

    fn settings() -> SettingsData {
        SettingsData {
            version: "0.0.1".to_string(),
            home_dir: Some("/home/a-user/QuiltSync".to_string()),
            data_dir: "/data".to_string(),
            auth_hosts: Vec::new(),
            log_level: "default".to_string(),
            log_env: LogEnv::Unset,
            logs_dir: "/logs".to_string(),
            logs_dir_is_temporary: false,
            os: "macos".to_string(),
            changelog: Vec::new(),
            publish: PublishSettingsData::default(),
            autosync: AutosyncSettingsData {
                pull_enabled: false,
                push_enabled: false,
                pull_interval_secs: 45,
                idle_timeout_secs: 300,
                close_to_tray: false,
            },
            fswatcher: FsWatcherSettingsData { enabled: true },
            experimental: ExperimentalSettingsData {
                entire_package_sync: false,
                main_page_v2: true,
            },
            ignore_rules: IgnoreRulesData {
                file: "/data/.quiltignore".to_string(),
                rules: Some(vec![
                    rule(".*/", true),
                    rule(".DS_Store", false),
                    rule("*.pem", true),
                ]),
            },
        }
    }

    fn stub_read(_: String) -> Answer<SettingsData> {
        let n = READS.with(|r| {
            r.set(r.get() + 1);
            r.get()
        });
        let answer = ANSWERS.with(|a| {
            let a = a.borrow();
            a[(n - 1).min(a.len() - 1)].clone()
        });
        Box::pin(async move { Ok(answer) })
    }

    fn stub_write(write: Write) -> Answer<()> {
        let delay = WRITE_DELAY.with(Cell::get);
        let fails = WRITE_FAILS.with(Cell::get);
        Box::pin(async move {
            if delay > 0 {
                sleep_ms(delay).await;
            }
            WRITES.with(|w| w.borrow_mut().push(write));
            if fails {
                Err("disk full".to_string())
            } else {
                Ok(())
            }
        })
    }

    fn stub_measure() -> Answer<commands::StorageSize> {
        MEASURES.with(|m| m.set(m.get() + 1));
        Box::pin(async {
            Ok(commands::StorageSize {
                total_bytes: 3_200_000_000,
                freeable_bytes: 1_100_000_000,
            })
        })
    }

    fn stub_free() -> Answer<FreedSpace> {
        let delay = FREE_DELAY.with(Cell::get);
        let fails = FREE_FAILS.with(Cell::get);
        Box::pin(async move {
            if delay > 0 {
                sleep_ms(delay).await;
            }
            if fails {
                return Err("acme/demo is busy".to_string());
            }
            Ok(FreedSpace {
                message: "Freed 1.1 GB: 3 objects".to_string(),
                freed_bytes: 1_100_000_000,
            })
        })
    }

    fn stub_accounts() -> Answer<Vec<AccountHostData>> {
        ACCOUNT_READS.with(|r| r.set(r.get() + 1));
        Box::pin(async {
            Ok(vec![AccountHostData {
                host: "quilt.test".to_string(),
                signed_in: true,
                current_role: Some("analyst".to_string()),
                roles: vec!["analyst".to_string(), "admin".to_string()],
                provisional: false,
            }])
        })
    }

    fn stub_settle(_: String) -> Answer<AccountHostData> {
        Box::pin(async { Err("not asked".to_string()) })
    }

    fn stub_sign_out(host: String) -> Answer<String> {
        SIGNED_OUT.with(|s| s.borrow_mut().push(host));
        Box::pin(async { Ok("Signed out".to_string()) })
    }

    fn stub_edit_rules() -> Answer<()> {
        RULE_EDITS.with(|e| e.set(e.get() + 1));
        Box::pin(async { Ok(()) })
    }

    const STUBS: SettingsIo = SettingsIo {
        read: stub_read,
        write: stub_write,
        measure: stub_measure,
        free: stub_free,
        accounts: stub_accounts,
        settle_account: stub_settle,
        sign_out: stub_sign_out,
        edit_rules: stub_edit_rules,
    };

    async fn page(answers: Vec<SettingsData>) -> web_sys::Element {
        unmount_earlier();
        ANSWERS.with(|a| *a.borrow_mut() = answers);
        READS.with(|r| r.set(0));
        WRITES.with(|w| w.borrow_mut().clear());
        WRITE_DELAY.with(|d| d.set(0));
        WRITE_FAILS.with(|f| f.set(false));
        ACCOUNT_READS.with(|r| r.set(0));
        FREE_DELAY.with(|d| d.set(0));
        FREE_FAILS.with(|f| f.set(false));
        MEASURES.with(|m| m.set(0));
        SIGNED_OUT.with(|s| s.borrow_mut().clear());
        RULE_EDITS.with(|e| e.set(0));
        at("/settings");
        let el = mount(|| {
            view! {
                <leptos_router::components::Router>
                    <Suspense fallback=|| view! { <p data-fallback>"loading"</p> }>
                        <SettingsScreen io=STUBS />
                    </Suspense>
                </leptos_router::components::Router>
            }
        });
        sleep_ms(50).await;
        el
    }

    /// The page again, as a second visit, with the stubs as they are.
    async fn remount() -> web_sys::Element {
        unmount_earlier();
        let el = mount(|| {
            view! {
                <leptos_router::components::Router>
                    <Suspense fallback=|| view! { <p data-fallback>"loading"</p> }>
                        <SettingsScreen io=STUBS />
                    </Suspense>
                </leptos_router::components::Router>
            }
        });
        sleep_ms(50).await;
        el
    }

    fn at(path: &str) {
        web_sys::window()
            .unwrap()
            .history()
            .unwrap()
            .replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(path))
            .unwrap();
    }

    fn path() -> String {
        web_sys::window().unwrap().location().pathname().unwrap()
    }

    fn alert(el: &web_sys::Element) -> Option<String> {
        el.query_selector("[role=alert]")
            .unwrap()
            .map(|a| a.text_content().unwrap_or_default())
    }

    fn writes() -> Vec<Write> {
        WRITES.with(|w| w.borrow().clone())
    }

    fn reads() -> usize {
        READS.with(Cell::get)
    }

    /// The toggle a reader finds by its label. Not by position: the layout
    /// moves the cards between its one- and two-column orders with the width.
    fn toggle(el: &web_sys::Element, label: &str) -> web_sys::HtmlInputElement {
        let boxes = el.query_selector_all("input[type=checkbox]").unwrap();
        (0..boxes.length())
            .map(|i| boxes.item(i).unwrap().unchecked_into::<web_sys::Element>())
            .find(|b| {
                b.closest("label")
                    .ok()
                    .flatten()
                    .and_then(|l| l.text_content())
                    .is_some_and(|t| t.trim_start().starts_with(label))
            })
            .unwrap_or_else(|| panic!("no toggle {label:?}; markup was {}", el.inner_html()))
            .unchecked_into()
    }

    const PULL: &str = "Get new revisions";
    const WATCH: &str = "Watch folders for changes";
    const PREVIEW: &str = "New design preview";
    const TRAY: &str = "Keep running in the tray";

    fn minutes(el: &web_sys::Element, n: u32) -> web_sys::HtmlInputElement {
        el.query_selector_all("input[type=number]")
            .unwrap()
            .item(n)
            .expect("a minutes field")
            .unchecked_into()
    }

    fn labelled(el: &web_sys::Element, label: &str) -> web_sys::HtmlInputElement {
        let id = element_saying(el, label)
            .get_attribute("for")
            .expect("a label for a control");
        el.query_selector(&format!("#{id}"))
            .unwrap()
            .expect("the control")
            .unchecked_into()
    }

    fn type_into(input: &web_sys::HtmlInputElement, text: &str) {
        input.set_value(text);
        input
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
    }

    fn commit(input: &web_sys::HtmlInputElement) {
        input
            .dispatch_event(&web_sys::Event::new("change").unwrap())
            .unwrap();
    }

    fn type_metadata(el: &web_sys::Element, text: &str) {
        let field: web_sys::HtmlTextAreaElement = el
            .query_selector("textarea")
            .unwrap()
            .expect("the metadata editor's field")
            .unchecked_into();
        field.set_value(text);
        field
            .dispatch_event(&web_sys::Event::new("input").unwrap())
            .unwrap();
    }

    /// The Publishing card's text.
    fn card(el: &web_sys::Element) -> String {
        el.query_selector("[data-publishing]")
            .unwrap()
            .expect("the Publishing card")
            .text_content()
            .unwrap_or_default()
    }

    /// Edit… pressed: the popup, open.
    async fn open_publishing(el: &web_sys::Element) -> web_sys::Element {
        let card = el.query_selector("[data-publishing]").unwrap().unwrap();
        button_saying(&card, "Edit…").click();
        sleep_ms(10).await;
        popup(el).expect("the popup opened")
    }

    fn popup(el: &web_sys::Element) -> Option<web_sys::Element> {
        el.query_selector("dialog[open][aria-label=Publishing]")
            .unwrap()
    }

    fn no_loading_frame(el: &web_sys::Element) {
        assert!(
            el.query_selector("[data-fallback]").unwrap().is_none(),
            "the page went back to the loading frame; markup was {}",
            el.inner_html()
        );
    }

    /// A save's re-read finds news the body draws, so the body is rebuilt;
    /// half-typed minutes and an unsaved Publishing draft survive it, and the
    /// loading frame never comes back.
    /// A rule's box shows the stored state; off and on again each write that
    /// one rule, and the box is held while its save is out.
    #[wasm_bindgen_test]
    async fn a_rule_box_writes_only_its_rule() {
        // Each save's re-read finds what it wrote.
        let mut pem_off = settings();
        pem_off.ignore_rules.rules = Some(vec![
            rule(".*/", true),
            rule(".DS_Store", false),
            rule("*.pem", false),
        ]);
        let mut both = pem_off.clone();
        both.ignore_rules.rules = Some(vec![
            rule(".*/", true),
            rule(".DS_Store", true),
            rule("*.pem", false),
        ]);
        let el = page(vec![settings(), pem_off, both]).await;
        assert!(toggle(&el, ".*/").checked());
        assert!(!toggle(&el, ".DS_Store").checked(), "stored off");

        WRITE_DELAY.with(|d| d.set(40));
        toggle(&el, "*.pem").click();
        sleep_ms(10).await;
        assert!(toggle(&el, "*.pem").disabled(), "held while it saves");
        assert!(!toggle(&el, ".*/").disabled(), "the others are not");
        sleep_ms(80).await;
        assert!(!toggle(&el, "*.pem").disabled());

        WRITE_DELAY.with(|d| d.set(0));
        toggle(&el, ".DS_Store").click();
        sleep_ms(50).await;
        assert_eq!(
            writes(),
            vec![
                Write::IgnoreRule {
                    rule: "*.pem".to_string(),
                    on: false,
                },
                Write::IgnoreRule {
                    rule: ".DS_Store".to_string(),
                    on: true,
                },
            ]
        );
        assert!(!toggle(&el, "*.pem").checked());
        assert!(toggle(&el, ".DS_Store").checked());
    }

    /// A rule turned off elsewhere, such as in the file, shows on the next
    /// read and is not written back.
    #[wasm_bindgen_test]
    async fn a_rule_changed_in_the_file_shows_without_being_written_back() {
        let mut edited = settings();
        edited.ignore_rules.rules = Some(vec![
            rule(".*/", false),
            rule(".DS_Store", false),
            rule("*.pem", true),
        ]);
        let el = page(vec![settings(), edited]).await;

        toggle(&el, WATCH).click();
        sleep_ms(50).await;

        assert!(!toggle(&el, ".*/").checked(), "the stored value shows");
        assert_eq!(writes(), vec![Write::Watch(false)]);
    }

    /// Edit my rules opens the file; coming back to the window reads the
    /// page again, once.
    #[wasm_bindgen_test]
    async fn edit_my_rules_reads_again_on_coming_back() {
        let el = page(vec![settings()]).await;
        let window = web_sys::window().unwrap();
        let focus = || {
            window
                .dispatch_event(&web_sys::Event::new("focus").unwrap())
                .unwrap()
        };
        focus();
        sleep_ms(20).await;
        assert_eq!(reads(), 1, "no read before the file was opened");

        let card = el.query_selector("[data-ignore-rules]").unwrap().unwrap();
        button_saying(&card, "Edit my rules").click();
        sleep_ms(20).await;
        assert_eq!(RULE_EDITS.with(Cell::get), 1);

        focus();
        sleep_ms(50).await;
        assert_eq!(reads(), 2, "coming back read again");
        focus();
        sleep_ms(50).await;
        assert_eq!(reads(), 2, "and only once");
        assert_eq!(ACCOUNT_READS.with(Cell::get), 1, "not news about accounts");
        no_loading_frame(&el);
    }

    /// An unreadable rules file draws no boxes, and Edit my rules stays.
    #[wasm_bindgen_test]
    async fn an_unreadable_rules_file_draws_no_boxes() {
        let mut unreadable = settings();
        unreadable.ignore_rules.rules = None;
        let el = page(vec![unreadable]).await;
        let card = el.query_selector("[data-ignore-rules]").unwrap().unwrap();
        assert!(
            card.query_selector("input[type=checkbox]")
                .unwrap()
                .is_none()
        );
        assert!(
            card.text_content()
                .unwrap_or_default()
                .contains(super::super::IGNORE_RULES_UNREADABLE)
        );
        button_saying(&card, "Edit my rules");
    }

    #[wasm_bindgen_test]
    async fn a_re_read_keeps_what_the_reader_has_not_saved() {
        let mut later = settings();
        later.version = "0.0.2".to_string();
        let el = page(vec![settings(), later]).await;
        type_into(&minutes(&el, 0), "7");
        let dialog = open_publishing(&el).await;
        type_into(&labelled(&dialog, "Message template"), "Plate {date}");

        toggle(&el, WATCH).click();
        sleep_ms(50).await;

        assert_eq!(reads(), 2, "the save read the page again");
        assert!(
            el.text_content().unwrap_or_default().contains("0.0.2"),
            "the body drew the new answer"
        );
        assert_eq!(minutes(&el, 0).value(), "7");
        assert!(popup(&el).is_some(), "the popup is still open");
        assert_eq!(
            labelled(&dialog, "Message template").value(),
            "Plate {date}"
        );
        assert_eq!(
            writes(),
            vec![Write::Watch(false)],
            "and nothing else was saved"
        );
        no_loading_frame(&el);
    }

    /// A value changed elsewhere shows on the next read, and seeding it is
    /// not a save.
    #[wasm_bindgen_test]
    async fn a_stored_change_shows_without_being_written_back() {
        let mut elsewhere = settings();
        elsewhere.autosync.pull_enabled = true;
        let el = page(vec![settings(), elsewhere]).await;
        assert!(!toggle(&el, PULL).checked());

        toggle(&el, WATCH).click();
        sleep_ms(50).await;

        assert!(toggle(&el, PULL).checked(), "the stored value shows");
        assert_eq!(
            writes(),
            vec![Write::Watch(false)],
            "and is not written back"
        );
    }

    /// The first read seeds every control and saves none.
    #[wasm_bindgen_test]
    async fn the_first_read_saves_nothing() {
        let el = page(vec![settings()]).await;
        assert!(toggle(&el, WATCH).checked());
        assert_eq!(minutes(&el, 0).value(), "1", "a stored 45 s shows 1");
        assert_eq!(writes(), Vec::new());
    }

    /// The pull minutes write the focused interval and nothing else; the
    /// stored 45 s is written only once the reader changes the field.
    #[wasm_bindgen_test]
    async fn a_minutes_field_writes_only_its_interval() {
        let el = page(vec![settings()]).await;
        let field = minutes(&el, 0);
        commit(&field);
        sleep_ms(20).await;
        assert_eq!(writes(), Vec::new(), "unchanged, 45 s stays");

        type_into(&field, "3");
        commit(&field);
        sleep_ms(50).await;
        assert_eq!(
            writes(),
            vec![Write::Autosync(AutosyncPatch {
                pull_interval_secs: Some(180),
                ..AutosyncPatch::default()
            })]
        );
        assert!(
            el.text_content().unwrap_or_default().contains("Saved"),
            "the row says it saved"
        );
    }

    /// Quick changes to different settings land in the order they were made.
    #[wasm_bindgen_test]
    async fn saves_land_in_the_order_chosen() {
        let el = page(vec![settings()]).await;
        WRITE_DELAY.with(|d| d.set(30));
        toggle(&el, WATCH).click();
        sleep_ms(5).await;
        toggle(&el, PULL).click();
        sleep_ms(5).await;
        toggle(&el, TRAY).click();
        sleep_ms(150).await;

        assert_eq!(
            writes(),
            vec![
                Write::Watch(false),
                Write::Autosync(AutosyncPatch {
                    pull_enabled: Some(true),
                    ..AutosyncPatch::default()
                }),
                Write::Autosync(AutosyncPatch {
                    close_to_tray: Some(true),
                    ..AutosyncPatch::default()
                }),
            ]
        );
        assert_eq!(reads(), 2, "one re-read, after the queue ran");
    }

    /// A toggle is held while its own save is out, so a second click cannot
    /// queue a second save of the same setting; the others stay free.
    #[wasm_bindgen_test]
    async fn a_toggle_is_held_while_its_save_is_out() {
        let el = page(vec![settings()]).await;
        WRITE_DELAY.with(|d| d.set(40));
        toggle(&el, PULL).click();
        sleep_ms(5).await;
        assert!(toggle(&el, PULL).disabled(), "held");
        assert!(!toggle(&el, WATCH).disabled(), "the others are not");
        toggle(&el, PULL).click();
        sleep_ms(100).await;
        assert!(!toggle(&el, PULL).disabled(), "free again");
        assert_eq!(writes().len(), 1, "one save");
    }

    /// A minutes field is read-only while its save is out, and the cursor
    /// stays in it: Enter does not throw the reader out of the field.
    #[wasm_bindgen_test]
    async fn a_minutes_field_keeps_the_cursor_while_it_saves() {
        let el = page(vec![settings()]).await;
        WRITE_DELAY.with(|d| d.set(40));
        let field = minutes(&el, 0);
        field.focus().unwrap();
        type_into(&field, "3");
        commit(&field);
        sleep_ms(5).await;
        assert!(field.read_only(), "held");
        let focused = web_sys::window()
            .unwrap()
            .document()
            .unwrap()
            .active_element();
        assert_eq!(focused, Some(field.clone().unchecked_into()));
        sleep_ms(100).await;
        assert!(!field.read_only(), "free again");
    }

    /// A save that fails says so, and the re-read puts the stored value back.
    #[wasm_bindgen_test]
    async fn a_failed_save_says_so_and_the_stored_value_returns() {
        let el = page(vec![settings()]).await;
        WRITE_FAILS.with(|f| f.set(true));
        toggle(&el, WATCH).click();
        sleep_ms(50).await;

        let alert = el
            .query_selector("[role=alert]")
            .unwrap()
            .unwrap_or_else(|| panic!("the band; markup was {}", el.inner_html()));
        assert!(
            alert
                .text_content()
                .unwrap_or_default()
                .contains(SAVE_FAILED)
        );
        assert!(toggle(&el, WATCH).checked(), "back to what is stored");
        assert_eq!(writes().len(), 1, "and the seed was not saved");
    }

    /// Check size measures; Free up space then updates the line from its own
    /// report, without measuring again.
    #[wasm_bindgen_test]
    async fn storage_is_measured_when_asked_and_updated_by_freeing() {
        let el = page(vec![settings()]).await;
        let storage = el.query_selector("[data-storage]").unwrap().unwrap();
        button_saying(&storage, "Check size").click();
        sleep_ms(30).await;
        let line = || storage.text_content().unwrap_or_default();
        assert!(
            line().contains("3.2\u{a0}GB · 1.1\u{a0}GB can be freed"),
            "{}",
            line()
        );

        button_saying(&storage, "Free up space").click();
        sleep_ms(30).await;
        assert!(
            line().contains("2.1\u{a0}GB · nothing to free"),
            "{}",
            line()
        );
        assert!(line().contains("Freed 1.1 GB"), "{}", line());
    }

    /// Saves queued when the reader leaves still run, in order.
    #[wasm_bindgen_test]
    async fn saves_queued_when_the_reader_leaves_still_run() {
        let el = page(vec![settings()]).await;
        WRITE_DELAY.with(|d| d.set(40));
        toggle(&el, WATCH).click();
        sleep_ms(5).await;
        toggle(&el, PULL).click();
        sleep_ms(5).await;
        unmount_earlier();
        sleep_ms(150).await;
        assert_eq!(
            writes(),
            vec![
                Write::Watch(false),
                Write::Autosync(AutosyncPatch {
                    pull_enabled: Some(true),
                    ..AutosyncPatch::default()
                }),
            ]
        );
    }

    /// A new visit's save waits for the saves the last visit left queued,
    /// so an old one never lands on top of the newer choice.
    #[wasm_bindgen_test]
    async fn a_new_visit_saves_after_the_last_one() {
        let el = page(vec![settings()]).await;
        WRITE_DELAY.with(|d| d.set(40));
        toggle(&el, PULL).click();
        sleep_ms(5).await;
        toggle(&el, WATCH).click();
        sleep_ms(5).await;

        let again = remount().await;
        toggle(&again, PULL).click();
        sleep_ms(200).await;

        let pull = Write::Autosync(AutosyncPatch {
            pull_enabled: Some(true),
            ..AutosyncPatch::default()
        });
        assert_eq!(writes(), vec![pull.clone(), Write::Watch(false), pull]);
    }

    /// A save that works after one that failed takes the failure down.
    #[wasm_bindgen_test]
    async fn a_retry_takes_the_save_failure_down() {
        let el = page(vec![settings()]).await;
        WRITE_FAILS.with(|f| f.set(true));
        toggle(&el, WATCH).click();
        sleep_ms(50).await;
        assert!(alert(&el).is_some_and(|a| a.contains(SAVE_FAILED)));

        WRITE_FAILS.with(|f| f.set(false));
        toggle(&el, WATCH).click();
        sleep_ms(50).await;
        assert_eq!(alert(&el), None, "markup was {}", el.inner_html());
    }

    /// Turning the preview off leaves for `/` once it is saved.
    #[wasm_bindgen_test]
    async fn turning_the_preview_off_goes_home() {
        let el = page(vec![settings()]).await;
        toggle(&el, PREVIEW).click();
        sleep_ms(50).await;
        assert_eq!(path(), "/");
    }

    /// The backend trims what Publishing saves; the re-read's value replaces
    /// what the card shows, and the next open starts from it.
    #[wasm_bindgen_test]
    async fn publishing_takes_the_stored_value_after_a_save() {
        let mut trimmed = settings();
        trimmed.publish.default_workflow = "release".to_string();
        let el = page(vec![settings(), trimmed]).await;
        let dialog = open_publishing(&el).await;
        type_into(&labelled(&dialog, "Default workflow"), " release ");
        sleep_ms(10).await;
        button_saying(&dialog, "Save").click();
        sleep_ms(60).await;
        assert!(popup(&el).is_none(), "Save closed the popup");
        assert!(card(&el).contains("release"), "{}", card(&el));
        assert!(!card(&el).contains(" release "), "{}", card(&el));
        let dialog = open_publishing(&el).await;
        assert_eq!(labelled(&dialog, "Default workflow").value(), "release");
    }

    /// A failed Publishing save says why in the popup, which stays open with
    /// the draft; the card keeps what is stored.
    #[wasm_bindgen_test]
    async fn a_failed_publishing_save_keeps_the_draft() {
        let el = page(vec![settings()]).await;
        WRITE_FAILS.with(|f| f.set(true));
        let dialog = open_publishing(&el).await;
        type_into(&labelled(&dialog, "Default workflow"), "release");
        sleep_ms(10).await;
        button_saying(&dialog, "Save").click();
        sleep_ms(60).await;
        assert!(popup(&el).is_some(), "still open");
        assert!(
            dialog
                .text_content()
                .unwrap_or_default()
                .contains(&format!("{SAVE_FAILED} disk full")),
            "says what failed, then why: {}",
            dialog.inner_html()
        );
        assert_eq!(labelled(&dialog, "Default workflow").value(), "release");
        assert!(!button_saying(&dialog, "Save").disabled(), "still to save");
        assert!(card(&el).contains("Each bucket's default"), "{}", card(&el));
        let alerts = el.query_selector_all("[role=alert]").unwrap();
        let in_band = (0..alerts.length())
            .map(|i| alerts.item(i).unwrap().unchecked_into::<web_sys::Element>())
            .any(|a| a.closest("dialog").unwrap().is_none());
        assert!(!in_band, "not in the band");
    }

    /// The test handles around Accounts, Storage and Publishing add no box, so each card
    /// is a cell of the layout's grid and stretches to its row like the rest.
    #[wasm_bindgen_test]
    async fn the_test_handles_leave_the_cards_as_grid_cells() {
        let el = page(vec![settings()]).await;
        for hook in ["[data-accounts]", "[data-storage]", "[data-publishing]"] {
            let slot = el.query_selector(hook).unwrap().unwrap();
            assert!(slot.class_list().contains(super::style::slot), "{hook}");
        }
        assert!(
            include_str!("../settings_v2.module.scss").contains(".slot {\n  display: contents;")
        );
    }

    /// Sign out, confirmed, erases that host's credentials through
    /// `erase_auth`, which also drops its cached clients.
    #[wasm_bindgen_test]
    async fn sign_out_erases_the_host() {
        let el = page(vec![settings()]).await;
        let accounts = el.query_selector("[data-accounts]").unwrap().unwrap();
        button_saying(&accounts, "Sign out").click();
        sleep_ms(20).await;
        let dialog = el.query_selector("dialog[open]").unwrap().expect("asked");
        button_saying(&dialog, "Sign out").click();
        sleep_ms(30).await;
        assert_eq!(
            SIGNED_OUT.with(|s| s.borrow().clone()),
            vec!["quilt.test".to_string()]
        );
    }

    /// A visit opened while an earlier visit's saves still run reads again
    /// once they have, without saving anything itself: its first read can
    /// land between them.
    #[wasm_bindgen_test]
    async fn a_new_visit_reads_again_when_the_last_ones_saves_have_run() {
        let mut saved = settings();
        saved.autosync.pull_enabled = true;
        saved.fswatcher.enabled = false;
        // Visit one's read, visit two's read before the saves land, then the
        // read after.
        let el = page(vec![settings(), settings(), saved]).await;
        WRITE_DELAY.with(|d| d.set(40));
        toggle(&el, PULL).click();
        sleep_ms(5).await;
        toggle(&el, WATCH).click();
        sleep_ms(5).await;

        let again = remount().await;
        assert!(!toggle(&again, PULL).checked(), "its first read is stale");
        sleep_ms(200).await;

        assert!(toggle(&again, PULL).checked(), "and the next one is not");
        assert!(!toggle(&again, WATCH).checked());
        assert_eq!(writes().len(), 2, "the visit saved nothing itself");
    }

    /// A sign-out's read of the accounts lands even when a save's quiet
    /// re-read comes in the same moment.
    #[wasm_bindgen_test]
    async fn a_sign_out_is_not_lost_under_a_save() {
        let el = page(vec![settings()]).await;
        assert_eq!(ACCOUNT_READS.with(Cell::get), 1);
        WRITE_DELAY.with(|d| d.set(20));
        toggle(&el, WATCH).click();
        let accounts = el.query_selector("[data-accounts]").unwrap().unwrap();
        button_saying(&accounts, "Sign out").click();
        sleep_ms(5).await;
        let dialog = el.query_selector("dialog[open]").unwrap().expect("asked");
        button_saying(&dialog, "Sign out").click();
        sleep_ms(100).await;
        assert_eq!(writes().len(), 1);
        assert_eq!(
            ACCOUNT_READS.with(Cell::get),
            2,
            "the sign-out's read, and only it"
        );
    }

    /// While a longer command holds the page's lock, the role switcher and
    /// Sign out take no choice, rather than accept one the page would drop.
    #[wasm_bindgen_test]
    async fn the_accounts_wait_for_the_lock() {
        let el = page(vec![settings()]).await;
        FREE_DELAY.with(|d| d.set(60));
        let accounts = el.query_selector("[data-accounts]").unwrap().unwrap();
        let role = || -> web_sys::HtmlSelectElement {
            accounts
                .query_selector("select")
                .unwrap()
                .expect("the switcher")
                .unchecked_into()
        };
        assert!(!role().disabled());
        let storage = el.query_selector("[data-storage]").unwrap().unwrap();
        button_saying(&storage, "Free up space").click();
        sleep_ms(10).await;
        assert!(role().disabled(), "held while freeing");
        assert!(button_saying(&accounts, "Sign out").disabled());
        sleep_ms(100).await;
        assert!(!role().disabled(), "free again");
    }

    /// A sweep that fails can have deleted part of what it counted, so the
    /// measured line goes back to Check size.
    #[wasm_bindgen_test]
    async fn a_failed_sweep_drops_the_measured_line() {
        let el = page(vec![settings()]).await;
        let storage = el.query_selector("[data-storage]").unwrap().unwrap();
        button_saying(&storage, "Check size").click();
        sleep_ms(30).await;
        FREE_FAILS.with(|f| f.set(true));
        button_saying(&storage, "Free up space").click();
        sleep_ms(30).await;
        let line = storage.text_content().unwrap_or_default();
        assert!(!line.contains("can be freed"), "{line}");
        button_saying(&storage, "Check size");
    }

    /// No measure runs beside a sweep: it would count what is being deleted.
    #[wasm_bindgen_test]
    async fn check_size_waits_for_free_up_space() {
        let el = page(vec![settings()]).await;
        FREE_DELAY.with(|d| d.set(60));
        let storage = el.query_selector("[data-storage]").unwrap().unwrap();
        button_saying(&storage, "Free up space").click();
        sleep_ms(10).await;
        let check = button_saying(&storage, "Check size");
        assert!(check.disabled(), "Check size is off while freeing");
        check.click();
        sleep_ms(100).await;
        assert_eq!(MEASURES.with(Cell::get), 0);
    }

    /// A save's own re-read does not ask the accounts again: each ask takes
    /// the host's credential lock.
    #[wasm_bindgen_test]
    async fn a_save_does_not_read_the_accounts_again() {
        let el = page(vec![settings()]).await;
        assert_eq!(ACCOUNT_READS.with(Cell::get), 1);
        toggle(&el, WATCH).click();
        sleep_ms(50).await;
        assert_eq!(reads(), 2);
        assert_eq!(ACCOUNT_READS.with(Cell::get), 1);
    }

    /// The Sign out confirmation is the page's: a re-read that rebuilds the
    /// body leaves it open.
    #[wasm_bindgen_test]
    async fn the_sign_out_confirmation_survives_a_re_read() {
        let mut later = settings();
        later.version = "0.0.2".to_string();
        let el = page(vec![settings(), later]).await;
        let accounts = el.query_selector("[data-accounts]").unwrap().unwrap();
        button_saying(&accounts, "Sign out").click();
        sleep_ms(20).await;
        let open = || el.query_selector_all("dialog[open]").unwrap().length();
        assert_eq!(open(), 1, "the confirmation opened");

        toggle(&el, WATCH).click();
        sleep_ms(50).await;
        assert!(el.text_content().unwrap_or_default().contains("0.0.2"));
        assert_eq!(open(), 1, "and is still open");
    }

    /// The popup opens on what is stored; Save writes the draft, and the card
    /// shows it.
    #[wasm_bindgen_test]
    async fn publishing_saves_the_draft() {
        let mut stored = settings();
        stored.publish.message_template = "Plate {date}".to_string();
        let mut saved = stored.clone();
        saved.publish.default_workflow = "release".to_string();
        let el = page(vec![stored, saved]).await;
        assert!(card(&el).contains("Plate {date}"), "{}", card(&el));
        let dialog = open_publishing(&el).await;
        assert_eq!(
            labelled(&dialog, "Message template").value(),
            "Plate {date}"
        );
        type_into(&labelled(&dialog, "Default workflow"), "release");
        sleep_ms(10).await;
        button_saying(&dialog, "Save").click();
        sleep_ms(50).await;
        assert_eq!(
            writes(),
            vec![Write::Publishing(PublishSettingsData {
                message_template: "Plate {date}".to_string(),
                default_workflow: "release".to_string(),
                ..PublishSettingsData::default()
            })]
        );
        assert!(card(&el).contains("release"), "{}", card(&el));
    }

    /// Cancel drops the draft, a reset one too: nothing is written, and the
    /// next open starts from what is stored.
    #[wasm_bindgen_test]
    async fn cancel_undoes_the_draft_and_a_reset() {
        let mut stored = settings();
        stored.publish.default_workflow = "release".to_string();
        let el = page(vec![stored]).await;
        let dialog = open_publishing(&el).await;
        button_saying(&dialog, "Reset to defaults").click();
        sleep_ms(10).await;
        assert_eq!(labelled(&dialog, "Default workflow").value(), "");
        button_saying(&dialog, "Cancel").click();
        sleep_ms(30).await;
        assert!(popup(&el).is_none());
        assert_eq!(writes(), Vec::new());
        assert!(card(&el).contains("release"), "{}", card(&el));
        let dialog = open_publishing(&el).await;
        assert_eq!(labelled(&dialog, "Default workflow").value(), "release");
    }

    /// Metadata that is not valid JSON says why under the editor, and Save
    /// is off until it is fixed.
    #[wasm_bindgen_test]
    async fn invalid_metadata_blocks_save() {
        let el = page(vec![settings()]).await;
        let dialog = open_publishing(&el).await;
        type_metadata(&dialog, "{\"source\": ");
        sleep_ms(10).await;
        assert!(button_saying(&dialog, "Save").disabled());
        assert!(
            dialog
                .text_content()
                .unwrap_or_default()
                .contains("Not valid JSON: "),
            "{}",
            dialog.inner_html()
        );
        type_metadata(&dialog, "{\"source\": \"desktop\"}");
        sleep_ms(10).await;
        assert!(!button_saying(&dialog, "Save").disabled());
    }

    /// The popup's Save takes the page's lock: the role switcher waits for
    /// it, and while another command holds the lock, Save does.
    #[wasm_bindgen_test]
    async fn publishing_shares_the_pages_lock() {
        let el = page(vec![settings()]).await;
        let accounts = el.query_selector("[data-accounts]").unwrap().unwrap();
        let role = || -> web_sys::HtmlSelectElement {
            accounts
                .query_selector("select")
                .unwrap()
                .expect("the switcher")
                .unchecked_into()
        };
        WRITE_DELAY.with(|d| d.set(60));
        let dialog = open_publishing(&el).await;
        type_into(&labelled(&dialog, "Default workflow"), "release");
        sleep_ms(10).await;
        button_saying(&dialog, "Save").click();
        sleep_ms(10).await;
        assert!(role().disabled(), "held while Publishing saves");
        assert!(
            dialog
                .query_selector("[inert] [role=group]")
                .unwrap()
                .is_some(),
            "and the metadata editor is sealed"
        );
        sleep_ms(120).await;
        assert!(!role().disabled(), "free again");

        FREE_DELAY.with(|d| d.set(60));
        let storage = el.query_selector("[data-storage]").unwrap().unwrap();
        button_saying(&storage, "Free up space").click();
        sleep_ms(10).await;
        let dialog = open_publishing(&el).await;
        assert!(
            button_saying(&dialog, "Save").disabled(),
            "sealed while freeing"
        );
        sleep_ms(100).await;
        assert!(!button_saying(&dialog, "Save").disabled(), "free again");
    }

    #[test]
    fn sign_in_comes_back_to_settings() {
        assert_eq!(
            sign_in_href("quilt.test"),
            "/login?host=quilt.test&back=%2Fsettings"
        );
    }

    #[test]
    fn the_mail_carries_what_the_reader_wrote() {
        let mailto = report_mailto("0.1.0", "macos", "Autosync stopped.", "/tmp/logs.zip");
        let body = mailto.split("&body=").nth(1).unwrap();
        let body = urlencoding::decode(body).unwrap();
        assert!(body.starts_with("Autosync stopped.\n\n"), "{body}");
        assert!(body.contains("/tmp/logs.zip"), "{body}");
        let empty = report_mailto("0.1.0", "macos", "  ", "/tmp/logs.zip");
        assert!(
            urlencoding::decode(&empty)
                .unwrap()
                .contains("Please describe the issue")
        );
    }
}
