//! Commands that need the desktop app: the folder picker, the clipboard, the
//! updater and *Quit anyway*. Each asks [`Desktop`] for the part only the
//! desktop can do.

use std::fs;
use std::path::PathBuf;

use quilt_uri::S3PackageUri;
use serde::Serialize;

use crate::engine::Error;
use crate::engine::desktop::Desktop;
use crate::engine::error::DesktopError;
use crate::engine::notify::Notify;
use crate::engine::quilt;
use crate::engine::services::Services;
use crate::engine::telemetry::{MixpanelEvent, prelude::*};

async fn open_directory_picker_command(desktop: &dyn Desktop) -> Result<PathBuf, Error> {
    let home_dir = dirs::home_dir().ok_or_else(|| Error::General("No home directory".into()))?;

    let canonical_home = home_dir.join(quilt::DEFAULT_HOME_DIR_NAME);
    let canonical_home_already_exists = canonical_home.exists();

    if !canonical_home_already_exists && let Err(e) = fs::create_dir_all(&canonical_home) {
        return Err(Error::from(e));
    }

    let result = match desktop.pick_folder(&canonical_home).await {
        Ok(Some(path)) => {
            debug!("Successfully selected {}", path.display());
            Ok(path)
        }
        Ok(None) => {
            debug!("User cancelled directory selection");
            Err(Error::Desktop(DesktopError::UserCancelled))
        }
        Err(err) => Err(err),
    };

    // Cleanup logic: remove temporary canonical directory if needed
    let should_delete_canonical = match &result {
        Ok(path) => path != &canonical_home,
        Err(_) => true,
    };

    if !canonical_home_already_exists
        && should_delete_canonical
        && let Err(e) = fs::remove_dir(&canonical_home)
    {
        error!("Failed to remove temporary QuiltSync directory: {}", e);
    }

    result
}

pub async fn open_directory_picker(s: &Services, desktop: &dyn Desktop) -> Result<PathBuf, String> {
    // Deliberately reported before the outcome, and the only site that is. The
    // criterion is whether the event's name stays true when the call returns
    // `Err`, and here it does: this command models the user *cancelling* as an
    // error, so waiting for success would silently narrow "the picker was shown"
    // to "a folder was chosen". Those are different questions and this event
    // answers the first. If the second is ever wanted, it is a second event.
    s.telemetry.track(MixpanelEvent::DirectoryPickerOpened);

    match open_directory_picker_command(desktop).await {
        Ok(path) => Ok(path),
        Err(err) => {
            error!("Failed to open directory picker: {}", err);
            Err(err.to_string())
        }
    }
}

/// Put `text` on the system clipboard.
///
/// The text, not the thing it describes: the caller decides what a copy *means*
/// — the `quilt+s3` address of a file or of a package — and this command decides
/// nothing about it. `uri` is for the event alone and never reaches the
/// clipboard, which is why the two are separate arguments rather than one that
/// gets rendered here: a `uri` with no path is a package's, and counts apart
/// from files.
///
/// Through the backend rather than `navigator.clipboard`, so it does not depend
/// on a webview setting that differs per platform: on Linux and Windows the DOM
/// API is gated behind wry's `clipboard` attribute, which Tauri leaves off and
/// exposes only on a builder this app does not call — its window comes from
/// `tauri.conf.json`.
#[allow(
    clippy::unused_async,
    reason = "every plain command is async, so the dispatch treats them alike"
)]
pub async fn copy_to_clipboard(
    s: &Services,
    desktop: &dyn Desktop,
    text: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    let msg_init = format!("Copying to clipboard: {text}");
    let msg_ok = "Successfully copied to clipboard".to_string();
    let msg_err = |err: &Error| format!("Failed to copy to clipboard: {err}");

    Notify::new(msg_init)
        .on_success(&s.telemetry, super::copied_event(uri.as_ref()))
        .map(desktop.copy_to_clipboard(&text), msg_ok, msg_err)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UpdateInfo {
    pub version: String,
}

pub async fn check_for_update(
    _s: &Services,
    desktop: &dyn Desktop,
) -> Result<Option<UpdateInfo>, String> {
    // The UI ignores a failed check so users are not nagged on every launch,
    // which leaves the log as the only place the failure shows.
    desktop
        .check_for_update()
        .await
        .map(|version| version.map(|version| UpdateInfo { version }))
        .inspect_err(|e| warn!("update check failed: {e}"))
}

pub async fn download_and_install_update(
    _s: &Services,
    desktop: &dyn Desktop,
) -> Result<(), String> {
    desktop.install_update().await
}

/// *Quit anyway* — the user accepts interrupting the apply.
#[allow(
    clippy::unused_async,
    reason = "every plain command is async, so the dispatch treats them alike"
)]
pub async fn quit_confirm(_s: &Services, desktop: &dyn Desktop) -> Result<(), String> {
    desktop.quit();
    Ok(())
}
