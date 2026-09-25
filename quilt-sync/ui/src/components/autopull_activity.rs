//! Autopull's side of the appbar's activity line: while the tick pulls or
//! publishes a package, the line says so.
//!
//! It is the line's only producer today, and what it reports is autopull's
//! transfers and nothing else. Mounted once in `App`, beside the notification
//! stack, and drawing nothing itself: it writes the [`kit::Activities`] that
//! `App` provides, and the kit's [`ActivityLine`](kit::ActivityLine) draws them.
//!
//! # State, not deltas
//!
//! The backend sends the whole of autopull's activity on every change — the
//! one transfer running, or `None` — never a start and a stop. So each payload
//! replaces the line's list outright, and a missed update costs nothing: the
//! next one says what is true.
//!
//! # Not the notification stack
//!
//! [`ToastStack`](super::ToastStack) says what *happened* and keeps it until
//! dismissed; this says what is running and keeps nothing. They share a
//! mount-then-listen shape and no names and no store, so neither can outlive or
//! overwrite the other.

use leptos::prelude::*;
use leptos::task::spawn_local;

use crate::commands;
use crate::commands::ActivityOp;
use crate::kit;
use crate::kit::Activity;
use crate::kit::ActivityKind;
use crate::tauri as tauri_bridge;

/// The words for autopull's activity: one [`Activity`] while a transfer runs,
/// none when it is idle.
pub fn activities_for(activity: Option<&commands::AutopullActivity>) -> Vec<Activity> {
    activity
        .map(|running| {
            let label = match running.op {
                ActivityOp::Pull => format!("Getting latest for {}\u{2026}", running.namespace),
                ActivityOp::Publish => format!("Publishing {}\u{2026}", running.namespace),
            };
            Activity {
                kind: ActivityKind::Autopull,
                label,
            }
        })
        .into_iter()
        .collect()
}

/// The line's list, plus the stamp that stops a slow hydration read from
/// overwriting a newer event — as the notification stack's store does.
///
/// Every event and every issued read takes a fresh stamp, and a read's answer is
/// adopted only while its own stamp is still the newest.
#[derive(Clone, Copy)]
struct Feed {
    activities: kit::Activities,
    stamp: StoredValue<u64>,
}

impl Feed {
    fn new(activities: kit::Activities) -> Self {
        Self {
            activities,
            stamp: StoredValue::new(0),
        }
    }

    /// Take the next stamp, invalidating every read in flight.
    fn bump(self) -> u64 {
        self.stamp
            .try_update_value(|s| {
                *s += 1;
                *s
            })
            .unwrap_or_default()
    }

    /// Adopt a read's answer only if nothing has arrived since it was asked for.
    fn apply_if_current(self, stamp: u64, activity: Option<&commands::AutopullActivity>) {
        if self.stamp.try_get_value().unwrap_or_default() == stamp {
            self.activities.set(activities_for(activity));
        }
    }

    /// An event is the newest word: adopt it at once.
    fn follow(self, activity: Option<&commands::AutopullActivity>) {
        self.bump();
        self.activities.set(activities_for(activity));
    }

    /// Read the backend's activity and adopt it if it is still the newest word.
    async fn hydrate(self) {
        let stamp = self.bump();
        if let Ok(activity) = commands::get_autopull_activity().await {
            self.apply_if_current(stamp, activity.as_ref());
        }
    }
}

/// Feeds the activity line from autopull. Draws nothing; without
/// [`kit::Activities`] above it, it does nothing either.
#[component]
pub fn AutopullActivityFeed() -> impl IntoView {
    let Some(activities) = use_context::<kit::Activities>() else {
        return;
    };
    let feed = Feed::new(activities);

    // Whatever autopull was already doing when the window opened.
    spawn_local(feed.hydrate());

    // The read after registration closes the startup race: any change after it
    // arrives as an event, which bumps the stamp.
    let listener = tauri_bridge::listen_then::<Option<commands::AutopullActivity>>(
        commands::AUTOPULL_ACTIVITY_EVENT,
        move |activity| feed.follow(activity.as_ref()),
        move || spawn_local(feed.hydrate()),
    );
    on_cleanup(move || drop(listener));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::AutopullActivity;

    fn running(op: ActivityOp) -> AutopullActivity {
        AutopullActivity {
            op,
            namespace: ("team", "pkg").into(),
        }
    }

    fn autopull(label: &str) -> Activity {
        Activity {
            kind: ActivityKind::Autopull,
            label: label.to_owned(),
        }
    }

    #[test]
    fn a_pull_reads_getting_latest_for_the_package() {
        assert_eq!(
            activities_for(Some(&running(ActivityOp::Pull))),
            vec![autopull("Getting latest for team/pkg\u{2026}")]
        );
    }

    #[test]
    fn a_publish_reads_publishing_the_package() {
        assert_eq!(
            activities_for(Some(&running(ActivityOp::Publish))),
            vec![autopull("Publishing team/pkg\u{2026}")]
        );
    }

    #[test]
    fn nothing_running_is_no_activity() {
        assert_eq!(activities_for(None), Vec::new());
    }

    mod feed {
        use super::*;
        use wasm_bindgen_test::*;

        fn shown(feed: Feed) -> Vec<Activity> {
            untrack(|| feed.activities.get())
        }

        /// A hydration read issued before an event and landing after it carries
        /// an answer the backend gave before that event, so it must not win.
        #[wasm_bindgen_test]
        fn a_hydration_older_than_an_event_is_dropped() {
            let feed = Feed::new(kit::Activities::new());
            let stamp = feed.bump(); // a hydration read is issued

            // Autopull starts publishing while it is in flight.
            feed.follow(Some(&running(ActivityOp::Publish)));

            // The read lands, saying autopull was idle when it was asked.
            feed.apply_if_current(stamp, None);
            assert_eq!(shown(feed), vec![autopull("Publishing team/pkg\u{2026}")]);
        }

        #[wasm_bindgen_test]
        fn a_hydration_that_is_still_the_newest_word_is_adopted() {
            let feed = Feed::new(kit::Activities::new());
            let stamp = feed.bump();
            feed.apply_if_current(stamp, Some(&running(ActivityOp::Pull)));
            assert_eq!(
                shown(feed),
                vec![autopull("Getting latest for team/pkg\u{2026}")]
            );
        }
    }
}
