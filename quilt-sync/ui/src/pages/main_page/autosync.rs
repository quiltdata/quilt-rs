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

use std::future::Future;
use std::time::Duration;

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

/// The shortest wait this card will schedule for a deadline still ahead of it.
///
/// Protects against a deadline a few milliseconds out: waking for it schedules a
/// refetch that can beat the watcher to the lock and read the same deadline back.
/// Two seconds bounds that, and costs nothing — a ring that sits full for up to two
/// seconds of a thirty-second cycle is what `kit/countdown.rs` already documents as
/// truthful: *"A full ring means 'due', not 'fired'"*.
const REFETCH_FLOOR: Duration = Duration::from_secs(2);

/// The wait for a deadline that has already passed.
///
/// Protects against something different, and longer-lived: `next_pull_at` is armed
/// *before* the loop sleeps, so it names the moment the sleep ends — the moment the
/// tick starts. For the whole of that tick (a status walk with a network round trip
/// per package) the recorded deadline is in the past, and nothing will move it. At
/// `REFETCH_FLOOR` the card would refetch every two seconds for the length of the
/// tick, and every refetch rebuilds the body, re-mounts the ring's svg and re-seeds
/// its CSS animation — a ring restarting from zero over and over, where sitting
/// full is the truthful drawing. A past deadline is *due*, not imminent, so waiting
/// longer for it costs nothing: `visibilitychange` still beats this, and a tick that
/// finishes arms a fresh deadline the next wake reads.
const REFETCH_DUE_FLOOR: Duration = Duration::from_secs(10);

/// The longest wait this card will schedule.
///
/// Not a policy — a guard against two panics. `set_timeout_with_handle` converts
/// the delay to `i32` milliseconds (`duration.as_millis().try_into().unwrap_throw()`)
/// and throws above ~24.9 days, and `Duration::from_secs_f64` panics on a
/// non-finite argument; either takes the page down. A deadline further out than a
/// day is also one the visibility listener beats, so waking early costs nothing.
const REFETCH_CEILING: Duration = Duration::from_hours(24);

