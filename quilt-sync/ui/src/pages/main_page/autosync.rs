//! The Autosync card: the state strip's first region on real data.
//!
//! # Nothing here is a reactive prop
//!
//! The body is rendered inside its resource's `Transition`, so a refetch rebuilds
//! it with plain values — the same shape `MainPage` already uses for
//! `PackageList`. That is deliberate rather than lazy. `ToggleRow.trailing` is an
//! `Option<AnyView>` evaluated once, and threading a signal into it would run
//! into Leptos's `#[component]` wrapping its body in `untrack_with_diagnostics`,
//! so a signal read deferred into a nested component is untracked. Rebuilding
//! also re-seeds the ring's CSS animation from the new deadline, which is exactly
//! what should happen when a new deadline arrives.
//!
//! # The words are the caller's
//!
//! §2: the wire carries a discriminator and the vocabulary lives in the UI.
//! `Countdown` renders nothing for an absent deadline *so that* the caller
//! supplies its own idle text — `nothing to publish` belongs to the publish
//! toggle, not to a clock.

use leptos::prelude::*;

use crate::commands;
use crate::commands::MainPageWatcherData;
use crate::commands::ToggleActivityData;
use crate::commands::ToggleStateData;
use crate::kit::Card;
use crate::kit::Countdown;
use crate::kit::StateLabel;
use crate::kit::StateTone;
use crate::kit::ToggleRow;

/// An interval in the shortest honest form: `30s`, `5 min`.
///
/// §1 — derived from the value, never sent as words. Whole minutes read as
/// minutes because that is how the quiet window is set; anything else reads as
/// seconds rather than inventing `1 min 30s`.
fn human_interval(ms: f64) -> String {
    let secs = ms / 1000.0;
    if secs >= 60.0 && (secs % 60.0).abs() < f64::EPSILON {
        format!("{} min", secs / 60.0)
    } else {
        format!("{secs}s")
    }
}

/// The trailing slot: a ring while the machinery is counting down, the caller's
/// idle words while it has nothing to do, and `Paused` when it stopped.
///
/// `Attention`, not `Danger`: a pause is a state the user can clear, and every
/// reason's fix is already some queue row's action.
fn trailing(
    toggle: &ToggleStateData,
    aria_label: String,
    idle: &'static str,
    repeat: bool,
) -> AnyView {
    match toggle.activity {
        ToggleActivityData::Armed => view! {
            <Countdown
                deadline=toggle.deadline
                interval=toggle.interval_ms
                aria_label=aria_label
                repeat=repeat
            />
        }
        .into_any(),
        ToggleActivityData::Idle => view! { {idle} }.into_any(),
        ToggleActivityData::Paused => {
            view! { <StateLabel tone=StateTone::Attention>"Paused"</StateLabel> }.into_any()
        }
    }
}

/// The card, on one payload. Split from [`AutosyncCard`] so it can be tested
/// without a Tauri host.
#[component]
fn AutosyncBody(data: MainPageWatcherData) -> impl IntoView {
    // The SETTING, which stays true while paused. Local signals because
    // `ToggleRow` writes them on change; the payload is the source of truth and
    // a refetch rebuilds this component with it.
    let pull_checked = RwSignal::new(data.pull.enabled);
    let publish_checked = RwSignal::new(data.publish.enabled);
    let pull_every = human_interval(data.pull.interval_ms);
    let publish_after = human_interval(data.publish.interval_ms);

    view! {
        <Card title="Autosync">
            <ToggleRow
                label="Get new revisions"
                sublabel=format!("Every {pull_every}, when nothing is changed here")
                checked=pull_checked
                trailing=trailing(
                    &data.pull,
                    format!("Checks for new revisions every {pull_every}"),
                    "not checking",
                    true,
                )
            />
            <ToggleRow
                label="Publish your changes"
                sublabel=format!("{publish_after} after your last edit")
                checked=publish_checked
                trailing=trailing(
                    &data.publish,
                    format!("Publishes {publish_after} after your last edit"),
                    "nothing to publish",
                    false,
                )
            />
        </Card>
    }
}

