use tauri::AppHandle;
use tauri::Manager;
use tauri::WindowEvent;
use tauri::async_runtime;
use tauri::image::Image;
use tauri::menu::Menu;
use tauri::menu::MenuItem;
use tauri::tray::MouseButton;
use tauri::tray::MouseButtonState;
use tauri::tray::TrayIcon;
use tauri::tray::TrayIconBuilder;
use tauri::tray::TrayIconEvent;
use thiserror::Error;
use tokio::sync::watch;

use crate::autopull::SharedAutosyncSettings;
use crate::autopull::SharedWindowMode;
use crate::autopull::SyncTrayStatus;
use crate::autopull::TrayMode;
use crate::autopull::WindowMode;

const TRAY_ICON_ASSET: &str = "icons/tray/trayicon.png";

#[derive(Debug, Error)]
pub enum TrayError {
    #[error(transparent)]
    Tauri(#[from] tauri::Error),
    #[error("tray icon asset missing: {0}")]
    MissingIcon(String),
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Owns the tray icon, the status-listener task, and the per-window
/// close handler. Hold one of these for the lifetime of the app —
/// dropping it aborts the listener task and removes the tray icon.
pub struct TrayController {
    _tray: TrayIcon,
    _listener: async_runtime::JoinHandle<()>,
}

const MENU_ID_OPEN: &str = "tray-open";
const MENU_ID_QUIT: &str = "tray-quit";

impl TrayController {
    pub fn install(
        app: &AppHandle,
        status_rx: watch::Receiver<SyncTrayStatus>,
    ) -> Result<Self, TrayError> {
        let open_item = MenuItem::with_id(app, MENU_ID_OPEN, "Open Quilt", true, None::<&str>)?;
        let quit_item = MenuItem::with_id(app, MENU_ID_QUIT, "Quit", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&open_item, &quit_item])?;

        let icon = load_tray_icon(app)?;
        let tray = TrayIconBuilder::new()
            .icon(icon)
            .menu(&menu)
            .tooltip("Quilt — idle")
            .on_menu_event(|app, event| match event.id().as_ref() {
                MENU_ID_OPEN => show_main_window(app),
                MENU_ID_QUIT => app.exit(0),
                _ => {}
            })
            .on_tray_icon_event(|tray, event| {
                // Linux + Windows convention: left-click restores the
                // window. macOS users get the menu on any click and
                // never receive this event in the same shape, so the
                // gate is harmless there.
                if let TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                } = event
                {
                    show_main_window(tray.app_handle());
                }
            })
            .build(app)?;

        let listener = spawn_status_listener(app.clone(), tray.clone(), status_rx);
        let _ = open_item;
        let _ = quit_item;

        install_close_handler(app)?;

        Ok(Self {
            _tray: tray,
            _listener: listener,
        })
    }
}

fn load_tray_icon(app: &AppHandle) -> Result<Image<'static>, TrayError> {
    let path = app
        .path()
        .resolve(TRAY_ICON_ASSET, tauri::path::BaseDirectory::Resource)
        .map_err(|_| TrayError::MissingIcon(TRAY_ICON_ASSET.to_string()))?;
    let bytes = std::fs::read(&path)?;
    let image = Image::from_bytes(&bytes)?.to_owned();
    Ok(image)
}

fn show_main_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
        #[cfg(target_os = "macos")]
        {
            let _ = app.set_activation_policy(tauri::ActivationPolicy::Regular);
        }
        let handle = app.clone();
        async_runtime::spawn(async move {
            let mode = handle.state::<SharedWindowMode>();
            *mode.write().await = WindowMode::Focused;
        });
    }
}

fn spawn_status_listener(
    _app: AppHandle,
    tray: TrayIcon,
    mut status_rx: watch::Receiver<SyncTrayStatus>,
) -> async_runtime::JoinHandle<()> {
    async_runtime::spawn(async move {
        // Apply the initial state once before waiting for changes.
        apply_status(&tray, &status_rx.borrow());
        while status_rx.changed().await.is_ok() {
            apply_status(&tray, &status_rx.borrow());
        }
    })
}

fn apply_status(tray: &TrayIcon, status: &SyncTrayStatus) {
    // The tray image is a single Quilt logo; the per-mode signal lives
    // in the tooltip so the icon stays recognisable across themes.
    let label = match status.mode {
        TrayMode::Idle => "Quilt — idle",
        TrayMode::Syncing => "Quilt — syncing",
        TrayMode::Paused => "Quilt — paused",
        TrayMode::Error => "Quilt — error",
    };
    let mut tooltip = label.to_string();
    if status.pending_changes > 0 {
        tooltip.push_str(&format!(
            "\n{} package(s) with local changes",
            status.pending_changes,
        ));
    }
    if let Some(error) = status.error.as_deref() {
        tooltip.push_str(&format!("\n{error}"));
    }
    let _ = tray.set_tooltip(Some(&tooltip));
}

fn install_close_handler(app: &AppHandle) -> Result<(), TrayError> {
    let Some(window) = app.get_webview_window("main") else {
        return Ok(());
    };
    let handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            let close_to_tray = {
                let settings = handle.state::<SharedAutosyncSettings>();
                let snapshot = async_runtime::block_on(settings.read());
                snapshot.close_to_tray
            };
            if !close_to_tray {
                return;
            }
            api.prevent_close();
            if let Some(window) = handle.get_webview_window("main") {
                let _ = window.hide();
            }
            #[cfg(target_os = "macos")]
            {
                let _ = handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }
            let mode_handle = handle.clone();
            async_runtime::spawn(async move {
                let mode = mode_handle.state::<SharedWindowMode>();
                *mode.write().await = WindowMode::Closed;
            });
        }
    });
    Ok(())
}
