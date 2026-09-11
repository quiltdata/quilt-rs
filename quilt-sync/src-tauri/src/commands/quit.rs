//! The quit prompt's three answers.
//!
//! The decision itself lives in [`crate::quit`]; these are the window's way of
//! reporting what it did with it.

use tauri::AppHandle;
use tauri::State;

use crate::quit::QuitGate;

/// The prompt is on screen. Until this lands the deferred quit is
/// unanswerable, and the grace period in [`crate::tray`] lets it through.
///
/// Quotes the generation the prompt was raised with, so an ack from a prompt
/// the user has since dismissed cannot vouch for the one now outstanding.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri's command macro dictates the signature: state and the app handle are \
              injected by value. Sync rather than async because there is nothing to await, \
              and an async command taking state would have to return a Result it could \
              never populate."
)]
pub fn quit_prompt_shown(gate: State<'_, QuitGate>, generation: u64) {
    gate.note_shown(generation);
}

/// *Quit anyway* — the user accepts interrupting the apply.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri's command macro dictates the signature: state and the app handle are \
              injected by value. Sync rather than async because there is nothing to await, \
              and an async command taking state would have to return a Result it could \
              never populate."
)]
pub fn quit_confirm(app: AppHandle) {
    app.exit(0);
}

/// *Stay* — the quit is abandoned, and a later one asks afresh.
#[tauri::command]
#[allow(
    clippy::needless_pass_by_value,
    reason = "Tauri's command macro dictates the signature: state and the app handle are \
              injected by value. Sync rather than async because there is nothing to await, \
              and an async command taking state would have to return a Result it could \
              never populate."
)]
pub fn quit_cancel(gate: State<'_, QuitGate>) {
    gate.cancel();
}
