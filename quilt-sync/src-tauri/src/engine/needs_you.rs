//! What needs the reader: the few states that keep syncing stopped until they
//! act.
//!
//! One set, so the tray, the OS notices and the window cannot disagree. It is
//! state, kept and sent as the autopull activity is: a watch channel, an event
//! carrying the whole set on every change, and a read for a window that
//! mounts. An entry enters when its cause starts and leaves when it clears.

use std::time::Duration;

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
pub const NEEDS_YOU_EVENT: &str = "needs-you";

/// How long a package's publish keeps failing before it needs the reader. A
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

/// Why it needs the reader.
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

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Entry {
    pub subject: Subject,
    pub cause: Cause,
    /// When the cause started. Kept while its details change.
    pub since: DateTime<Utc>,
}

impl Entry {
    fn is(&self, subject: &Subject, kind: Kind) -> bool {
        self.subject == *subject && self.cause.kind() == kind
    }

    /// The OS notice: titled with what it is about, saying the cause.
    #[must_use]
    pub fn notice(&self) -> (String, String) {
        let title = match &self.subject {
            Subject::Package { namespace } => namespace.to_string(),
            Subject::Host { host } => host.to_string(),
            Subject::App => "QuiltSync".to_string(),
        };
        let body = match &self.cause {
            Cause::Paused { reason } => format!("Syncing stopped: {}.", pause_words(reason)),
            Cause::NoAccess { role: Some(role) } => {
                format!("Syncing stopped: no access as {role}.")
            }
            Cause::NoAccess { role: None } => "Syncing stopped: no access.".to_string(),
            Cause::SignIn { expired: true } => {
                "Sign-in expired. Packages on this catalog are not syncing.".to_string()
            }
            Cause::SignIn { expired: false } => {
                "Signed out. Packages on this catalog are not syncing.".to_string()
            }
            Cause::Credentials => "AWS credentials on this computer aren't usable. \
                 Update ~/.aws or run aws sso login."
                .to_string(),
            Cause::PublishFailing { .. } => {
                "Publishing has kept failing for an hour. It is still retrying.".to_string()
            }
            Cause::Watcher { .. } => {
                "Watching package folders for changes stopped working.".to_string()
            }
        };
        (title, body)
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

/// A pause's reason, as a clause after *Syncing stopped:*.
fn pause_words(reason: &PausedDto) -> String {
    match reason {
        PausedDto::PendingChanges => "this copy has changes that are not published".to_string(),
        PausedDto::PendingCommit => "a commit is waiting to be published".to_string(),
        PausedDto::Diverged => "this copy and the bucket both have new revisions".to_string(),
        PausedDto::PullConflict { files } if files.len() == 1 => "conflicts in 1 file".to_string(),
        PausedDto::PullConflict { files } => format!("conflicts in {} files", files.len()),
        PausedDto::RoleDenied { role: Some(role) } => format!("no access as {role}"),
        PausedDto::RoleDenied { role: None } => "no access".to_string(),
        PausedDto::Other { message } => message.trim_end_matches('.').to_string(),
    }
}

/// The set. Every surface that says something needs the reader reads it.
pub struct NeedsYou {
    tx: watch::Sender<Vec<Entry>>,
}

impl Default for NeedsYou {
    fn default() -> Self {
        Self {
            tx: watch::Sender::new(Vec::new()),
        }
    }
}

impl NeedsYou {
    /// The set now, oldest entry first.
    #[must_use]
    pub fn get(&self) -> Vec<Entry> {
        self.tx.borrow().clone()
    }

    #[must_use]
    pub fn subscribe(&self) -> watch::Receiver<Vec<Entry>> {
        self.tx.subscribe()
    }

    /// `subject` needs the reader for `cause`. An entry already standing for
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
/// mounts later reads it with `get_needs_you` first.
pub fn spawn_forwarder(
    events: EventSink,
    mut rx: watch::Receiver<Vec<Entry>>,
) -> tokio::task::JoinHandle<()> {
    crate::engine::events::spawn(async move {
        events.emit(NEEDS_YOU_EVENT, rx.borrow_and_update().clone());
        while rx.changed().await.is_ok() {
            events.emit(NEEDS_YOU_EVENT, rx.borrow_and_update().clone());
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
        let set = NeedsYou::default();
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
        let set = NeedsYou::default();
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
        let set = NeedsYou::default();
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
                "Syncing stopped: conflicts in 2 files.".to_string()
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
                "Sign-in expired. Packages on this catalog are not syncing.".to_string()
            )
        );
        assert_eq!(
            entry(Subject::App, Cause::Credentials).notice().0,
            "QuiltSync"
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
            })
        );
    }
}
