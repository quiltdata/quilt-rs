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
///
/// The vocabulary is closed and its widest phrase is `11 months ago`, which is what
/// lets a row give the time a fixed column. Years keep it closed at the top.
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
        () if days < 365.0 => format!("{} months ago", (days / 30.0).clamp(2.0, 11.0) as u64),
        () if days < 730.0 => "1 year ago".to_string(),
        () => format!("{} years ago", (days / 365.0) as u64),
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

#[cfg(test)]
mod tests {
    use wasm_bindgen_test::*;

    use super::phrase;

    const SECOND: f64 = 1000.0;
    const MINUTE: f64 = 60.0 * SECOND;
    const HOUR: f64 = 60.0 * MINUTE;
    const DAY: f64 = 24.0 * HOUR;

    #[wasm_bindgen_test]
    fn each_bucket_starts_where_its_neighbour_ends() {
        let cases = [
            (44.0 * SECOND, "just now"),
            (45.0 * SECOND, "1 min ago"),
            (59.0 * MINUTE, "59 min ago"),
            (60.0 * MINUTE, "1 hour ago"),
            (2.0 * HOUR, "2 hours ago"),
            (23.0 * HOUR, "23 hours ago"),
            (24.0 * HOUR, "yesterday"),
            (47.0 * HOUR, "yesterday"),
            (48.0 * HOUR, "2 days ago"),
            (6.0 * DAY, "6 days ago"),
            (7.0 * DAY, "1 week ago"),
            (14.0 * DAY, "2 weeks ago"),
            (59.0 * DAY, "8 weeks ago"),
            (60.0 * DAY, "2 months ago"),
            (364.0 * DAY, "11 months ago"),
            (365.0 * DAY, "1 year ago"),
            (729.0 * DAY, "1 year ago"),
            (730.0 * DAY, "2 years ago"),
            (5.0 * 365.0 * DAY, "5 years ago"),
        ];
        for (elapsed, want) in cases {
            assert_eq!(phrase(elapsed), want, "at {elapsed} ms");
        }
    }

    #[wasm_bindgen_test]
    fn negative_elapsed_is_clamped_to_now() {
        assert_eq!(phrase(-5.0 * MINUTE), "just now");
    }

    /// The row's time column is sized against this: no phrase for any age up to
    /// fifty years is longer than `11 months ago`.
    #[wasm_bindgen_test]
    fn no_phrase_is_wider_than_eleven_months_ago() {
        let widest = "11 months ago".chars().count();
        let mut days = 0.0;
        while days < 50.0 * 365.0 {
            let text = phrase(days * DAY);
            assert!(
                text.chars().count() <= widest,
                "{text:?} at {days} days is wider than {widest} characters"
            );
            days += 1.0;
        }
    }
}
