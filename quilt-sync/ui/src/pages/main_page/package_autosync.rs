//! Each package's autosync state, as its row and the Autosync card say it.
//!
//! One state per package; the card adds them up. The card says `nothing to publish`
//! only when no package has changes, and a paused package is counted beside the
//! others rather than hiding them.
//!
//! Only the gallery draws this so far. The watcher payload still carries one
//! deadline per toggle, and [`super::autosync`] draws the card from that.
//!
//! The words take a `clock` that turns a time into `14:20`, so they can be tested
//! on the host, where there is no JavaScript `Date`.

use leptos::prelude::*;

use crate::commands::PausedReasonData;
use crate::kit::AutosyncNote;
use crate::kit::Countdown;
use crate::kit::Spinner;
use crate::kit::StateLabel;
use crate::kit::StateTone;
use crate::kit::countdown::EpochMillis;

stylance::import_crate_style!(style, "src/pages/main_page/package_autosync.module.scss");

/// One package's autosync state.
#[derive(Clone, Debug, PartialEq)]
pub enum PackageAutosync {
    NoChanges,
    Waiting {
        publish_at: EpochMillis,
        /// The app first saw these changes at launch, so they changed while it was
        /// closed and the wait started then.
        first_seen_at_start: bool,
    },
    Publishing,
    Retrying {
        error: String,
        next_try_at: EpochMillis,
    },
    Paused {
        reason: PausedReasonData,
    },
    /// The package has changes and the publish toggle is off.
    Off,
}

/// A row's second line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowNote {
    pub words: String,
    pub tone: StateTone,
    /// Draws a spinner before the words.
    pub busy: bool,
}

