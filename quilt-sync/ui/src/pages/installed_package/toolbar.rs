use leptos::prelude::*;
use leptos_router::NavigateOptions;
use leptos_router::hooks::use_navigate;

use crate::commands::{self, InstalledPackageData};
use crate::components::buttons;
use crate::components::{Notification, ToolbarActions};
use crate::util;

/// The remote button's label, and whether it is drawn as a warning. A bucket
/// is a configured remote, with or without a catalog: the computer's AWS
/// credentials reach a bucket alone.
fn remote_button(
    uri: Option<&quilt_uri::S3PackageUri>,
    remote_locked: bool,
    status: &str,
) -> (&'static str, bool) {
    if !util::has_bucket(uri) {
        ("Set remote", true)
    } else if remote_locked {
        ("Show remote", false)
    } else {
        ("Change remote", status == "error")
    }
}

// ── Toolbar actions (rendered into Layout's toolbar) ──

pub(super) fn build_toolbar_actions(
    data: &InstalledPackageData,
    notification: RwSignal<Option<Notification>>,
    ui_locked: RwSignal<bool>,
    show_set_remote_popup: RwSignal<bool>,
) -> ToolbarActions {
    let namespace = data.namespace.clone();
    let uri = data.uri.clone();
    let origin_url = data.uri.as_ref().and_then(util::catalog_url);
    let has_catalog = origin_url.is_some();
    let catalog_disabled = data.status == "local";

    let (remote_label, remote_warning) =
        remote_button(data.uri.as_ref(), data.remote_locked, &data.status);

    ToolbarActions::new(move || {
        let navigate = use_navigate();

        let ns_for_open = namespace.clone();
        let uri_for_open = uri.clone();
        let on_open_file_browser = move |_| {
            let ns = ns_for_open.clone();
            let uri = uri_for_open.clone();
            leptos::task::spawn_local(async move {
                match commands::open_in_file_browser(ns.to_string(), uri).await {
                    Ok(msg) => notification.set(Some(Notification::Success(msg))),
                    Err(e) => notification.set(Some(Notification::Error(e))),
                }
            });
        };

        let url_for_catalog = origin_url.clone();
        let on_open_catalog = move |_| {
            if let Some(url) = url_for_catalog.clone() {
                leptos::task::spawn_local(async move {
                    let _ = commands::open_in_web_browser(url).await;
                });
            }
        };

        let ns_for_uninstall = namespace.clone();
        let uri_for_uninstall = uri.clone();
        let on_uninstall = move |_| {
            let ns = ns_for_uninstall.clone();
            let uri = uri_for_uninstall.clone();
            let navigate = navigate.clone();
            ui_locked.set(true);
            leptos::task::spawn_local(async move {
                // v1 keeps the downloaded files: only the v2 confirmation
                // offers to delete them.
                match commands::package_uninstall(ns.to_string(), uri, false).await {
                    Ok(msg) => {
                        notification.set(Some(Notification::Success(msg)));
                        navigate("/", NavigateOptions::default());
                    }
                    Err(e) => {
                        ui_locked.set(false);
                        notification.set(Some(Notification::Error(e)));
                    }
                }
            });
        };

        let on_set_remote = move |_| show_set_remote_popup.set(true);

        view! {
            <li>
                <buttons::OpenInFileBrowser on_click=on_open_file_browser />
            </li>
            {if has_catalog {
                view! {
                    <li>
                        <buttons::OpenInCatalog on_click=on_open_catalog disabled=catalog_disabled />
                    </li>
                }.into_any()
            } else {
                ().into_any()
            }}
            <li>
                <buttons::SetRemote
                    on_click=on_set_remote
                    warning=remote_warning
                    label=remote_label
                />
            </li>
            <li>
                <buttons::Remove on_click=on_uninstall />
            </li>
        }
        .into_any()
    })
}

#[cfg(test)]
mod tests {
    use super::remote_button;

    fn uri(s: &str) -> quilt_uri::S3PackageUri {
        s.parse().unwrap()
    }

    /// No bucket: the package needs one, so the button says so as a warning.
    #[test]
    fn no_bucket_asks_for_a_remote() {
        assert_eq!(remote_button(None, false, "local"), ("Set remote", true));
    }

    /// A bucket without a catalog is configured, the same as one with a
    /// catalog: no warning.
    #[test]
    fn a_bucket_without_a_catalog_is_configured() {
        let bare = uri("quilt+s3://data-bucket#package=team/survey");
        assert_eq!(
            remote_button(Some(&bare), false, "local"),
            ("Change remote", false)
        );
        assert_eq!(
            remote_button(Some(&bare), true, "up_to_date"),
            ("Show remote", false)
        );
        let with_catalog = uri("quilt+s3://data-bucket#package=team/survey&catalog=quilt.test");
        assert_eq!(
            remote_button(Some(&with_catalog), true, "up_to_date"),
            remote_button(Some(&bare), true, "up_to_date"),
        );
    }

    /// An unpublished remote that fails to read is flagged, so the reader
    /// checks what they typed.
    #[test]
    fn an_unpublished_remote_in_error_is_flagged() {
        let bare = uri("quilt+s3://data-bucket#package=team/survey");
        assert_eq!(
            remote_button(Some(&bare), false, "error"),
            ("Change remote", true)
        );
    }
}
