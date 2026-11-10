use std::fmt::Write;
use std::sync::Arc;
use std::time::Duration;

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

use crate::engine::telemetry::prelude::*;

use super::MAIN;
use super::show_window;
use crate::engine::action_needed::Entry;
use crate::engine::autopull::SyncTrayStatus;
use crate::engine::autopull::TrayMode;
use crate::engine::quit::QuitAction;
use crate::engine::services::Services;

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
const MENU_ID_BROWSER: &str = "tray-browser";
const MENU_ID_QUIT: &str = "tray-quit";

impl TrayController {
    /// `link` is the engine's page with its token, for *Open in browser*.
    pub fn install(
        app: &AppHandle,
        status_rx: watch::Receiver<SyncTrayStatus>,
        action_needed_rx: watch::Receiver<Vec<Entry>>,
        link: String,
    ) -> Result<Self, TrayError> {
        let open_item = MenuItem::with_id(app, MENU_ID_OPEN, "Open Quilt", true, None::<&str>)?;
        let browser_item =
            MenuItem::with_id(app, MENU_ID_BROWSER, "Open in browser", true, None::<&str>)?;
        let quit_item = MenuItem::with_id(app, MENU_ID_QUIT, "Quit", true, None::<&str>)?;
        let menu = Menu::with_items(app, &[&open_item, &browser_item, &quit_item])?;

        let icon = load_tray_icon(app)?;
        let icons = Icons {
            dotted: with_dot(&icon),
            plain: icon.clone(),
        };
        let tray = TrayIconBuilder::new()
            .icon(icon)
            .menu(&menu)
            .tooltip("Quilt — idle")
            .on_menu_event(move |app, event| match event.id().as_ref() {
                MENU_ID_OPEN => show_window(app),
                MENU_ID_BROWSER => {
                    if let Err(err) = opener::open_browser(&link) {
                        warn!("failed to open the browser: {err}");
                    }
                }
                MENU_ID_QUIT => request_quit(app),
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
                    show_window(tray.app_handle());
                }
            })
            .build(app)?;

        let listener = spawn_status_listener(tray.clone(), icons, status_rx, action_needed_rx);
        let _ = open_item;
        let _ = browser_item;
        let _ = quit_item;

