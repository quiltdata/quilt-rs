//! A timestamp as elapsed time, with the exact value on hover.
//!
//! # It does not tick
//!
//! Rendered once from the value it is given, so "2 min ago" stays "2 min ago"
//! until something re-renders the row. That is deliberate: a list of forty rows
//! would otherwise want forty timers, and the page already re-renders on every
//! autosync status event, which is far more often than a minute.
//!
//! The cost is that a page left open for an hour shows stale relative times. The
//! exact value in the `title` is never stale, which is the other reason it is
//! there.

use leptos::prelude::*;
use wasm_bindgen::JsValue;

use super::countdown::EpochMillis;

stylance::import_crate_style!(style, "src/kit/relative_time.module.scss");

/// Coarse buckets, deliberately. Nobody reads a file list to learn that something
/// changed 43 minutes ago rather than 44 — they read it to know whether it was
/// today. Precision belongs in the `title`.
fn phrase(elapsed_ms: f64) -> String {
    let secs = (elapsed_ms / 1000.0).max(0.0);
    let mins = secs / 60.0;
    let hours = mins / 60.0;
    let days = hours / 24.0;

    #[allow(
        clippy::cast_possible_truncation,
        clippy::cast_sign_loss,
        reason = "clamped above zero, and the values are bucketed before casting"
    )]
    match () {
        () if secs < 45.0 => "just now".to_string(),
        () if mins < 60.0 => format!("{} min ago", mins.max(1.0) as u64),
        () if hours < 2.0 => "1 hour ago".to_string(),
        () if hours < 24.0 => format!("{} hours ago", hours as u64),
        () if days < 2.0 => "yesterday".to_string(),
        () if days < 7.0 => format!("{} days ago", days as u64),
        () if days < 14.0 => "1 week ago".to_string(),
        () if days < 60.0 => format!("{} weeks ago", (days / 7.0) as u64),
        () => format!("{} months ago", (days / 30.0).max(2.0) as u64),
    }
}

#[component]
pub fn RelativeTime(
    /// When it happened. Epoch milliseconds, matching `Countdown` — the UI crate
    /// still has no date/time dependency, and `js_sys::Date` covers both the
    /// arithmetic and the locale formatting.
    at: EpochMillis,
) -> impl IntoView {
    let exact = js_sys::Date::new(&JsValue::from_f64(at));
    let title = exact
        .to_locale_string("default", &JsValue::undefined())
        .as_string()
        .unwrap_or_default();
    let machine = exact.to_iso_string().as_string().unwrap_or_default();
    let text = phrase(js_sys::Date::now() - at);

    view! {
        // `<time>` with `datetime`, so the machine-readable value travels with the
        // human one rather than only existing in a tooltip.
        <time class=style::root datetime=machine title=title>
            {text}
        </time>
    }
}
