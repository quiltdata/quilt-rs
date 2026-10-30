//! The quit prompt's answers; *Quit anyway* belongs to the desktop app.
//!
//! The decision itself lives in [`crate::engine::quit`]; these are the window's way of
//! reporting what it did with it.

use crate::engine::services::Services;

/// The prompt is on screen. Until this lands the deferred quit is
/// unanswerable, and the tray's grace period lets it through.
///
/// Quotes the generation the prompt was raised with, so an ack from a prompt
/// the user has since dismissed cannot vouch for the one now outstanding.
#[allow(
    clippy::unused_async,
    reason = "every plain command is async, so the dispatch treats them alike"
)]
pub async fn quit_prompt_shown(s: &Services, generation: u64) -> Result<(), String> {
    s.quit_gate.note_shown(generation);
    Ok(())
}

/// *Stay* — the quit is abandoned, and a later one asks afresh.
#[allow(
    clippy::unused_async,
    reason = "every plain command is async, so the dispatch treats them alike"
)]
pub async fn quit_cancel(s: &Services) -> Result<(), String> {
    s.quit_gate.cancel();
    Ok(())
}