/// Why a package is paused and what fixes it. `None` for the fix when the reason is
/// the engine's own message, which names its fix if it has one.
fn pause_words(reason: &PausedReasonData) -> (String, Option<&'static str>) {
    match reason {
        PausedReasonData::PendingChanges => (
            "local changes stop the pull".to_string(),
            Some("publish them"),
        ),
        PausedReasonData::PendingCommit => (
            "a revision is not published".to_string(),
            Some("publish it"),
        ),
        PausedReasonData::Diverged => (
            "changed in both places".to_string(),
            Some("resolve it on the package page"),
        ),
        PausedReasonData::PullConflict { files } => (
            format!("conflicts in {}", plural(files.len(), "file", "files")),
            Some("publish your version, then resolve"),
        ),
        PausedReasonData::RoleDenied { role: Some(role) } => (
            format!("no access as {role}"),
            Some("change your role in Accounts"),
        ),
        PausedReasonData::RoleDenied { role: None } => (
            "no access with your role".to_string(),
            Some("change your role in Accounts"),
        ),
        PausedReasonData::Other { message } => (message.clone(), None),
        PausedReasonData::Unrecognised => {
            ("for a reason this version can't show".to_string(), None)
        }
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

/// What a package row says under its name. `None` for a package with no changes.
pub fn row_note(state: &PackageAutosync, clock: &dyn Fn(EpochMillis) -> String) -> Option<RowNote> {
    let note = |words: String, tone: StateTone, busy: bool| Some(RowNote { words, tone, busy });
    match state {
        PackageAutosync::NoChanges => None,
        PackageAutosync::Waiting {
            publish_at,
            first_seen_at_start: false,
        } => note(
            format!("publishes at {}", clock(*publish_at)),
            StateTone::Neutral,
            false,
        ),
        PackageAutosync::Waiting {
            publish_at,
            first_seen_at_start: true,
        } => note(
            format!(
                "changed while QuiltSync was closed, publishes at {}",
                clock(*publish_at)
            ),
            StateTone::Neutral,
            false,
        ),
        PackageAutosync::Publishing => note("publishing…".to_string(), StateTone::Neutral, true),
        PackageAutosync::Retrying { error, next_try_at } => note(
            format!("{error} · next try at {}", clock(*next_try_at)),
            StateTone::Attention,
            false,
        ),
        PackageAutosync::Paused { reason } => {
            let (why, fix) = pause_words(reason);
            let words = match fix {
                Some(fix) => format!("paused: {why} · {fix}"),
                None => format!("paused: {why}"),
            };
            note(words, StateTone::Attention, false)
        }
        PackageAutosync::Off => note(
            "not published automatically".to_string(),
            StateTone::Neutral,
            false,
        ),
    }
}

/// The publish toggle's totals over every package.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct PublishTotals {
    pub enabled: bool,
    pub waiting: usize,
    /// The earliest publish time among the waiting packages.
    pub next_at: Option<EpochMillis>,
    pub publishing: usize,
    pub retrying: usize,
    pub paused: usize,
    /// Waiting packages first seen at launch, and the earliest of their times.
    pub first_seen: usize,
    pub first_seen_next_at: Option<EpochMillis>,
}

fn earliest(a: Option<EpochMillis>, b: EpochMillis) -> EpochMillis {
    a.map_or(b, |a| a.min(b))
}

impl PublishTotals {
    pub fn of(enabled: bool, states: &[PackageAutosync]) -> Self {
        let mut totals = Self {
            enabled,
            ..Self::default()
        };
        for state in states {
            match state {
                PackageAutosync::NoChanges | PackageAutosync::Off => {}
                PackageAutosync::Waiting {
                    publish_at,
                    first_seen_at_start,
                } => {
                    totals.waiting += 1;
                    totals.next_at = Some(earliest(totals.next_at, *publish_at));
                    if *first_seen_at_start {
                        totals.first_seen += 1;
                        totals.first_seen_next_at =
                            Some(earliest(totals.first_seen_next_at, *publish_at));
                    }
                }
                PackageAutosync::Publishing => totals.publishing += 1,
                PackageAutosync::Retrying { .. } => totals.retrying += 1,
                PackageAutosync::Paused { .. } => totals.paused += 1,
            }
        }
        totals
    }

    fn has_changes(&self) -> bool {
        self.waiting + self.publishing + self.retrying + self.paused > 0
    }

    /// The words in the trailing slot when there is no ring or label to draw.
    pub fn idle_words(&self) -> Option<&'static str> {
        if !self.enabled {
            Some("Off")
        } else if !self.has_changes() {
            Some("nothing to publish")
        } else {
            None
        }
    }

    /// The line under the toggle: `3 waiting, next at 14:20 · 1 paused`.
    pub fn totals_line(&self, clock: &dyn Fn(EpochMillis) -> String) -> Option<String> {
        if self.idle_words().is_some() {
            return None;
        }
        let mut parts = Vec::new();
        if let Some(next_at) = self.next_at {
            parts.push(format!(
                "{} waiting, next at {}",
                self.waiting,
                clock(next_at)
            ));
        }
        if self.publishing > 0 {
            parts.push(format!("publishing {}", self.publishing));
        }
        if self.retrying > 0 {
            parts.push(format!("{} retrying", self.retrying));
        }
        if self.paused > 0 {
            parts.push(format!("{} paused", self.paused));
        }
        Some(parts.join(" · "))
    }

    /// `2 packages changed while QuiltSync was closed. Publishing at 12:00.`
    pub fn startup_sentence(&self, clock: &dyn Fn(EpochMillis) -> String) -> Option<String> {
        if !self.enabled {
            return None;
        }
        let at = self.first_seen_next_at?;
        Some(format!(
            "{} changed while QuiltSync was closed. Publishing at {}.",
            plural(self.first_seen, "package", "packages"),
            clock(at)
        ))
    }
}

/// `14:20`, in local time.
pub fn local_clock(at: EpochMillis) -> String {
    let date = js_sys::Date::new(&at.into());
    format!("{:02}:{:02}", date.get_hours(), date.get_minutes())
}

/// A row's second line as drawn, or `None` when it has nothing to say.
pub fn row_note_view(state: &PackageAutosync) -> Option<AnyView> {
    let RowNote { words, tone, busy } = row_note(state, &local_clock)?;
    Some(view! { <AutosyncNote words=words tone=tone busy=busy /> }.into_any())
}

