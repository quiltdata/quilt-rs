//! Progress toward a deadline, drawn as a determinate ring.
//!
//! # No timer
//!
//! Rust reads the clock exactly once — at render, to work out how far into the
//! cycle we already are — and CSS does the rest via a negative `animation-delay`.
//! There is no interval, nothing to dispose of, and no per-second re-render.
//!
//! That also removes a class of bug rather than managing it. A JS countdown that
//! decrements on a tick drifts whenever ticks stop arriving on schedule — a
//! throttled view, a busy CPU, a machine that slept — and the error accumulates
//! with nothing to correct it. A CSS animation is positioned by the document
//! timeline, so it is simply *at* the right place when the machine wakes.
//!
//! # It is a prediction, not the truth
//!
//! The watcher decides when it actually ticks; this only estimates from the last
//! known deadline. A full ring means "due", not "fired", and the real status event
//! is what re-seeds it. A wake-up that sits at full for a few seconds is therefore
//! truthful rather than broken.

use leptos::prelude::*;

stylance::import_crate_style!(style, "src/kit/countdown.module.scss");

/// Epoch milliseconds, as `js_sys::Date::now()` reports them.
///
/// Not `chrono` — the UI crate has no date/time dependency, and
/// `std::time::SystemTime::now()` panics on `wasm32-unknown-unknown`. Whether to
/// add `chrono` for the whole DTO layer is a separate decision; this component
/// does not force it.
pub type EpochMillis = f64;

/// Ring geometry, in the SVG's own units. The circumference reaches CSS as a
/// custom property so the dash array and the keyframes cannot disagree.
const RADIUS: f64 = 6.0;
const CIRCUMFERENCE: f64 = 2.0 * std::f64::consts::PI * RADIUS;

#[component]
pub fn Countdown(
    /// When the next tick is due. `None` renders nothing, and the caller supplies
    /// its own idle text — that keeps the copy where the caller's vocabulary is
    /// (`nothing to publish` belongs to the publish toggle, not to a clock).
    #[prop(into)]
    deadline: Signal<Option<EpochMillis>>,
    /// The whole wait, in milliseconds — 30s for the pull tick, the quiet window
    /// for publish. A determinate ring cannot be drawn from a remaining time
    /// alone; it needs to know what that time is a fraction of.
    interval: f64,
    /// What the cycle *is*, for the hover title and the accessible name — for
    /// example "Checks for new revisions every 30 seconds".
    ///
    /// Deliberately the period rather than the remainder. A ring alone says
    /// something is progressing but not when, and R1 forbids leaving that
    /// unlabelled — but a live `0:23` would need the per-second tick this
    /// component exists to avoid, and the caller is the only thing that knows what
    /// the cycle means anyway.
    #[prop(into)]
    aria_label: String,
    /// Loop at the interval. True for the pull tick, which recurs; false for the
    /// publish quiet window, which happens once and then holds full.
    #[prop(optional)]
    repeat: bool,
) -> impl IntoView {
    let class = if repeat {
        format!("{} {}", style::arc, style::repeat)
    } else {
        style::arc.to_string()
    };

    // The single clock read. Anything already elapsed becomes a negative delay,
    // which is what places the animation mid-cycle.
    //
    // Geometry goes out as SVG *attributes* and timing as custom properties, so
    // the two fail independently. If the custom properties never arrive, the
    // attributes still draw the arc at its correct seeded position — a legible
    // frozen ring that says "timing is broken", rather than a full circle that
    // looks like a finished countdown.
    let seed = move || {
        deadline.get().map(|deadline| {
            let remaining_ms = (deadline - js_sys::Date::now()).clamp(0.0, interval);
            let elapsed_ms = interval - remaining_ms;
            let offset = CIRCUMFERENCE * (remaining_ms / interval);
            let style = format!(
                "--cd-circumference:{CIRCUMFERENCE}; --cd-duration:{interval}ms; \
                 --cd-delay:-{elapsed_ms}ms; --cd-seeded-offset:{offset}"
            );
            (style, offset)
        })
    };

    view! {
        {move || {
            seed()
                .map(|(seed, offset)| {
                    let class = class.clone();
                    let aria_label = aria_label.clone();
                    view! {
                        // `progressbar` rather than an image: a screen reader
                        // reports it on request instead of announcing every tick.
                        <svg
                            class=style::root
                            style=seed
                            viewBox="0 0 16 16"
                            role="progressbar"
                            aria-label=aria_label.clone()
                            title=aria_label
                        >
                            <circle
                                class=style::track
                                cx="8"
                                cy="8"
                                r=RADIUS
                                fill="none"
                                stroke-width="2.5"
                            />
                            <circle
                                class=class
                                cx="8"
                                cy="8"
                                r=RADIUS
                                fill="none"
                                stroke-width="2.5"
                                stroke-linecap="round"
                                stroke-dasharray=CIRCUMFERENCE
                                stroke-dashoffset=offset
                            />
                        </svg>
                    }
                })
        }}
    }
}
