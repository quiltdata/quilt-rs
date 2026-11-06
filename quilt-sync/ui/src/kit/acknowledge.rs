//! How long an in-place acknowledgment stands: a copy mark, a row's *Copied*,
//! the accounts card's announcement, the settings page's *Saved*.
//!
//! One value for all of them, so a reader learns one rhythm. Long enough to be
//! seen after the click that caused it, short enough that a reader coming back
//! later is not told something stale.

use std::time::Duration;

pub const ACKNOWLEDGE_FOR: Duration = Duration::from_millis(1_500);
