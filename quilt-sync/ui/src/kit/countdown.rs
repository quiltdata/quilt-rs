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
//! # Hidden from the accessibility tree
//!
//! The ring is `aria-hidden` and carries no role and no name.
//!
//! It cannot report a value. The arc is placed by CSS from the single clock read
//! above, so anything written into the DOM is stale the moment it lands, and
//! keeping it current needs the timer this component exists to avoid. A
//! `progressbar` with no `aria-valuenow` reads as indeterminate, which the ring
//! is not.
//!
//! Nothing is lost by hiding it. Every caller states the cycle in words the
//! reader already reaches — `Every 30s, keeping any local changes` sits inside
//! the toggle's own `<label>`, so it is part of the checkbox's accessible name.
//! The ring adds where we are within that cycle, which is the part it cannot
//! say.
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
    /// What the cycle *is*, for the hover title — for example "Checks for new
    /// revisions every 30 seconds".
    ///
    /// Deliberately the period rather than the remainder: a live `0:23` would
    /// need the per-second tick this component exists to avoid, and the caller
    /// is the only thing that knows what the cycle means anyway.
    ///
    /// Sighted readers only. The ring is hidden from the accessibility tree, and
    /// the caller states the cycle in words — see the module doc.
    #[prop(into)]
    title: String,
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
                    let title = title.clone();
                    view! {
                        // Hidden from the accessibility tree — see the module doc.
                        // The `title` is the sighted reader's hover text and
                        // nothing else.
                        <svg
                            class=style::root
                            style=seed
                            viewBox="0 0 16 16"
                            aria-hidden="true"
                            title=title
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::mount;
    use wasm_bindgen_test::*;

    fn ring(el: &web_sys::Element) -> Option<web_sys::Element> {
        el.query_selector("svg").unwrap()
    }

    /// The module doc's claim, pinned: no role and no name, because the ring has
    /// no live value to report and the caller already states the cycle in words.
    #[wasm_bindgen_test]
    fn the_ring_is_hidden_from_the_accessibility_tree() {
        let el = mount(|| {
            view! {
                <Countdown
                    deadline=Signal::stored(Some(js_sys::Date::now() + 10_000.0))
                    interval=30_000.0
                    title="Checks for new revisions every 30s"
                />
            }
        });
        let ring = ring(&el).expect("the ring");
        assert_eq!(ring.get_attribute("aria-hidden").as_deref(), Some("true"));
        assert_eq!(ring.get_attribute("role"), None, "no role at all");
        assert_eq!(ring.get_attribute("aria-label"), None, "and no name");
        assert_eq!(
            ring.get_attribute("title").as_deref(),
            Some("Checks for new revisions every 30s"),
            "the hover text stays: it is for sighted readers"
        );
    }

    /// `None` is the caller's cue to draw its own idle words, so the component
    /// must draw nothing at all rather than an empty or full ring.
    #[wasm_bindgen_test]
    fn no_deadline_draws_no_ring() {
        let el = mount(|| {
            view! {
                <Countdown
                    deadline=Signal::stored(None)
                    interval=30_000.0
                    title="Checks for new revisions every 30s"
                />
            }
        });
        assert!(ring(&el).is_none());
    }
}
