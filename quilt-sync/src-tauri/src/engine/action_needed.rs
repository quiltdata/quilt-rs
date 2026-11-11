//! What needs action: the few states that keep syncing stopped until they
//! act.
//!
//! One set, so the tray, the OS notices and the window cannot disagree. It is
//! state, kept and sent as the autopull activity is: a watch channel, an event
//! carrying the whole set on every change, and a read for a window that
//! mounts. An entry enters when its cause starts and leaves when it clears.

use std::collections::BTreeMap;
use std::sync::Mutex;
use std::time::Duration;
use std::time::Instant;

use chrono::DateTime;
use chrono::Utc;
use quilt_uri::Host;
use quilt_uri::Namespace;
use serde::Serialize;
use tokio::sync::watch;

use crate::engine::autopull::PausedReason;
use crate::engine::commands::PausedDto;
use crate::engine::events::EventSink;
use crate::engine::routes;

/// Event the window follows the set by. Kept in lockstep with the UI.
pub const ACTION_NEEDED_EVENT: &str = "action-needed";

/// How long a package's publish keeps failing before it needs action. A
/// starting value: by then autosync has retried it about once a minute.
pub const PUBLISH_FAILING_AFTER: Duration = Duration::from_hours(1);

/// Who an entry is about.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub enum Subject {
    Package {
        namespace: Namespace,
    },
    /// A catalog: its session is per host, not per package.
    Host {
        host: Host,
    },
    App,
}

impl Subject {
    #[must_use]
    pub fn package(namespace: &Namespace) -> Self {
        Self::Package {
            namespace: namespace.clone(),
        }
    }
}

/// Why it needs action.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum Cause {
    /// Autosync stopped for the package until the reader acts.
    Paused { reason: PausedDto },
    /// Storage refused the package's bucket to the active role. `None` when
    /// the role could not be named.
    NoAccess { role: Option<String> },
    /// The catalog's session expired, or there is none.
    SignIn { expired: bool },
    /// The AWS credentials on this computer are unusable for a package with
    /// no catalog.
    Credentials,
    /// Autosync has kept failing to publish the package.
    PublishFailing { error: String },
    /// The file watcher's subscriber failed.
    Watcher { message: String },
}

/// The kind of a [`Cause`], without its details: an entry is one subject and
/// one kind.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    Paused,
    NoAccess,
    SignIn,
    Credentials,
    PublishFailing,
    Watcher,
}

impl Cause {
    /// A pause, as the set counts it. A denial is refused access, the same
    /// cause a page's read finds, so the two cannot count one package twice.
    #[must_use]
    pub fn of_pause(reason: &PausedReason) -> Self {
        match PausedDto::from(reason) {
            PausedDto::RoleDenied { role } => Self::NoAccess { role },
            reason => Self::Paused { reason },
        }
    }

    #[must_use]
    pub const fn kind(&self) -> Kind {
        match self {
            Self::Paused { .. } => Kind::Paused,
            Self::NoAccess { .. } => Kind::NoAccess,
            Self::SignIn { .. } => Kind::SignIn,
            Self::Credentials => Kind::Credentials,
            Self::PublishFailing { .. } => Kind::PublishFailing,
            Self::Watcher { .. } => Kind::Watcher,
        }
    }
}

/// Sent with the tray menu's line and the page that fixes it, so the window's
/// inbox lists each entry as the tray does.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(into = "Wire")]
pub struct Entry {
    pub subject: Subject,
    pub cause: Cause,
    /// When the cause started. Kept while its details change.
    pub since: DateTime<Utc>,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Wire {
    subject: Subject,
    cause: Cause,
    since: DateTime<Utc>,
    line: String,
    page: String,
}

impl From<Entry> for Wire {
    fn from(entry: Entry) -> Self {
        Self {
            line: entry.menu_line(),
            page: routes::page(entry.page()),
            subject: entry.subject,
            cause: entry.cause,
            since: entry.since,
        }
    }
}

impl Entry {
    fn is(&self, subject: &Subject, kind: Kind) -> bool {
        self.subject == *subject && self.cause.kind() == kind
    }