/// How long to wait before asking the backend for a fresh deadline.
///
/// `None` when nothing is counting down: an idle or paused toggle has no moment
/// worth waking for, and polling one would be a timer where the design has none.
///
/// Which floor applies depends on whether the deadline is merely imminent or
/// already due — see the two constants for what each protects against.
///
/// Total, and deliberately so — a panic in wasm takes the page, and every argument
/// that could cause one is bounded here. `Duration::from_secs_f64` panics on a
/// negative, NaN, or infinite argument; `set_timeout_with_handle` throws on a delay
/// that does not fit `i32` milliseconds. The floors and the ceiling between them
/// leave nothing outside `[2s, 24h]`.
///
/// The order is load-bearing: `f64::max` returns the non-NaN operand, so the floor
/// absorbs NaN first. `f64::clamp` would not do — it returns NaN for a NaN
/// receiver, reinstating the panic the floor exists to prevent. NaN is not `<= 0.0`
/// either, so it takes the imminent branch and lands on `REFETCH_FLOOR` — a floor
/// still, which is all totality asks.
fn delay_until(deadline: Option<f64>, now: f64) -> Option<Duration> {
    let remaining_secs = (deadline? - now) / 1000.0;
    let floor = if remaining_secs <= 0.0 {
        REFETCH_DUE_FLOOR
    } else {
        REFETCH_FLOOR
    };
    Some(Duration::from_secs_f64(
        remaining_secs
            .max(floor.as_secs_f64())
            .min(REFETCH_CEILING.as_secs_f64()),
    ))
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
fn AutosyncBody(data: MainPageWatcherData, reload: Trigger) -> impl IntoView {
    // The SETTING, which stays true while paused. Local signals because
    // `ToggleRow` writes them on change; the payload is the source of truth and
    // a refetch rebuilds this component with it.
    let pull_checked = RwSignal::new(data.pull.enabled);
    let publish_checked = RwSignal::new(data.publish.enabled);
    let pull_every = human_interval(data.pull.interval_ms);
    let publish_after = human_interval(data.publish.interval_ms);

    // The only clock in this card. Not an interval: `Countdown` draws itself from
    // CSS and needs a correct deadline, not a tick — so the single thing the UI
    // must do is ask again when this deadline expires. The earlier of the two
    // deadlines, because whichever expires first is the one that makes the payload
    // stale.
    //
    // One `set_timeout` per body, and a body is rebuilt per payload, so the
    // handle's lifetime is the deadline's lifetime. `on_cleanup` clears it when the
    // payload is replaced or the page unmounts — the same shape
    // `components/set_remote_popup.rs` uses for its debounce.
    let armed = [&data.pull, &data.publish]
        .into_iter()
        .filter_map(|t| t.deadline)
        .min_by(f64::total_cmp);
    let timer: StoredValue<Option<TimeoutHandle>> = StoredValue::new(None);
    if let Some(delay) = delay_until(armed, js_sys::Date::now())
        && let Ok(handle) = set_timeout_with_handle(move || reload.notify(), delay)
    {
        timer.set_value(Some(handle));
    }
    on_cleanup(move || {
        if let Some(Some(handle)) = timer.try_get_value() {
            handle.clear();
        }
    });

    // Write, then ask again. The off->on edge clears the paused set, so the
    // payload after a write differs by more than the bit that was flipped.
    let write = move |pull: Option<bool>, push: Option<bool>| {
        leptos::task::spawn_local(async move {
            if let Err(err) = commands::set_autosync_direction(pull, push).await {
                web_sys::console::error_1(&format!("set_autosync_direction failed: {err}").into());
            }
            reload.notify();
        });
    };

    // `ToggleRow` owns the `on:change`, so the change is observed through the
    // signal rather than a handler. What the backend last heard, seeded from the
    // payload: an effect runs once on mount, and writing back the value it just
    // read would be a write per render.
    let pull_written = StoredValue::new(data.pull.enabled);
    let publish_written = StoredValue::new(data.publish.enabled);
    Effect::new(move |_| {
        let enabled = pull_checked.get();
        if pull_written.get_value() != enabled {
            pull_written.set_value(enabled);
            write(Some(enabled), None);
        }
    });
    Effect::new(move |_| {
        let enabled = publish_checked.get();
        if publish_written.get_value() != enabled {
            publish_written.set_value(enabled);
            write(None, Some(enabled));
        }
    });

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

/// Refetch when the window becomes visible again.
///
/// Split into its own component so it can be mounted alone in a test, and kept out
/// of [`AutosyncBody`] because the body is replaced on every payload while this
/// should be registered once.
///
/// `visibilitychange` is fired at the document and bubbles, so a window listener
/// sees it. The guard matters: it fires on becoming hidden too, and refetching then
/// would wake a backgrounded page to read a clock nobody is looking at.
#[component]
fn AutosyncListener(reload: Trigger) -> impl IntoView {
    let handle = window_event_listener(leptos::ev::visibilitychange, move |_| {
        if !document().hidden() {
            reload.notify();
        }
    });
    on_cleanup(move || handle.remove());
}

/// The card's payload resource, and the two triggers that refetch it.
///
/// A free function rather than a line inside [`AutosyncCard`] because the fetch is
/// the only observable a refetch has — with the fetcher as a parameter a test can
/// mount this exact wiring and count the calls. `AutosyncCard` passes the real
/// command.
///
/// Two triggers, not one. `reload` is the card's own: the expiring deadline, the
/// visibility listener, a toggle write. `refresh` is the page's, so the appbar's
/// Refresh button reaches this card too — without it a Refresh updates the package
/// rows and leaves the card asserting `Paused` beside rows that have just stopped
/// being conflicted, and a paused card has no deadline, so its own trigger has
/// nothing scheduled to correct it.
fn watcher_resource<Fut>(
    reload: Trigger,
    refresh: Trigger,
    fetch: impl Fn() -> Fut + 'static,
) -> LocalResource<Result<MainPageWatcherData, String>>
where
    Fut: Future<Output = Result<MainPageWatcherData, String>> + 'static,
{
    LocalResource::new(move || {
        reload.track();
        refresh.track();
        fetch()
    })
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
pub fn AutosyncCard(
    /// The page's own reload trigger, so the appbar's Refresh refetches this card
    /// as well as the package rows.
    refresh: Trigger,
) -> impl IntoView {
    // Notified when a deadline expires, when the window becomes visible again, and
    // after a toggle write.
    let reload = Trigger::new();
    let watcher = watcher_resource(reload, refresh, commands::get_main_page_watcher);

    view! {
        <AutosyncListener reload=reload />
        <Transition fallback=|| ()>
            {move || Suspend::new(async move {
                match watcher.await {
                    Ok(data) => view! { <AutosyncBody data=data reload=reload /> }.into_any(),
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

    /// A promise-backed sleep. The crate has no `gloo-timers` and needs none:
    /// this is four lines over `set_timeout` and `wasm-bindgen-futures` is
    /// already a dependency.
    async fn sleep_ms(ms: i32) {
        let promise = js_sys::Promise::new(&mut |resolve, _| {
            window()
                .set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms)
                .unwrap();
        });
        wasm_bindgen_futures::JsFuture::from(promise).await.unwrap();
    }

    /// Pull armed `ms` from now, publish with nothing to do.
    fn armed_in_ms(ms: f64) -> MainPageWatcherData {
        MainPageWatcherData {
            pull: ToggleStateData {
                enabled: true,
                activity: ToggleActivityData::Armed,
                deadline: Some(js_sys::Date::now() + ms),
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

    /// Pull counting down, publish with nothing to do — the ordinary steady state.
    fn armed_payload() -> MainPageWatcherData {
        armed_in_ms(23_000.0)
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
        let el = mount(|| view! { <AutosyncBody data=armed_payload() reload=Trigger::new() /> });
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
        let el = mount(|| view! { <AutosyncBody data=paused_payload() reload=Trigger::new() /> });
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
        let el = mount(|| view! { <AutosyncBody data=armed_payload() reload=Trigger::new() /> });
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
        let el = mount(|| view! { <AutosyncBody data=armed_payload() reload=Trigger::new() /> });
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
        let el = mount(|| view! { <AutosyncBody data=paused_payload() reload=Trigger::new() /> });
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

    #[wasm_bindgen_test]
    fn nothing_counting_down_wakes_for_nothing() {
        // No deadline means no moment worth waking for. An idle or paused toggle
        // must not schedule a poll.
        assert_eq!(delay_until(None, 1_000.0), None);
    }

    #[wasm_bindgen_test]
    fn a_future_deadline_waits_for_it() {
        assert_eq!(
            delay_until(Some(31_000.0), 1_000.0),
            Some(Duration::from_secs(30))
        );
    }

    #[wasm_bindgen_test]
    fn a_deadline_about_to_arrive_waits_the_imminent_floor() {
        // Still ahead, but not worth a wake of its own: the refetch would race the
        // watcher's tick to the lock and read the same deadline back.
        // One of these two names the constant and one pins its value, so changing
        // `REFETCH_FLOOR` cannot pass unnoticed.
        assert_eq!(
            delay_until(Some(60_100.0), 60_000.0),
            Some(Duration::from_secs(2))
        );
        assert_eq!(delay_until(Some(60_500.0), 60_000.0), Some(REFETCH_FLOOR));
    }

    #[wasm_bindgen_test]
    fn a_deadline_already_past_waits_the_due_floor_not_the_imminent_one() {
        // A deadline already past is not one about to arrive. `next_pull_at` is
        // armed before the loop sleeps, so it stays in the past for the whole of
        // the tick that follows — a status walk with a network round trip per
        // package — and nothing moves it in the meantime. At the imminent floor
        // the card would refetch every two seconds for the length of that tick,
        // re-seeding the ring's animation each time, where `Countdown`'s own doc
        // says sitting full is the truthful drawing: "A wake-up that sits at full
        // for a few seconds is therefore truthful rather than broken."
        // The deadline exactly at `now` is due, not imminent — it has arrived.
        // One of these two names the constant and one pins its value, so changing
        // `REFETCH_DUE_FLOOR` cannot pass unnoticed.
        assert_eq!(
            delay_until(Some(0.0), 60_000.0),
            Some(Duration::from_secs(10))
        );
        assert_eq!(
            delay_until(Some(60_000.0), 60_000.0),
            Some(REFETCH_DUE_FLOOR)
        );
    }

    #[wasm_bindgen_test]
    fn a_nonsense_deadline_waits_the_floor_rather_than_panicking() {
        // `Duration::from_secs_f64` panics on a negative or NaN argument, and a
        // panic in wasm takes the whole page.
        assert_eq!(delay_until(Some(f64::NAN), 1_000.0), Some(REFETCH_FLOOR));
    }

    #[wasm_bindgen_test]
    fn a_deadline_too_far_out_waits_the_ceiling_rather_than_throwing() {
        // Two panics live above the floor: `Duration::from_secs_f64` panics on a
        // non-finite argument, and `set_timeout_with_handle` converts the delay to
        // `i32` milliseconds and throws above ~24.9 days.
        assert_eq!(delay_until(Some(f64::INFINITY), 0.0), Some(REFETCH_CEILING));
        // The reachable one, and the reason this is not exotic: a well-formed
        // deadline a month out needs no corruption, only a large `interval_ms`.
        assert_eq!(
            delay_until(Some(30.0 * 24.0 * 60.0 * 60.0 * 1000.0), 0.0),
            Some(REFETCH_CEILING)
        );
    }

    #[wasm_bindgen_test]
    async fn an_expiring_deadline_asks_the_backend_again() {
        // The end-to-end property, and the one the pure function cannot show: the
        // scheduled wake actually notifies. Mount a body whose deadline has all
        // but arrived, sleep past the wake the floor holds it to, and assert the
        // trigger fired exactly once.
        //
        // `AutosyncBody` takes the trigger as a prop so this test can watch it;
        // the card owns the resource and passes its own.
        let fired = RwSignal::new(0);
        let reload = Trigger::new();
        Effect::new(move |_| {
            reload.track();
            fired.update(|n| *n += 1);
        });
        // 500ms, not 30: comfortably inside `REFETCH_FLOOR`, and comfortably
        // ahead of the clock `delay_until` reads a moment later, so the body takes
        // the imminent branch even if the runtime stalls between building this
        // payload and mounting it. A deadline that had slipped into the past would
        // take the due branch and wait `REFETCH_DUE_FLOOR` instead.
        let el = mount(move || {
            view! { <AutosyncBody data=armed_in_ms(500.0) reload=reload /> }
        });
        // Let the effect's own first, unprompted run land before counting, and
        // pin it — otherwise a delta of one could be that run and no wake at all.
        sleep_ms(50).await;
        let before = fired.get_untracked();
        assert_eq!(before, 1, "the effect's first run, before any wake");
        // 2500ms: an absolute bound, not a multiple of anything. Long enough for
        // a wake held to `REFETCH_FLOOR`, short enough that a spin shows up as
        // more than one.
        sleep_ms(2_500).await;
        assert_eq!(
            fired.get_untracked() - before,
            1,
            "the deadline must schedule exactly one refetch, not zero and not a spin"
        );
        drop(el);
    }

    #[wasm_bindgen_test]
    async fn flipping_a_toggle_asks_the_backend_again() {
        // The write and the refetch are one gesture: the off->on edge clears the
        // paused set, so the payload after a write differs by more than the bit the
        // user flipped. A card that wrote without refetching would keep showing
        // `Paused` on a toggle that had just cleared every pause.
        //
        // `paused_payload` has no deadline, so nothing here is scheduled: the only
        // thing that can notify `reload` in this test is the flip.
        let fired = RwSignal::new(0);
        let reload = Trigger::new();
        Effect::new(move |_| {
            reload.track();
            fired.update(|n| *n += 1);
        });
        let el = mount(move || view! { <AutosyncBody data=paused_payload() reload=reload /> });
        // Let the effect queue drain — the test's own effect and the body's two
        // write guards all run once at mount — and pin the baseline, so a stale
        // read cannot pass for the flip's notify.
        sleep_ms(50).await;
        let before = fired.get_untracked();
        assert_eq!(
            before, 1,
            "the effect's first run; mounting must not write anything back"
        );
        let input: web_sys::HtmlInputElement = el
            .query_selector("input[type=checkbox]")
            .unwrap()
            .expect("the pull toggle")
            .dyn_into()
            .unwrap();
        input.click();
        // 200ms: an absolute bound, not a multiple of any constant this file reads.
        // There is no Tauri bridge here, so the write resolves as `Err` at once and
        // only the notify that follows it is under test.
        sleep_ms(200).await;
        assert_eq!(
            fired.get_untracked() - before,
            1,
            "flipping a toggle must write and then ask for the payload again"
        );
        drop(el);
    }

    #[wasm_bindgen_test]
    async fn the_pages_refresh_asks_the_backend_again() {
        // The appbar's Refresh notifies `MainPage`'s trigger, which is this card's
        // `refresh` prop. Without that wiring a Refresh updates the package rows
        // and leaves the card asserting `Paused` beside rows that have just
        // stopped being conflicted — and a paused payload carries no deadline, so
        // the card's own trigger has nothing scheduled to correct it.
        //
        // The fetch is the only observable a refetch has, which is why
        // `watcher_resource` takes its fetcher: this mounts the card's exact
        // wiring with a counting one.
        let calls = RwSignal::new(0);
        let reload = Trigger::new();
        let refresh = Trigger::new();
        let _el = mount(move || {
            let watcher = watcher_resource(reload, refresh, move || {
                calls.update(|n| *n += 1);
                async move { Ok(armed_payload()) }
            });
            view! {
                <Transition fallback=|| ()>
                    {move || Suspend::new(async move {
                        let _ = watcher.await;
                    })}
                </Transition>
            }
        });
        // Let the first fetch land and pin the baseline, so a stale read cannot
        // pass for the refresh's own.
        sleep_ms(50).await;
        let before = calls.get_untracked();
        assert_eq!(before, 1, "the resource's first fetch, before any refresh");
        refresh.notify();
        // 200ms: an absolute bound, not a multiple of any constant this file
        // reads. Nothing else here can fetch — the payload is idle-and-armed but
        // no body is mounted to schedule a wake from it.
        sleep_ms(200).await;
        assert_eq!(
            calls.get_untracked() - before,
            1,
            "the page's Refresh must refetch the card, exactly once"
        );
    }

    #[wasm_bindgen_test]
    async fn becoming_visible_again_asks_the_backend_again() {
        // A slept machine wakes with a deadline many ticks old. The CSS ring is
        // correctly placed for the deadline it was seeded with — which is the
        // wrong deadline — so the fix is to refetch, not to count elapsed ticks.
        let fired = RwSignal::new(0);
        let reload = Trigger::new();
        Effect::new(move |_| {
            reload.track();
            fired.update(|n| *n += 1);
        });
        let _el = mount(move || view! { <AutosyncListener reload=reload /> });
        sleep_ms(50).await;
        let before = fired.get_untracked();
        assert_eq!(before, 1, "the effect's first run, before any event");
        let event = web_sys::Event::new("visibilitychange").unwrap();
        window().dispatch_event(&event).unwrap();
        sleep_ms(50).await;
        assert_eq!(fired.get_untracked() - before, 1);
    }
}