        install_close_handler(app);

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

/// The tray's two images: the logo, and the logo with the action-needed dot.
struct Icons {
    plain: Image<'static>,
    dotted: Image<'static>,
}

fn spawn_status_listener(
    tray: TrayIcon,
    icons: Icons,
    mut status_rx: watch::Receiver<SyncTrayStatus>,
    mut action_needed_rx: watch::Receiver<Vec<Entry>>,
) -> async_runtime::JoinHandle<()> {
    async_runtime::spawn(async move {
        let mut dotted = false;
        loop {
            let need_you = action_needed_rx.borrow_and_update().len();
            if dotted != (need_you > 0) {
                dotted = need_you > 0;
                let icon = if dotted { &icons.dotted } else { &icons.plain };
                let _ = tray.set_icon(Some(icon.clone()));
            }
            let tooltip = tooltip(need_you, &status_rx.borrow_and_update());
            let _ = tray.set_tooltip(Some(&tooltip));
            tokio::select! {
                changed = status_rx.changed() => if changed.is_err() { break },
                changed = action_needed_rx.changed() => if changed.is_err() { break },
            }
        }
    })
}

/// The logo with a red dot in its top-right corner, cut out from the logo by
/// a clear ring so it reads apart from the logo's own orange. Red is the
/// tone the main page's queue gives these causes.
fn with_dot(icon: &Image<'_>) -> Image<'static> {
    const RED: [u8; 4] = [0xE5, 0x48, 0x4D, 0xFF];
    let (width, height) = (icon.width(), icon.height());
    let side = f64::from(width.min(height));
    let radius = side * 0.25;
    let gap = side * 0.07;
    let (cx, cy) = (f64::from(width) - radius, radius);
    let mut rgba = icon.rgba().to_vec();
    for (at, pixel) in rgba.as_chunks_mut::<4>().0.iter_mut().enumerate() {
        let at = u32::try_from(at).unwrap_or(u32::MAX);
        let (x, y) = (f64::from(at % width) + 0.5, f64::from(at / width) + 0.5);
        let distance = (x - cx).hypot(y - cy);
        if distance <= radius {
            pixel.copy_from_slice(&RED);
        } else if distance <= radius + gap {
            pixel.copy_from_slice(&[0; 4]);
        }
    }
    Image::new_owned(rgba, width, height)
}

/// The tooltip: how many things need action, then the sync status.
fn tooltip(need_you: usize, status: &SyncTrayStatus) -> String {
    // The tray image is a single Quilt logo; the per-mode signal lives
    // in the tooltip so the icon stays recognisable across themes.
    let label = match status.mode {
        TrayMode::Idle => "Quilt — idle",
        TrayMode::Syncing => "Quilt — syncing",
        TrayMode::Paused => "Quilt — paused",
        TrayMode::Error => "Quilt — error",
    };
    let mut tooltip = String::new();
    match need_you {
        0 => {}
        1 => tooltip.push_str("1 needs action\n"),
        n => {
            let _ = writeln!(tooltip, "{n} need action");
        }
    }
    tooltip.push_str(label);
    if status.pending_changes > 0 {
        let _ = write!(
            tooltip,
            "\n{} package(s) with local changes",
            status.pending_changes,
        );
    }
    if let Some(error) = status.error.as_deref() {
        let _ = write!(tooltip, "\n{error}");
    }
    tooltip
}

/// Event the window listens for to raise the prompt; a browser tab doesn't.
/// Kept in lockstep with the UI's `listen(...)` call.
const QUIT_PROMPT_EVENT: &str = "quit-prompt";

/// How long a raised prompt has to report itself on screen before the quit is
/// treated as unanswerable. Long enough for a webview that is merely busy,
/// short enough that a click on Quit never looks ignored.
const PROMPT_GRACE: Duration = Duration::from_millis(1500);

/// Route a quit request through the gate: exit when nothing is being written,
/// otherwise ask and let the answer decide.
fn request_quit(app: &AppHandle) {
    let Some(services) = app.try_state::<Arc<Services>>() else {
        // Nothing to ask with. A quit that cannot be questioned proceeds.
        app.exit(0);
        return;
    };
    let applying = services.watcher.apply_in_progress();
    match services.quit_gate.request(applying) {
        QuitAction::Exit => app.exit(0),
        QuitAction::AlreadyPrompting => {}
        QuitAction::Prompt(generation) => prompt_before_quit(app, &services, generation),
    }
}

/// Raise the window and ask. The tray path usually has it hidden, and an
/// overlay needs something to sit on.
///
/// The failure direction is deliberate: if the ask cannot be delivered, or
/// nothing reports it on screen within [`PROMPT_GRACE`], the quit goes
/// through. A Quit that appears to do nothing is a worse outcome than an
/// interrupted apply — the apply is what the reconcile is being made to
/// survive, whereas an app that will not close has no remedy at all.
fn prompt_before_quit(app: &AppHandle, services: &Services, generation: u64) {
    show_window(app);
    if !services.events.has_listeners() {
        warn!("no window or tab to raise the quit prompt in, quitting");
        app.exit(0);
        return;
    }
    services.events.emit(QUIT_PROMPT_EVENT, generation);
    let handle = app.clone();
    async_runtime::spawn(async move {
        tokio::time::sleep(PROMPT_GRACE).await;
        if handle
            .try_state::<Arc<Services>>()
            .is_some_and(|services| services.quit_gate.unanswerable(generation))
        {
            warn!("quit prompt never reported itself on screen, quitting");
            handle.exit(0);
        }
    });
}

fn install_close_handler(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN) else {
        return;
    };
    let handle = app.clone();
    window.on_window_event(move |event| {
        if let WindowEvent::CloseRequested { api, .. } = event {
            let close_to_tray = {
                let services = handle.state::<Arc<Services>>();
                let snapshot = async_runtime::block_on(services.autosync_settings.read());
                snapshot.close_to_tray
            };
            if !close_to_tray {
                // Closing IS quitting in this configuration, so it gets the
                // same question. Refuse the close either way and let the
                // answer decide: `prevent_close` then exit is the only way to
                // ask, since the event itself cannot be awaited.
                api.prevent_close();
                request_quit(&handle);
                return;
            }
            api.prevent_close();
            if let Some(window) = handle.get_webview_window(MAIN) {
                let _ = window.hide();
            }
            #[cfg(target_os = "macos")]
            {
                let _ = handle.set_activation_policy(tauri::ActivationPolicy::Accessory);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_tooltip_leads_with_what_needs_action() {
        let status = SyncTrayStatus::default();
        assert_eq!(tooltip(0, &status), "Quilt — idle");
        assert_eq!(tooltip(2, &status), "2 need action\nQuilt — idle");
    }

    #[test]
    fn the_dot_sits_in_the_top_right_corner_apart_from_the_logo() {
        let icon = Image::new_owned(vec![0xFF; 100 * 100 * 4], 100, 100);
        let dotted = with_dot(&icon);
        let pixel = |x: usize, y: usize| &dotted.rgba()[(y * 100 + x) * 4..][..4];
        assert_eq!(pixel(80, 20), [0xE5, 0x48, 0x4D, 0xFF], "the dot");
        assert_eq!(pixel(46, 24), [0, 0, 0, 0], "the clear ring");
        assert_eq!(pixel(10, 90), [0xFF; 4], "the logo");
    }
}
