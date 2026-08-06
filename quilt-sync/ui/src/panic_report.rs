//! Get a frontend panic to somebody who can act on it.
//!
//! The UI is a WASM module in a webview. Its panic hook can reach the browser
//! console, and that is the whole of its reach — so a panic here was visible only to
//! a developer with the inspector open, which no user has and no support archive
//! contains. The backend holds the crash client, so the report goes there.

use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct PanicArgs {
    message: String,
}

/// Report panics to the backend, *in addition* to whatever hook is already set.
///
/// Chains rather than replaces: the console hook stays, because its output is what a
/// developer actually reads while working, and this is for the panic nobody is
/// watching. Reporting happens first — the previous hook ends by throwing into JS,
/// so anything after it may not run.
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        crate::tauri::invoke_and_forget(
            "report_ui_panic",
            &PanicArgs {
                message: info.to_string(),
            },
        );
        previous(info);
    }));
}
