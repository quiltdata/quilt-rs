//! The quit prompt's three answers.
//!
//! The decision itself lives in [`crate::quit`]; these are the window's way of
//! reporting what it did with it.

use tauri::AppHandle;

use crate::services::Services;

/// The prompt is on screen. Until this lands the deferred quit is
/// unanswerable, and the grace period in [`crate::tray`] lets it through.
///
/// Quotes the generation the prompt was raised with, so an ack from a prompt
/// the user has since dismissed cannot vouch for the one now outstanding.
#[allow(
    clippy::unused_async,
    reason = "every plain command is async, so the IPC wrappers and the dispatcher treat them alike"
)]
pub async fn quit_prompt_shown(s: &Services, generation: u64) -> Result<(), String> {
    s.quit_gate.note_shown(generation);
    Ok(())
}

/// *Quit anyway* — the user accepts interrupting the apply. Desktop only.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri's command macro dictates the signature: the app handle is injected by value"
)]
pub fn quit_confirm(app: AppHandle) {
    app.exit(0);
}

/// *Stay* — the quit is abandoned, and a later one asks afresh.
#[allow(
    clippy::unused_async,
    reason = "every plain command is async, so the IPC wrappers and the dispatcher treat them alike"
)]
pub async fn quit_cancel(s: &Services) -> Result<(), String> {
    s.quit_gate.cancel();
    Ok(())
}