    /// The OS notice: titled with what it is about, saying the cause in the
    /// same words as the main page's queue (`quilt-ui-kit`'s
    /// `package_state::words`). The backend posts it with the window hidden,
    /// so it can't call the kit: a change to one wording changes both.
    #[must_use]
    pub fn notice(&self) -> (String, String) {
        const STOPPED: &str = "Syncing has stopped.";
        let title = match &self.subject {
            Subject::Package { namespace } => namespace.to_string(),
            Subject::Host { host } => host.to_string(),
            Subject::App => "QuiltSync".to_string(),
        };
        let body = match (&self.subject, &self.cause) {
            (_, Cause::Paused { reason }) => pause_words(reason, STOPPED),
            (_, Cause::NoAccess { role: Some(role) }) => {
                format!("Cannot be read as {role}. {STOPPED}")
            }
            (_, Cause::NoAccess { role: None }) => format!("Cannot be read. {STOPPED}"),
            (_, Cause::SignIn { expired: true }) => {
                "Sign-in expired. Packages on this catalog have stopped syncing.".to_string()
            }
            (Subject::Host { host }, Cause::SignIn { expired: false }) => {
                format!("Signed out of {host}. Packages on this catalog have stopped syncing.")
            }
            (_, Cause::SignIn { expired: false }) => {
                "Signed out. Packages on this catalog have stopped syncing.".to_string()
            }
            (_, Cause::Credentials) => "AWS credentials on this computer aren't usable. \
                 Update ~/.aws or run aws sso login."
                .to_string(),
            (_, Cause::PublishFailing { .. }) => {
                "Publishing has kept failing for an hour. It is still retrying.".to_string()
            }
            (_, Cause::Watcher { .. }) => {
                "Watching package folders for changes stopped working.".to_string()
            }
        };
        (title, body)
    }

    /// The tray menu's line: what it is about, then the notice's first
    /// sentence. An app entry needs no name in its own app's menu.
    #[must_use]
    pub fn menu_line(&self) -> String {
        let (title, body) = self.notice();
        let cause = body
            .split(". ")
            .next()
            .unwrap_or(&body)
            .trim_end_matches('.');
        match self.subject {
            Subject::App => cause.to_string(),
            _ => format!("{title} — {cause}"),
        }
    }

    /// The page that fixes it: the package's page, the main page for a
    /// session or the computer's credentials (its queue offers the remedy),
    /// and Settings for the file watcher.
    #[must_use]
    pub fn page(&self) -> routes::Paths {
        match (&self.subject, &self.cause) {
            (_, Cause::Watcher { .. }) => routes::Paths::Settings,
            (Subject::Package { namespace }, _) => {
                routes::Paths::InstalledPackage(namespace.clone(), routes::EntriesFilter::default())
            }
            _ => routes::Paths::Home,
        }
    }
}

/// A pause, as the queue words the package's state, then `stopped`.
fn pause_words(reason: &PausedDto, stopped: &str) -> String {
    match reason {
        PausedDto::PendingCommit => format!("Has a revision it has not published. {stopped}"),
        PausedDto::Diverged => format!("Changed in both places. {stopped}"),
        PausedDto::PullConflict { files } if files.len() == 1 => {
            format!("Has a conflict in 1 file. {stopped}")
        }
        PausedDto::PullConflict { files } => {
            format!("Has conflicts in {} files. {stopped}", files.len())
        }
        PausedDto::RoleDenied { role: Some(role) } => {
            format!("Cannot be read as {role}. {stopped}")
        }
        PausedDto::RoleDenied { role: None } => format!("Cannot be read. {stopped}"),
        PausedDto::PendingChanges => "Has stopped syncing.".to_string(),
        PausedDto::Other { message } => format!("Has stopped syncing. {message}"),
    }
}

/// The set. Every surface that says something needs action reads it.
pub struct ActionNeeded {
    tx: watch::Sender<Vec<Entry>>,
    /// When each package's current run of failing publishes began. Kept apart
    /// from the retry backoff, which a pass that only waits for the quiet
    /// window resets.
    publish_failing_since: Mutex<BTreeMap<Namespace, Instant>>,
}

impl Default for ActionNeeded {
    fn default() -> Self {
        Self {
            tx: watch::Sender::new(Vec::new()),
            publish_failing_since: Mutex::default(),
        }
    }
}

impl ActionNeeded {
    /// The set now, oldest entry first.
    #[must_use]
    pub fn get(&self) -> Vec<Entry> {
        self.tx.borrow().clone()
    }

    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<Vec<Entry>> {
        self.tx.subscribe()
    }