/// The card and its payload.
///
/// **No skeleton and no fallback content.** §6: chrome is never skeletonised,
/// only the queue and the two lists — and this payload is a memory read (three
/// `RwLock`s the watcher already holds), so the pending window is shorter than a
/// frame. A failed fetch renders nothing and logs: the only way it can fail is a
/// missing bridge or an unregistered command, and asserting anything about
/// autosync on the strength of a failed read would be the manufactured state
/// plan 2's final review removed from the row path.
#[component]
pub fn AutosyncCard() -> impl IntoView {
    let watcher = LocalResource::new(|| async move { commands::get_main_page_watcher().await });

    view! {
        <Transition fallback=|| ()>
            {move || Suspend::new(async move {
                match watcher.await {
                    Ok(data) => view! { <AutosyncBody data=data /> }.into_any(),
                    Err(err) => {
                        web_sys::console::error_1(
                            &format!("get_main_page_watcher failed: {err}").into(),
                        );
                        ().into_any()
                    }
                }
            })}
        </Transition>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen_test::*;

    fn mount<N: IntoView + 'static>(f: impl FnOnce() -> N + 'static) -> web_sys::Element {
        let doc = web_sys::window().unwrap().document().unwrap();
        let container: web_sys::HtmlElement =
            doc.create_element("div").unwrap().dyn_into().unwrap();
        doc.body().unwrap().append_child(&container).unwrap();
        leptos::mount::mount_to(container.clone(), f).forget();
        container.into()
    }

    /// Pull counting down, publish with nothing to do — the ordinary steady state.
    fn armed_payload() -> MainPageWatcherData {
        MainPageWatcherData {
            pull: ToggleStateData {
                enabled: true,
                activity: ToggleActivityData::Armed,
                deadline: Some(js_sys::Date::now() + 23_000.0),
                interval_ms: 30_000.0,
            },
            publish: ToggleStateData {
                enabled: true,
                activity: ToggleActivityData::Idle,
                deadline: None,
                interval_ms: 300_000.0,
            },
            paused: Vec::new(),
        }
    }

    /// Both directions stopped, and both settings still ON — §4.2's whole point.
    fn paused_payload() -> MainPageWatcherData {
        MainPageWatcherData {
            pull: ToggleStateData {
                enabled: true,
                activity: ToggleActivityData::Paused,
                deadline: None,
                interval_ms: 30_000.0,
            },
            publish: ToggleStateData {
                enabled: true,
                activity: ToggleActivityData::Paused,
                deadline: None,
                interval_ms: 300_000.0,
            },
            paused: Vec::new(),
        }
    }

    #[wasm_bindgen_test]
    fn an_armed_toggle_draws_a_ring() {
        // `Countdown` renders a `role="progressbar"` svg. A toggle that is armed and
        // draws no ring is the countdown silently missing, which is what this pins.
        let el = mount(|| view! { <AutosyncBody data=armed_payload() /> });
        assert_eq!(
            el.query_selector_all("[role=progressbar]")
                .unwrap()
                .length(),
            1,
            "one ring: pull is armed, publish is idle"
        );
    }

    #[wasm_bindgen_test]
    fn a_paused_toggle_draws_no_ring_and_offers_nothing_to_press() {
        // §5: the card REPORTS, the queue acts. Every `PausedReason` is user-fixable
        // and its fix is already some queue row's action, so there is no [Resume]
        // here or anywhere — and v1's "push manually to resume" must not reappear.
        let el = mount(|| view! { <AutosyncBody data=paused_payload() /> });
        let text = el.text_content().unwrap();
        assert!(text.contains("Paused"), "got: {text}");
        assert_eq!(
            el.query_selector_all("[role=progressbar]")
                .unwrap()
                .length(),
            0,
            "a stopped countdown must not keep counting"
        );
        assert!(!text.to_lowercase().contains("resume"), "got: {text}");
    }

    #[wasm_bindgen_test]
    fn an_idle_publish_says_what_it_is_waiting_for() {
        // A blank would leave the user guessing between broken, working, and nothing
        // to do — the gallery scene's own note. `Countdown` renders nothing for a
        // `None` deadline precisely so the caller supplies its own words.
        let el = mount(|| view! { <AutosyncBody data=armed_payload() /> });
        assert!(
            el.text_content().unwrap().contains("nothing to publish"),
            "got: {}",
            el.text_content().unwrap()
        );
    }

    #[wasm_bindgen_test]
    fn both_directions_are_named_and_their_intervals_are_derived() {
        // §1: the number is derived from the value it describes, never sent as
        // words. 30_000 ms is "30s"; 300_000 ms is "5 min".
        let el = mount(|| view! { <AutosyncBody data=armed_payload() /> });
        let text = el.text_content().unwrap();
        assert!(text.contains("Get new revisions"), "got: {text}");
        assert!(text.contains("Publish your changes"), "got: {text}");
        assert!(text.contains("Every 30s"), "got: {text}");
        assert!(text.contains("5 min after your last edit"), "got: {text}");
    }

    #[wasm_bindgen_test]
    fn the_checkbox_shows_the_setting_not_the_activity() {
        // §4.2: `enabled` is the SETTING and stays true while paused — what stopped
        // is the machinery. A card that unchecked itself on a pause would tell the
        // user they had turned something off.
        let el = mount(|| view! { <AutosyncBody data=paused_payload() /> });
        let boxes = el.query_selector_all("input[type=checkbox]").unwrap();
        assert_eq!(boxes.length(), 2);
        for i in 0..boxes.length() {
            let input: web_sys::HtmlInputElement = boxes.item(i).unwrap().dyn_into().unwrap();
            assert!(input.checked(), "checkbox {i} must follow `enabled`");
        }
    }

    #[wasm_bindgen_test]
    fn thirty_seconds_reads_as_seconds_and_five_minutes_as_minutes() {
        assert_eq!(human_interval(30_000.0), "30s");
        assert_eq!(human_interval(300_000.0), "5 min");
        assert_eq!(human_interval(90_000.0), "90s");
    }
}
