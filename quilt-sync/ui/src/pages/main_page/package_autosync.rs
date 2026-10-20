//! Each package's autosync state, added up on the Autosync card's publish toggle.
//!
//! The toggle's trailing slot stacks one mark per package with changes: the
//! package's name as a hint, then its ring, a spinner, or a `Retrying` or `Paused`
//! label. No clock times: a ring says roughly how far along the wait is, and
//! promises no minute. The card says `nothing to publish` only when no package has
//! changes, and a paused package sits in the stack beside the others.
//!
//! Only the gallery draws this so far. The watcher payload still carries one
//! deadline per toggle, and [`super::autosync`] draws the card from that.

use leptos::prelude::*;

use crate::commands::PausedReasonData;
use crate::kit::Countdown;
use crate::kit::Spinner;
use crate::kit::StateLabel;
use crate::kit::StateTone;
use crate::kit::countdown::EpochMillis;

stylance::import_crate_style!(style, "src/pages/main_page/package_autosync.module.scss");

/// The most marks the stack draws; the rest are counted under it.
const STACK_MAX: usize = 3;

/// One package's autosync state.
#[derive(Clone, Debug, PartialEq)]
pub enum PackageAutosync {
    NoChanges,
    Waiting {
        publish_at: EpochMillis,
        /// A changed file is known to have changed after the app last saw this
        /// package, so it changed while the app was closed and the wait started at
        /// launch. Only that proof sets it: a package with no saved time, a file
        /// whose change time can't be read, or a commit made outside the app also
        /// waits from launch, but without it. The card's startup line counts only
        /// these, so it may leave a package out and never claims one it can't show.
        changed_while_closed: bool,
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

/// What one package's mark in the stack draws.
#[derive(Clone, Debug, PartialEq)]
pub enum Mark {
    Ring {
        publish_at: EpochMillis,
    },
    Publishing,
    /// The hover hint says what failed.
    Retrying {
        hint: String,
    },
    /// The hover hint says why, and what fixes it.
    Paused {
        hint: String,
    },
}

impl Mark {
    /// Publishing first, then the waits in the order they end, then the troubles.
    fn order(&self) -> (u8, f64) {
        match self {
            Mark::Publishing => (0, 0.0),
            Mark::Ring { publish_at } => (1, *publish_at),
            Mark::Retrying { .. } => (2, 0.0),
            Mark::Paused { .. } => (3, 0.0),
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct StackItem {
    pub namespace: String,
    pub mark: Mark,
}

/// The publish toggle, over every package.
#[derive(Clone, Debug, PartialEq)]
pub enum PublishCard {
    Off,
    NothingToPublish,
    Stack {
        items: Vec<StackItem>,
        /// Packages with changes past [`STACK_MAX`].
        more: usize,
        /// Waiting packages first seen at launch.
        first_seen: usize,
    },
}

/// Why a package is paused and what fixes it. An engine message is kept as is,
/// since it names its own fix if it has one.
fn pause_hint(reason: &PausedReasonData) -> String {
    match reason {
        PausedReasonData::PendingChanges => {
            "Paused: local changes stop the pull. Publish them.".to_string()
        }
        PausedReasonData::PendingCommit => {
            "Paused: a revision is not published. Publish it.".to_string()
        }
        PausedReasonData::Diverged => {
            "Paused: changed in both places. Resolve it on the package page.".to_string()
        }
        PausedReasonData::PullConflict { files } => format!(
            "Paused: conflicts in {}. Publish your version, then resolve.",
            plural(files.len(), "file", "files")
        ),
        PausedReasonData::RoleDenied { role: Some(role) } => {
            format!("Paused: no access as {role}. Change your role in Accounts.")
        }
        PausedReasonData::RoleDenied { role: None } => {
            "Paused: no access with your role. Change your role in Accounts.".to_string()
        }
        PausedReasonData::Other { message } => format!("Paused: {message}"),
        PausedReasonData::Unrecognised => "Paused".to_string(),
    }
}

fn plural(n: usize, one: &str, many: &str) -> String {
    if n == 1 {
        format!("1 {one}")
    } else {
        format!("{n} {many}")
    }
}

impl PublishCard {
    pub fn of(enabled: bool, packages: &[(String, PackageAutosync)]) -> Self {
        if !enabled {
            return Self::Off;
        }
        let mut first_seen = 0;
        let mut items: Vec<StackItem> = packages
            .iter()
            .filter_map(|(namespace, state)| {
                let mark = match state {
                    PackageAutosync::NoChanges | PackageAutosync::Off => return None,
                    PackageAutosync::Waiting {
                        publish_at,
                        changed_while_closed,
                    } => {
                        if *changed_while_closed {
                            first_seen += 1;
                        }
                        Mark::Ring {
                            publish_at: *publish_at,
                        }
                    }
                    PackageAutosync::Publishing => Mark::Publishing,
                    PackageAutosync::Retrying { error, .. } => Mark::Retrying {
                        hint: format!("{error}. Tries again soon."),
                    },
                    PackageAutosync::Paused { reason } => Mark::Paused {
                        hint: pause_hint(reason),
                    },
                };
                Some(StackItem {
                    namespace: namespace.clone(),
                    mark,
                })
            })
            .collect();
        if items.is_empty() {
            return Self::NothingToPublish;
        }
        items.sort_by(|a, b| {
            let (a, b) = (a.mark.order(), b.mark.order());
            a.0.cmp(&b.0).then(a.1.total_cmp(&b.1))
        });
        let more = items.len().saturating_sub(STACK_MAX);
        items.truncate(STACK_MAX);
        Self::Stack {
            items,
            more,
            first_seen,
        }
    }

    /// `2 packages changed while QuiltSync was closed; their wait started at launch.`
    pub fn startup_sentence(&self) -> Option<String> {
        match self {
            Self::Stack { first_seen, .. } if *first_seen > 0 => {
                let (subject, their) = if *first_seen == 1 {
                    ("1 package".to_string(), "its")
                } else {
                    (format!("{first_seen} packages"), "their")
                };
                Some(format!(
                    "{subject} changed while QuiltSync was closed; {their} wait started at launch."
                ))
            }
            _ => None,
        }
    }
}

fn mark_view(mark: &Mark, wait_ms: f64, ring_title: &str) -> AnyView {
    match mark {
        Mark::Ring { publish_at } => view! {
            <Countdown
                deadline=Signal::stored(Some(*publish_at))
                interval=wait_ms
                title=ring_title.to_string()
            />
        }
        .into_any(),
        Mark::Publishing => view! { <Spinner aria_label="Publishing" /> }.into_any(),
        Mark::Retrying { hint } => view! {
            <span title=hint.clone()>
                <StateLabel tone=StateTone::Attention>"Retrying"</StateLabel>
            </span>
        }
        .into_any(),
        Mark::Paused { hint } => view! {
            <span title=hint.clone()>
                <StateLabel tone=StateTone::Attention>"Paused"</StateLabel>
            </span>
        }
        .into_any(),
    }
}

/// The publish toggle's trailing slot: `Off`, `nothing to publish`, or the stack.
pub fn publish_trailing(card: &PublishCard, wait_ms: f64, ring_title: &str) -> AnyView {
    match card {
        PublishCard::Off => view! { "Off" }.into_any(),
        PublishCard::NothingToPublish => view! { "nothing to publish" }.into_any(),
        PublishCard::Stack { items, more, .. } => {
            let marks = items
                .iter()
                .map(|item| {
                    view! {
                        <span class=style::item>
                            <span class=style::name title=item.namespace.clone()>
                                {item.namespace.clone()}
                            </span>
                            {mark_view(&item.mark, wait_ms, ring_title)}
                        </span>
                    }
                })
                .collect_view();
            let more = (*more > 0).then(|| {
                view! { <span class=style::name>{format!("and {more} more")}</span> }
            });
            view! { <span class=style::stack>{marks} {more}</span> }.into_any()
        }
    }
}

/// The line under the publish toggle, when packages were first seen at launch.
pub fn publish_detail(card: &PublishCard) -> Option<AnyView> {
    let sentence = card.startup_sentence()?;
    Some(view! { <p class=style::line>{sentence}</p> }.into_any())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn waiting(at: f64) -> PackageAutosync {
        PackageAutosync::Waiting {
            publish_at: at,
            changed_while_closed: false,
        }
    }

    fn first_seen(at: f64) -> PackageAutosync {
        PackageAutosync::Waiting {
            publish_at: at,
            changed_while_closed: true,
        }
    }

    fn paused(reason: PausedReasonData) -> PackageAutosync {
        PackageAutosync::Paused { reason }
    }

    fn retrying() -> PackageAutosync {
        PackageAutosync::Retrying {
            error: "Could not reach S3".to_string(),
            next_try_at: 0.0,
        }
    }

    fn named(states: Vec<PackageAutosync>) -> Vec<(String, PackageAutosync)> {
        states
            .into_iter()
            .enumerate()
            .map(|(i, state)| (format!("user/p{i}"), state))
            .collect()
    }

    fn card(enabled: bool, states: Vec<PackageAutosync>) -> PublishCard {
        PublishCard::of(enabled, &named(states))
    }

    fn names(card: &PublishCard) -> Vec<&str> {
        match card {
            PublishCard::Stack { items, .. } => {
                items.iter().map(|item| item.namespace.as_str()).collect()
            }
            other => panic!("expected a stack, got {other:?}"),
        }
    }

    #[test]
    fn changed_packages_never_read_as_nothing_to_publish() {
        let card = card(true, vec![waiting(5.0), waiting(1.0), waiting(3.0)]);
        assert_eq!(names(&card), ["user/p1", "user/p2", "user/p0"]);
    }

    #[test]
    fn nothing_to_publish_only_when_no_package_has_changes() {
        assert_eq!(
            card(true, vec![PackageAutosync::NoChanges; 3]),
            PublishCard::NothingToPublish
        );
        assert_eq!(card(true, vec![]), PublishCard::NothingToPublish);
    }

    #[test]
    fn the_toggle_off_reads_off_whatever_the_packages_hold() {
        assert_eq!(
            card(false, vec![waiting(1.0), PackageAutosync::Off]),
            PublishCard::Off
        );
        assert_eq!(card(false, vec![first_seen(1.0)]).startup_sentence(), None);
    }

    #[test]
    fn a_paused_package_sits_beside_the_others() {
        let card = card(
            true,
            vec![
                paused(PausedReasonData::Diverged),
                waiting(2.0),
                waiting(1.0),
            ],
        );
        assert_eq!(names(&card), ["user/p2", "user/p1", "user/p0"]);
    }

    #[test]
    fn publishing_leads_and_troubles_trail() {
        let card = card(
            true,
            vec![
                paused(PausedReasonData::Diverged),
                retrying(),
                waiting(1.0),
                PackageAutosync::Publishing,
            ],
        );
        let PublishCard::Stack { items, more, .. } = &card else {
            panic!("expected a stack");
        };
        let order: Vec<u8> = items.iter().map(|item| item.mark.order().0).collect();
        assert_eq!(order, [0, 1, 2]);
        assert_eq!(*more, 1, "the paused one is past the cap and counted");
    }

    #[test]
    fn the_stack_keeps_the_soonest_and_counts_the_rest() {
        let card = card(true, (0..5).map(|i| waiting(f64::from(5 - i))).collect());
        assert_eq!(names(&card), ["user/p4", "user/p3", "user/p2"]);
        let PublishCard::Stack { more, .. } = card else {
            panic!("expected a stack");
        };
        assert_eq!(more, 2);
    }

    #[test]
    fn the_startup_sentence_counts_only_packages_changed_while_closed() {
        let two = card(true, vec![first_seen(9.0), waiting(1.0), first_seen(8.0)]);
        assert_eq!(
            two.startup_sentence().as_deref(),
            Some("2 packages changed while QuiltSync was closed; their wait started at launch.")
        );
        assert_eq!(
            card(true, vec![first_seen(1.0)])
                .startup_sentence()
                .as_deref(),
            Some("1 package changed while QuiltSync was closed; its wait started at launch.")
        );
        assert_eq!(card(true, vec![waiting(1.0)]).startup_sentence(), None);
    }

    #[test]
    fn a_pause_hint_says_why_and_what_fixes_it() {
        assert_eq!(
            pause_hint(&PausedReasonData::PullConflict {
                files: vec!["a.csv".to_string()],
            }),
            "Paused: conflicts in 1 file. Publish your version, then resolve."
        );
        assert_eq!(
            pause_hint(&PausedReasonData::RoleDenied {
                role: Some("analyst".to_string()),
            }),
            "Paused: no access as analyst. Change your role in Accounts."
        );
        assert_eq!(
            pause_hint(&PausedReasonData::Other {
                message: "package does not satisfy the workflow".to_string(),
            }),
            "Paused: package does not satisfy the workflow"
        );
    }
}