    /// `subject` needs action for `cause`. An entry already standing for
    /// that kind of cause takes the new details and keeps its time.
    pub fn enter(&self, subject: Subject, cause: Cause) {
        self.tx.send_if_modified(|set| {
            match set.iter_mut().find(|e| e.is(&subject, cause.kind())) {
                Some(entry) if entry.cause == cause => false,
                Some(entry) => {
                    entry.cause = cause;
                    true
                }
                None => {
                    set.push(Entry {
                        subject,
                        cause,
                        since: Utc::now(),
                    });
                    true
                }
            }
        });
    }

    /// The cause of `kind` for `subject` cleared.
    pub fn leave(&self, subject: &Subject, kind: Kind) {
        self.retain(|entry| !entry.is(subject, kind));
    }

    /// A pass with something to publish failed at `now`: how long the
    /// package's publishes have kept failing.
    pub fn publish_failed(&self, namespace: &Namespace, now: Instant) -> Duration {
        let mut since = self
            .publish_failing_since
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        now.saturating_duration_since(*since.entry(namespace.clone()).or_insert(now))
    }

    /// The package published, or has nothing left to: its failing run is
    /// over.
    pub fn publish_settled(&self, namespace: &Namespace) {
        self.publish_failing_since
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .remove(namespace);
        self.leave(&Subject::package(namespace), Kind::PublishFailing);
    }

    /// Keep the failing publish runs of the packages `keep` holds to.
    pub fn keep_publish_runs(&self, keep: impl Fn(&Namespace) -> bool) {
        self.publish_failing_since
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .retain(|namespace, _| keep(namespace));
    }

    /// Publishing was turned off: no publish is failing.
    pub fn publishing_off(&self) {
        self.publish_failing_since
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clear();
        self.forget(Kind::PublishFailing);
    }

    /// Every entry of `kind` no longer applies.
    pub fn forget(&self, kind: Kind) {
        self.retain(|entry| entry.cause.kind() != kind);
    }

    /// Keep only the entries `keep` holds to.
    pub fn retain(&self, keep: impl Fn(&Entry) -> bool) {
        self.tx.send_if_modified(|set| {
            let before = set.len();
            set.retain(|entry| keep(entry));
            set.len() != before
        });
    }

    /// The session or credentials a package is read on worked.
    pub fn session_works(&self, origin: Option<&Host>) {
        match origin {
            Some(host) => self.leave(&Subject::Host { host: host.clone() }, Kind::SignIn),
            None => self.leave(&Subject::App, Kind::Credentials),
        }
    }