/// The publish toggle's trailing slot: a spinner while a package publishes, the ring
/// to the next publish time while one waits, then `Retrying` or `Paused`.
pub fn publish_trailing(totals: &PublishTotals, wait_ms: f64, title: String) -> AnyView {
    if let Some(words) = totals.idle_words() {
        return view! { {words} }.into_any();
    }
    if totals.publishing > 0 {
        view! { <Spinner aria_label="Publishing" /> }.into_any()
    } else if let Some(next_at) = totals.next_at {
        view! {
            <Countdown deadline=Signal::stored(Some(next_at)) interval=wait_ms title=title />
        }
        .into_any()
    } else if totals.retrying > 0 {
        view! { <StateLabel tone=StateTone::Attention>"Retrying"</StateLabel> }.into_any()
    } else {
        view! { <StateLabel tone=StateTone::Attention>"Paused"</StateLabel> }.into_any()
    }
}

/// The lines under the publish toggle: the totals, then the startup sentence.
/// `None` when there is neither.
pub fn publish_detail(totals: &PublishTotals) -> Option<AnyView> {
    let lines: Vec<String> = [
        totals.totals_line(&local_clock),
        totals.startup_sentence(&local_clock),
    ]
    .into_iter()
    .flatten()
    .collect();
    if lines.is_empty() {
        return None;
    }
    Some(
        lines
            .into_iter()
            .map(|line| view! { <p class=style::line>{line}</p> })
            .collect_view()
            .into_any(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    const MINUTE: f64 = 60_000.0;

    /// Minutes as `+N`, so a test can read which time won.
    fn clock(at: EpochMillis) -> String {
        format!("+{}", at / MINUTE)
    }

    fn waiting(minutes: f64) -> PackageAutosync {
        PackageAutosync::Waiting {
            publish_at: minutes * MINUTE,
            first_seen_at_start: false,
        }
    }

    fn first_seen(minutes: f64) -> PackageAutosync {
        PackageAutosync::Waiting {
            publish_at: minutes * MINUTE,
            first_seen_at_start: true,
        }
    }

    fn paused() -> PackageAutosync {
        PackageAutosync::Paused {
            reason: PausedReasonData::Diverged,
        }
    }

    fn retrying() -> PackageAutosync {
        PackageAutosync::Retrying {
            error: "could not reach S3".to_string(),
            next_try_at: 5.0 * MINUTE,
        }
    }

    fn line(enabled: bool, states: &[PackageAutosync]) -> Option<String> {
        PublishTotals::of(enabled, states).totals_line(&clock)
    }

    #[test]
    fn changed_packages_never_read_as_nothing_to_publish() {
        let states = vec![waiting(20.0); 5];
        let totals = PublishTotals::of(true, &states);
        assert_eq!(totals.idle_words(), None);
        assert_eq!(
            totals.totals_line(&clock).as_deref(),
            Some("5 waiting, next at +20")
        );
    }

    #[test]
    fn nothing_to_publish_only_when_no_package_has_changes() {
        let totals = PublishTotals::of(true, &vec![PackageAutosync::NoChanges; 3]);
        assert_eq!(totals.idle_words(), Some("nothing to publish"));
        assert_eq!(totals.totals_line(&clock), None);
        assert_eq!(
            PublishTotals::of(true, &[]).idle_words(),
            Some("nothing to publish")
        );
    }

    #[test]
    fn the_toggle_off_reads_off_whatever_the_packages_hold() {
        let states = [PackageAutosync::Off, waiting(3.0), paused()];
        let totals = PublishTotals::of(false, &states);
        assert_eq!(totals.idle_words(), Some("Off"));
        assert_eq!(totals.totals_line(&clock), None);
        assert_eq!(totals.startup_sentence(&clock), None);
    }

    #[test]
    fn the_next_time_is_the_earliest_of_the_waiting() {
        assert_eq!(
            line(true, &[waiting(40.0), waiting(20.0), waiting(30.0)]).as_deref(),
            Some("3 waiting, next at +20")
        );
    }

    #[test]
    fn a_paused_package_is_counted_beside_the_others() {
        assert_eq!(
            line(true, &[waiting(20.0), paused(), waiting(30.0)]).as_deref(),
            Some("2 waiting, next at +20 · 1 paused")
        );
    }

    #[test]
    fn every_kind_is_counted_in_one_order() {
        let states = [
            paused(),
            retrying(),
            PackageAutosync::Publishing,
            waiting(20.0),
            PackageAutosync::NoChanges,
        ];
        assert_eq!(
            line(true, &states).as_deref(),
            Some("1 waiting, next at +20 · publishing 1 · 1 retrying · 1 paused")
        );
    }

    #[test]
    fn only_retrying_still_has_a_line() {
        assert_eq!(
            line(true, &[retrying(), retrying()]).as_deref(),
            Some("2 retrying")
        );
    }

    #[test]
    fn the_startup_sentence_counts_only_packages_first_seen_at_start() {
        let states = [first_seen(120.0), waiting(10.0), first_seen(119.0)];
        let totals = PublishTotals::of(true, &states);
        assert_eq!(
            totals.startup_sentence(&clock).as_deref(),
            Some("2 packages changed while QuiltSync was closed. Publishing at +119.")
        );
        assert_eq!(
            totals.totals_line(&clock).as_deref(),
            Some("3 waiting, next at +10")
        );
    }

    #[test]
    fn one_package_first_seen_is_singular() {
        let totals = PublishTotals::of(true, &[first_seen(60.0)]);
        assert_eq!(
            totals.startup_sentence(&clock).as_deref(),
            Some("1 package changed while QuiltSync was closed. Publishing at +60.")
        );
    }

    #[test]
    fn no_startup_sentence_without_packages_first_seen() {
        let totals = PublishTotals::of(true, &[waiting(5.0)]);
        assert_eq!(totals.startup_sentence(&clock), None);
    }

    fn words(state: &PackageAutosync) -> Option<String> {
        row_note(state, &clock).map(|note| note.words)
    }

    #[test]
    fn each_row_state_has_its_words() {
        assert_eq!(words(&PackageAutosync::NoChanges), None);
        assert_eq!(words(&waiting(20.0)).as_deref(), Some("publishes at +20"));
        assert_eq!(
            words(&first_seen(120.0)).as_deref(),
            Some("changed while QuiltSync was closed, publishes at +120")
        );
        assert_eq!(
            words(&PackageAutosync::Publishing).as_deref(),
            Some("publishing…")
        );
        assert_eq!(
            words(&retrying()).as_deref(),
            Some("could not reach S3 · next try at +5")
        );
        assert_eq!(
            words(&PackageAutosync::Off).as_deref(),
            Some("not published automatically")
        );
    }

    #[test]
    fn only_publishing_is_busy_and_only_trouble_is_attention() {
        let note = |state: &PackageAutosync| row_note(state, &clock).unwrap();
        assert!(note(&PackageAutosync::Publishing).busy);
        assert!(!note(&waiting(1.0)).busy);
        assert_eq!(note(&retrying()).tone, StateTone::Attention);
        assert_eq!(note(&paused()).tone, StateTone::Attention);
        assert_eq!(note(&waiting(1.0)).tone, StateTone::Neutral);
        assert_eq!(note(&PackageAutosync::Off).tone, StateTone::Neutral);
    }

    #[test]
    fn a_pause_says_why_and_what_fixes_it() {
        let paused_by = |reason| words(&PackageAutosync::Paused { reason }).unwrap();
        assert_eq!(
            paused_by(PausedReasonData::PullConflict {
                files: vec!["a.csv".to_string(), "b.csv".to_string()],
            }),
            "paused: conflicts in 2 files · publish your version, then resolve"
        );
        assert_eq!(
            paused_by(PausedReasonData::PullConflict {
                files: vec!["a.csv".to_string()],
            }),
            "paused: conflicts in 1 file · publish your version, then resolve"
        );
        assert_eq!(
            paused_by(PausedReasonData::RoleDenied {
                role: Some("analyst".to_string()),
            }),
            "paused: no access as analyst · change your role in Accounts"
        );
    }

    #[test]
    fn an_engine_message_is_shown_as_is_with_no_fix_added() {
        let message = "package does not satisfy the workflow".to_string();
        assert_eq!(
            words(&PackageAutosync::Paused {
                reason: PausedReasonData::Other { message },
            })
            .as_deref(),
            Some("paused: package does not satisfy the workflow")
        );
    }
}
