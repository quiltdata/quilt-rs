use tauri::AppHandle;
use tauri::Manager;
use tauri_plugin_deep_link::DeepLinkExt;
use url::Url;

use crate::quilt;
use crate::routes;
use crate::telemetry::prelude::*;
use crate::Error;
use crate::Result;

pub fn get_remote_package_url(current_url: &Url, uri_str: &str) -> Result<Url> {
    let uri: quilt::uri::S3PackageUri = uri_str.parse()?;
    Ok(routes::from_url(
        routes::Paths::RemotePackage(uri.clone()),
        current_url.to_owned(),
    ))
}

pub fn navigate_to_url<R: tauri::Runtime>(app_handle: &AppHandle<R>, url: Url) -> Result {
    match app_handle.get_webview_window("main") {
        Some(win) => {
            win.navigate(url)?;
            win.set_focus()?;
            Ok(())
        }
        None => Err(Error::Window),
    }
}

/// Navigate to a Quilt package URI by parsing it and redirecting to the package page
pub fn navigate_to_uri<R: tauri::Runtime>(app_handle: &AppHandle<R>, uri_str: &str) -> Result {
    let win = app_handle.get_webview_window("main").ok_or(Error::Window)?;
    let current_url = win.url()?;
    let redirect_url = get_remote_package_url(&current_url, uri_str)?;
    navigate_to_url(app_handle, redirect_url)
}

async fn wait_for_main_window<R: tauri::Runtime>(app_handle: &AppHandle<R>) -> Result<()> {
    let mut attempts = 0;
    const MAX_ATTEMPTS: u32 = 10;
    const RETRY_DELAY_MS: u64 = 200;

    while attempts < MAX_ATTEMPTS {
        if let Some(window) = app_handle.get_webview_window("main") {
            // Additional check: ensure window is visible and ready
            if let Ok(true) = window.is_visible() {
                return Ok(());
            }
        }

        attempts += 1;
        tokio::time::sleep(tokio::time::Duration::from_millis(RETRY_DELAY_MS)).await;
    }

    Err(Error::Window)
}

fn handle_deep_link_navigation<R: tauri::Runtime>(app_handle: &AppHandle<R>, urls: Vec<Url>) {
    let Some(first_url) = urls.first() else {
        return;
    };

    let url_str = first_url.to_string();
    let handle = app_handle.clone();

    tauri::async_runtime::spawn(async move {
        match wait_for_main_window(&handle).await {
            Ok(()) => {
                if let Err(err) = navigate_to_uri(&handle, &url_str) {
                    error!("Failed to handle deep link '{}': {}", url_str, err);
                }
            }
            Err(_) => {
                error!(
                    "Failed to find ready main window, deep link lost: {}",
                    url_str
                );
            }
        }
    });
}

pub fn setup_deep_link_handler(app_handle: &AppHandle) {
    let deep_link = app_handle.deep_link();

    let handle_for_runtime = app_handle.clone();
    deep_link.on_open_url(move |event| {
        let urls = event.urls();
        debug!("Processing runtime deep link: {:?}", urls);
        handle_deep_link_navigation(&handle_for_runtime, urls);
    });

    if let Ok(Some(urls)) = deep_link.get_current() {
        debug!("Processing startup deep link: {:?}", urls);
        handle_deep_link_navigation(app_handle, urls);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_remote_package_url() {
        let current_url = Url::parse("http://localhost:3000/").unwrap();
        let uri_str = "quilt+s3://bucket#package=foo/bar@hash";

        let result = get_remote_package_url(&current_url, uri_str);
        assert!(result.is_ok());
    }

    #[test]
    fn test_get_remote_package_url_invalid_uri() {
        let current_url = Url::parse("http://localhost:3000/").unwrap();
        let uri_str = "invalid-uri";

        let result = get_remote_package_url(&current_url, uri_str);
        assert!(result.is_err());
    }
}