    /// The session or credentials a package is read on failed. `expired`
    /// tells a rejected session from a missing one.
    pub fn session_failed(&self, origin: Option<&Host>, expired: bool) {
        match origin {
            Some(host) => self.enter(
                Subject::Host { host: host.clone() },
                Cause::SignIn { expired },
            ),
            None => self.enter(Subject::App, Cause::Credentials),
        }
    }
}

/// Send the set to the window: once now, then on every change. A window that
/// mounts later reads it with `get_action_needed` first.
pub fn spawn_forwarder(
    events: EventSink,
    mut rx: watch::Receiver<Vec<Entry>>,
) -> tokio::task::JoinHandle<()> {
    crate::engine::events::spawn(async move {
        events.emit(ACTION_NEEDED_EVENT, rx.borrow_and_update().clone());
        while rx.changed().await.is_ok() {
            events.emit(ACTION_NEEDED_EVENT, rx.borrow_and_update().clone());
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns(name: &str) -> Namespace {
        Namespace::try_from(name).unwrap()
    }

    fn host(name: &str) -> Host {
        name.parse().unwrap()
    }

    #[test]
    fn one_entry_per_subject_and_kind_keeping_its_time() {
        let set = ActionNeeded::default();
        let mut rx = set.subscribe();
        set.enter(
            Subject::package(&ns("a/b")),
            Cause::of_pause(&PausedReason::Diverged),
        );
        assert!(rx.has_changed().unwrap());
        rx.mark_unchanged();
        let since = set.get()[0].since;

        set.enter(
            Subject::package(&ns("a/b")),
            Cause::of_pause(&PausedReason::Diverged),
        );
        assert!(
            !rx.has_changed().unwrap(),
            "the same cause again is no news"
        );

        set.enter(
            Subject::package(&ns("a/b")),
            Cause::of_pause(&PausedReason::PendingCommit),
        );
        let now = set.get();
        assert_eq!(now.len(), 1, "a new reason is the same pause");
        assert_eq!(now[0].since, since);
        assert_eq!(
            now[0].cause,
            Cause::Paused {
                reason: PausedDto::PendingCommit
            }
        );
    }

    #[test]
    fn a_denial_pause_and_a_denied_read_are_one_entry() {
        let set = ActionNeeded::default();
        set.enter(Subject::package(&ns("a/b")), Cause::NoAccess { role: None });
        set.enter(
            Subject::package(&ns("a/b")),
            Cause::of_pause(&PausedReason::RoleDenied {
                role: "analyst".into(),
            }),
        );
        assert_eq!(
            set.get()
                .iter()
                .map(|e| e.cause.clone())
                .collect::<Vec<_>>(),
            vec![Cause::NoAccess {
                role: Some("analyst".into())
            }]
        );
    }

    #[test]
    fn leaving_takes_only_that_kind_for_that_subject() {
        let set = ActionNeeded::default();
        let pkg = Subject::package(&ns("a/b"));
        set.enter(pkg.clone(), Cause::of_pause(&PausedReason::Diverged));
        set.enter(
            pkg.clone(),
            Cause::PublishFailing {
                error: "offline".into(),
            },
        );
        set.session_failed(Some(&host("quilt.test")), true);
        set.session_failed(None, true);

        set.leave(&pkg, Kind::Paused);
        set.session_works(Some(&host("other.test")));
        set.session_works(None);

        let left: Vec<Kind> = set.get().iter().map(|e| e.cause.kind()).collect();
        assert_eq!(left, vec![Kind::PublishFailing, Kind::SignIn]);
    }

    /// A package pruned and installed again starts a new failing run.
    #[test]
    fn a_pruned_package_starts_a_new_failing_run() {
        let set = ActionNeeded::default();
        let start = Instant::now();
        set.publish_failed(&ns("a/b"), start);
        set.keep_publish_runs(|_| false);
        let later = start + PUBLISH_FAILING_AFTER;
        assert_eq!(set.publish_failed(&ns("a/b"), later), Duration::ZERO);
    }

    #[test]
    fn forgetting_a_kind_drops_it_for_every_subject() {
        let set = ActionNeeded::default();
        let failing = || Cause::PublishFailing {
            error: "offline".into(),
        };
        set.enter(Subject::package(&ns("a/b")), failing());
        set.enter(Subject::package(&ns("c/d")), failing());
        set.enter(
            Subject::package(&ns("a/b")),
            Cause::of_pause(&PausedReason::Diverged),
        );
        set.forget(Kind::PublishFailing);
        let left: Vec<Kind> = set.get().iter().map(|e| e.cause.kind()).collect();
        assert_eq!(left, vec![Kind::Paused]);
    }

    #[test]
    fn a_notice_names_its_subject_and_cause() {
        let entry = |subject, cause| Entry {
            subject,
            cause,
            since: Utc::now(),
        };
        assert_eq!(
            entry(
                Subject::package(&ns("user/plate-07")),
                Cause::of_pause(&PausedReason::PullConflict(vec!["a".into(), "b".into()])),
            )
            .notice(),
            (
                "user/plate-07".to_string(),
                "Has conflicts in 2 files. Syncing has stopped.".to_string()
            )
        );
        assert_eq!(
            entry(
                Subject::Host {
                    host: host("open.quiltdata.com")
                },
                Cause::SignIn { expired: true },
            )
            .notice(),
            (
                "open.quiltdata.com".to_string(),
                "Sign-in expired. Packages on this catalog have stopped syncing.".to_string()
            )
        );
        assert_eq!(
            entry(Subject::App, Cause::Credentials).notice().0,
            "QuiltSync"
        );
    }

    #[test]
    fn a_menu_line_names_its_subject_and_the_cause() {
        let entry = |subject, cause| Entry {
            subject,
            cause,
            since: Utc::now(),
        };
        assert_eq!(
            entry(
                Subject::package(&ns("user/plate-07")),
                Cause::of_pause(&PausedReason::PullConflict(vec!["a".into(), "b".into()])),
            )
            .menu_line(),
            "user/plate-07 — Has conflicts in 2 files"
        );
        assert_eq!(
            entry(
                Subject::Host {
                    host: host("open.quiltdata.com")
                },
                Cause::SignIn { expired: true },
            )
            .menu_line(),
            "open.quiltdata.com — Sign-in expired"
        );
        assert_eq!(
            entry(Subject::App, Cause::Credentials).menu_line(),
            "AWS credentials on this computer aren't usable"
        );
    }

    #[test]
    fn a_click_opens_the_page_that_fixes_it() {
        let entry = |subject, cause| Entry {
            subject,
            cause,
            since: Utc::now(),
        };
        assert_eq!(
            routes::page(
                entry(Subject::package(&ns("a/b")), Cause::NoAccess { role: None }).page()
            ),
            "/installed-package?namespace=a%2Fb"
        );
        assert_eq!(
            routes::page(
                entry(
                    Subject::Host {
                        host: host("quilt.test")
                    },
                    Cause::SignIn { expired: false }
                )
                .page()
            ),
            "/"
        );
        assert_eq!(
            routes::page(entry(Subject::App, Cause::Credentials).page()),
            "/"
        );
        assert_eq!(
            routes::page(
                entry(
                    Subject::App,
                    Cause::Watcher {
                        message: "limit".into()
                    }
                )
                .page()
            ),
            "/settings"
        );
    }

    #[test]
    fn the_wire_form_names_subject_and_cause_by_kind() {
        let entry = Entry {
            subject: Subject::package(&ns("a/b")),
            cause: Cause::of_pause(&PausedReason::PullConflict(vec!["x".into()])),
            since: DateTime::from_timestamp(0, 0).unwrap(),
        };
        assert_eq!(
            serde_json::to_value(&entry).unwrap(),
            serde_json::json!({
                "subject": {"kind": "package", "namespace": "a/b"},
                "cause": {"kind": "paused", "reason": {"kind": "pull_conflict", "files": ["x"]}},
                "since": "1970-01-01T00:00:00Z",
                "line": "a/b — Has a conflict in 1 file",
                "page": "/installed-package?namespace=a%2Fb",
            })
        );
    }
}
