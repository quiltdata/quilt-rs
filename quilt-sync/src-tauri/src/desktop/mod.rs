//! The desktop app: the window, the tray, deep links and the desktop features
//! the engine's commands use.
//!
//! The window is one more client of the engine: it loads the engine's page and
//! talks over its socket like a browser tab.

use tauri::AppHandle;
use tauri::Manager;
use tauri::WebviewUrl;
use tauri::WebviewWindow;
use tauri::WebviewWindowBuilder;

pub mod commands;
pub mod tray;
pub mod uri;

/// The window's label, for every lookup of it.
pub const MAIN: &str = "main";

/// Open the window on `link`, the engine's page with its token.
///
/// # Errors
///
/// When `link` isn't a URL or Tauri can't build the window.
pub fn open_window(app: &AppHandle, link: &str) -> tauri::Result<WebviewWindow> {
    let url = link.parse().map_err(tauri::Error::InvalidUrl)?;
    WebviewWindowBuilder::new(app, MAIN, WebviewUrl::External(url))
        .title("QuiltSync")
        .inner_size(1280.0, 900.0)
        .min_inner_size(640.0, 560.0)
        .resizable(true)
        .build()
}

/// Bring the window to the front, from the tray or a deep link.
pub fn show_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window(MAIN) {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        #[cfg(target_os = "macos")]
        {
            let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
        }
    }
}
