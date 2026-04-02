use askama::Template;
use rust_i18n::t;

use crate::debug_tools;
use crate::error::Error;
use crate::model::QuiltModel;
use crate::quilt;
use crate::quilt::lineage::UpstreamState;
use crate::quilt::uri::Namespace;
use crate::routes::EntriesFilter;
use crate::routes::Paths;
use crate::telemetry::prelude::*;
use crate::ui::btn;
use crate::ui::crumbs;
use crate::ui::layout::Layout;
use crate::ui::Icon;

#[derive(Debug)]
struct InstalledPackage {
    namespace: Namespace,
    origin: Option<url::Url>,
    origin_host: Option<quilt::uri::Host>,
    remote: Option<quilt::uri::ManifestUri>,
    status: UpstreamState,
}

#[derive(Debug)]
pub struct ViewInstalledPackagesList {
    installed_packages_list: Vec<InstalledPackage>,
}

#[derive(Template)]
#[template(path = "./components/installed-package-item.html")]
struct TmplInstalledPackage<'a> {
    button_commit: Option<btn::TmplButton<'a>>,
    button_error_action: Option<btn::TmplButton<'a>>,
    button_merge: Option<btn::TmplButton<'a>>,
    button_open_local: btn::TmplButton<'a>,
    button_open_remote: Option<btn::TmplButton<'a>>,
    button_sync: Option<btn::TmplButton<'a>>,
    button_uninstall: btn::TmplButton<'a>,
    is_error: bool,
    namespace: quilt::uri::Namespace,
    remote: Option<quilt::uri::ManifestUri>,
}

impl From<InstalledPackage> for TmplInstalledPackage<'_> {
    fn from(value: InstalledPackage) -> Self {
        let InstalledPackage {
            namespace,
            origin,
            origin_host,
            remote,
            status,
        } = value;
        let is_error = status == UpstreamState::Error;
        TmplInstalledPackage {
            button_commit: if !is_error {
                Some(Self::button_commit(&namespace))
            } else {
                None
            },
            button_error_action: Self::button_error_action(
                &namespace,
                &status,
                origin_host.as_ref(),
            ),
            button_merge: Self::button_merge(&namespace, &status),
            button_open_local: Self::button_open_local(&namespace),
            button_open_remote: Self::button_open_remote(origin.as_ref(), &status),
            button_sync: Self::button_sync(&namespace, &status, origin.is_some()),
            button_uninstall: Self::button_uninstall(&namespace),
            is_error,
            namespace,
            remote,
        }
    }
}

impl<'a> TmplInstalledPackage<'a> {
    fn button_open_local(namespace: &Namespace) -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_data("namespace", namespace.to_string())
            .set_icon(Icon::FolderOpen)
            .set_js(btn::JsSelector::OpenInFileBrowser)
            .set_label(t!("buttons.open_package_in_file_browser"))
            .set_size(btn::Size::Small)
    }

    fn button_open_remote(
        origin: Option<&url::Url>,
        status: &UpstreamState,
    ) -> Option<btn::TmplButton<'a>> {
        let origin = origin?;
        let btn = btn::TmplButton::builder()
            .set_data("url", origin.to_string())
            .set_icon(Icon::OpenInBrowser)
            .set_js(btn::JsSelector::OpenInWebBrowser)
            .set_label(t!("buttons.open_package_in_catalog"))
            .set_size(btn::Size::Small);
        Some(if *status == UpstreamState::Local {
            btn.set_disabled()
        } else {
            btn
        })
    }

    fn button_commit(namespace: &Namespace) -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_icon(Icon::Commit)
            .set_label(t!("buttons.commit_package"))
            .set_size(btn::Size::Small)
            .set_href(Paths::Commit(namespace.clone(), EntriesFilter::default()))
    }

    fn button_uninstall(namespace: &Namespace) -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_data("namespace", namespace.to_string())
            .set_icon(Icon::Block)
            .set_js(btn::JsSelector::PackagesUninstall)
            .set_label(t!("buttons.uninstall_package"))
            .set_size(btn::Size::Small)
    }

    fn button_sync(
        namespace: &Namespace,
        status: &UpstreamState,
        has_origin: bool,
    ) -> Option<btn::TmplButton<'a>> {
        match status {
            UpstreamState::Ahead => Some(
                btn::TmplButton::builder()
                    .set_data("namespace", namespace.to_string())
                    .set_icon(Icon::CloudUpload)
                    .set_js(btn::JsSelector::PackagesPush)
                    .set_label(t!("buttons.push_package"))
                    .set_color(btn::Color::Primary)
                    .set_size(btn::Size::Small),
            ),
            UpstreamState::Behind => Some(
                btn::TmplButton::builder()
                    .set_data("namespace", namespace.to_string())
                    .set_icon(Icon::CloudDownload)
                    .set_js(btn::JsSelector::PackagesPull)
                    .set_label(t!("buttons.pull_package"))
                    .set_color(btn::Color::Primary)
                    .set_size(btn::Size::Small),
            ),
            // Remote configured but never pushed — show Push as the natural next step
            UpstreamState::Local if has_origin => Some(
                btn::TmplButton::builder()
                    .set_data("namespace", namespace.to_string())
                    .set_icon(Icon::CloudUpload)
                    .set_js(btn::JsSelector::PackagesPush)
                    .set_label(t!("buttons.push_package"))
                    .set_color(btn::Color::Primary)
                    .set_size(btn::Size::Small),
            ),
            _ => None,
        }
    }

    fn button_error_action(
        namespace: &Namespace,
        status: &UpstreamState,
        origin_host: Option<&quilt::uri::Host>,
    ) -> Option<btn::TmplButton<'a>> {
        match status {
            // Local without origin — offer to set remote
            UpstreamState::Local if origin_host.is_none() => Some(
                btn::TmplButton::builder()
                    .set_data("namespace", namespace.to_string())
                    .set_icon(Icon::CloudUpload)
                    .set_js(btn::JsSelector::SetRemote)
                    .set_label(t!("buttons.set_remote"))
                    .set_size(btn::Size::Small),
            ),
            // Local with origin — no error action needed (Push is in button_sync)
            UpstreamState::Local => None,
            _ => match origin_host {
                None => Some(
                    btn::TmplButton::builder()
                        .set_data("namespace", namespace.to_string())
                        .set_icon(Icon::Warning)
                        .set_js(btn::JsSelector::SetOrigin)
                        .set_label(t!("buttons.set_origin"))
                        .set_color(btn::Color::Warning)
                        .set_size(btn::Size::Small),
                ),
                Some(host) => match status {
                    UpstreamState::Error => Some(
                        btn::TmplButton::builder()
                            .set_icon(Icon::Warning)
                            .set_label(t!("error.login"))
                            .set_color(btn::Color::Warning)
                            .set_size(btn::Size::Small)
                            .set_href(Paths::Login(
                                host.clone(),
                                Paths::InstalledPackagesList.to_string(),
                            )),
                    ),
                    _ => None,
                },
            },
        }
    }

    fn button_merge(namespace: &Namespace, status: &UpstreamState) -> Option<btn::TmplButton<'a>> {
        match status {
            UpstreamState::Diverged => Some(
                btn::TmplButton::builder()
                    .set_data("namespace", namespace.to_string())
                    .set_icon(Icon::Merge)
                    .set_label(t!("buttons.merge_package"))
                    .set_color(btn::Color::Primary)
                    .set_size(btn::Size::Small)
                    .set_href(Paths::Merge(namespace.clone())),
            ),
            _ => None,
        }
    }
}

#[derive(Template)]
#[template(path = "./pages/installed-packages-list.html")]
pub struct TmplPageInstalledPackagesList<'a> {
    list: Vec<TmplInstalledPackage<'a>>,
    layout: Layout<'a>,
}
impl<'a> TmplPageInstalledPackagesList<'a> {
    fn breadcrumbs() -> crumbs::TmplBreadcrumbs<'a> {
        crumbs::TmplBreadcrumbs {
            list: vec![crumbs::Current::create(t!(
                "breadcrumbs.installed_packages_list"
            ))],
        }
    }

    fn button_create() -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_icon(Icon::Add)
            .set_js(btn::JsSelector::CreatePackage)
            .set_label(t!("buttons.create_package"))
            .set_size(btn::Size::Small)
    }
}

impl ViewInstalledPackagesList {
    pub async fn create(
        model: &impl QuiltModel,
        tracing: &crate::telemetry::Telemetry,
    ) -> Result<ViewInstalledPackagesList, Error> {
        let list = model.get_installed_packages_list().await?;
        let mut installed_packages_list = Vec::new();
        for installed_package in list {
            match Self::load_package(model, tracing, &installed_package).await {
                Ok(pkg) => installed_packages_list.push(pkg),
                Err(err) => {
                    warn!(
                        "Failed to load package {}: {err}",
                        installed_package.namespace
                    );
                }
            }
        }
        debug!("Packages list is {:?}", installed_packages_list);
        Ok(ViewInstalledPackagesList {
            installed_packages_list,
        })
    }

    async fn load_package(
        model: &impl QuiltModel,
        tracing: &crate::telemetry::Telemetry,
        installed_package: &quilt::InstalledPackage,
    ) -> Result<InstalledPackage, Error> {
        let lineage = model
            .get_installed_package_lineage(installed_package)
            .await?;

        let remote_uri = match lineage.remote_uri.as_ref() {
            Some(uri) => uri,
            None => {
                return Ok(InstalledPackage {
                    namespace: installed_package.namespace.clone(),
                    origin: None,
                    origin_host: None,
                    remote: None,
                    status: UpstreamState::Local,
                });
            }
        };

        if remote_uri.origin.is_none() {
            return Ok(InstalledPackage {
                namespace: installed_package.namespace.clone(),
                origin: None,
                origin_host: None,
                remote: Some(remote_uri.clone()),
                status: UpstreamState::Error,
            });
        }

        let origin_host = debug_tools::try_remote_origin_host(remote_uri)?;
        tracing.add_host(&origin_host);
        let uri = quilt::uri::S3PackageUri::from(remote_uri);
        let origin_url = uri.display_for_host(&origin_host)?;
        let status = match model
            .get_installed_package_status(installed_package, None)
            .await
        {
            Ok(status) => status.upstream_state,
            Err(err) => {
                warn!(
                    "Failed to get status for {}: {err}",
                    installed_package.namespace
                );
                UpstreamState::Error
            }
        };

        Ok(InstalledPackage {
            namespace: installed_package.namespace.clone(),
            origin: Some(origin_url),
            origin_host: Some(origin_host),
            remote: Some(remote_uri.clone()),
            status,
        })
    }

    pub fn render(self) -> Result<String, Error> {
        let tmpl = TmplPageInstalledPackagesList::from(self);
        Ok(tmpl.render()?)
    }
}

impl From<ViewInstalledPackagesList> for TmplPageInstalledPackagesList<'_> {
    fn from(view: ViewInstalledPackagesList) -> Self {
        let mut list = Vec::new();

        let consumed_list = view.installed_packages_list.into_iter();
        for item in consumed_list {
            list.push(TmplInstalledPackage::from(item));
        }

        TmplPageInstalledPackagesList {
            layout: Layout::builder()
                .set_breadcrumbs(Self::breadcrumbs())
                .set_actions(vec![Self::button_create()]),
            list,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use quilt::uri::ManifestUri;
    use quilt::uri::S3PackageUri;

    use crate::Result;

    fn create_test_package(namespace: &str, status: UpstreamState) -> Result<InstalledPackage> {
        Ok(InstalledPackage {
            namespace: namespace.try_into().unwrap(),
            origin: Some(url::Url::parse("https://test.quilt.dev").unwrap()),
            origin_host: Some("test.quilt.dev".parse().unwrap()),
            remote: Some(ManifestUri::try_from(S3PackageUri::try_from(
                format!("quilt+s3://test#package={namespace}@abcdef").as_str(),
            )?)?),
            status,
        })
    }

    #[test]
    fn test_button_rendering_for_different_statuses() -> Result {
        // Create packages with different statuses
        let packages = vec![
            create_test_package("test/ahead", UpstreamState::Ahead)?,
            create_test_package("test/behind", UpstreamState::Behind)?,
            create_test_package("test/diverged", UpstreamState::Diverged)?,
            create_test_package("test/uptodate", UpstreamState::UpToDate)?,
        ];

        // Create view with test packages
        let view = ViewInstalledPackagesList {
            installed_packages_list: packages,
        };

        // Render the view
        let html = view.render()?;

        // Check for push button in the package with Ahead status
        assert!(html.contains(r#"data-namespace="test/ahead""#));
        assert!(html.contains(r#"js-packages-push"#));
        assert!(html.contains(r#"cloud_upload"#));

        // Check for pull button in the package with Behind status
        assert!(html.contains(r#"data-namespace="test/behind""#));
        assert!(html.contains(r#"js-packages-pull"#));
        assert!(html.contains(r#"cloud_download"#));

        // Check for merge button in the package with Diverged status
        assert!(html.contains(r#"data-namespace="test/diverged""#));
        assert!(html.contains(r#"merge"#));
        assert!(html.contains(r#"href="merge.html#namespace=test/diverged""#));

        // Check that all packages have common buttons
        for namespace in [
            "test/ahead",
            "test/behind",
            "test/diverged",
            "test/uptodate",
        ] {
            // Check for commit button
            assert!(html.contains(&format!(r#"href="commit.html#namespace={namespace}""#)));

            // Check for open local button
            assert!(html.contains(&format!(r#"data-namespace="{namespace}""#)));
            assert!(html.contains(r#"js-open-in-file-browser"#));

            // Check for uninstall button
            assert!(html.contains(r#"js-packages-uninstall"#));
        }

        Ok(())
    }

    #[test]
    fn test_sync_button_rendering() -> Result<()> {
        // Test that sync buttons are only rendered for appropriate statuses
        let ahead_package = create_test_package("test/ahead", UpstreamState::Ahead)?;
        let behind_package = create_test_package("test/behind", UpstreamState::Behind)?;
        let uptodate_package = create_test_package("test/uptodate", UpstreamState::UpToDate)?;

        // Check that Ahead status has push button
        let ahead_tmpl = TmplInstalledPackage::from(ahead_package);
        let ahead_html = ahead_tmpl.to_string();
        assert!(ahead_html.contains(r#"js-packages-push"#));
        assert!(!ahead_html.contains(r#"js-packages-pull"#));

        // Check that Behind status has pull button
        let behind_tmpl = TmplInstalledPackage::from(behind_package);
        let behind_html = behind_tmpl.to_string();
        assert!(behind_html.contains(r#"js-packages-pull"#));
        assert!(!behind_html.contains(r#"js-packages-push"#));

        // Check that UpToDate status has neither push nor pull buttons
        let uptodate_tmpl = TmplInstalledPackage::from(uptodate_package);
        let uptodate_html = uptodate_tmpl.to_string();
        assert!(!uptodate_html.contains(r#"js-packages-pull"#));
        assert!(!uptodate_html.contains(r#"js-packages-push"#));

        Ok(())
    }

    #[test]
    fn test_merge_button_rendering() -> Result<()> {
        // Test that merge button is only rendered for Diverged status
        let diverged_package = create_test_package("test/diverged", UpstreamState::Diverged)?;
        let uptodate_package = create_test_package("test/uptodate", UpstreamState::UpToDate)?;

        // Check that Diverged status has merge button
        let diverged_tmpl = TmplInstalledPackage::from(diverged_package);
        let diverged_html = diverged_tmpl.to_string();
        assert!(diverged_html.contains(r#"merge"#));
        assert!(diverged_html.contains(r#"href="merge.html#namespace=test/diverged""#));

        // Check that UpToDate status doesn't have merge button
        let uptodate_tmpl = TmplInstalledPackage::from(uptodate_package);
        let uptodate_html = uptodate_tmpl.to_string();
        assert!(!uptodate_html.contains(r#"merge"#));
        assert!(!uptodate_html.contains(r#"href="merge.html"#));

        Ok(())
    }

    fn create_local_package_with_origin(namespace: &str) -> Result<InstalledPackage> {
        Ok(InstalledPackage {
            namespace: namespace.try_into().unwrap(),
            origin: Some(url::Url::parse("https://test.quilt.dev").unwrap()),
            origin_host: Some("test.quilt.dev".parse().unwrap()),
            remote: Some(ManifestUri {
                origin: Some("test.quilt.dev".parse().unwrap()),
                bucket: "test".to_string(),
                namespace: namespace.try_into().unwrap(),
                hash: String::new(),
            }),
            status: UpstreamState::Local,
        })
    }

    fn create_test_package_no_origin(namespace: &str) -> Result<InstalledPackage> {
        Ok(InstalledPackage {
            namespace: namespace.try_into().unwrap(),
            origin: None,
            origin_host: None,
            remote: Some(ManifestUri::try_from(S3PackageUri::try_from(
                format!("quilt+s3://test#package={namespace}@abcdef").as_str(),
            )?)?),
            status: UpstreamState::Error,
        })
    }

    #[test]
    fn test_error_status_hides_sync_and_merge_buttons() -> Result<()> {
        let error_package = create_test_package("test/error", UpstreamState::Error)?;
        let error_tmpl = TmplInstalledPackage::from(error_package);
        let error_html = error_tmpl.to_string();

        // Error status should show red border
        assert!(error_html.contains("qui-installed-package-item error"));

        // Error status should not show push, pull, or merge buttons
        assert!(!error_html.contains(r#"js-packages-push"#));
        assert!(!error_html.contains(r#"js-packages-pull"#));
        assert!(!error_html.contains(r#"href="merge.html"#));

        // Should still show "Open in Catalog" since origin is valid
        assert!(error_html.contains(r#"js-open-in-web-browser"#));

        // Should show Login button for StatusFailed
        assert!(error_html.contains(
            r#"href="login.html#host=test.quilt.dev&#38;back=installed-packages-list.html""#
        ));

        // Should not show commit button for error-state packages
        assert!(!error_html.contains(r#"href="commit.html"#));

        // But should still have common buttons
        assert!(error_html.contains(r#"js-open-in-file-browser"#));
        assert!(error_html.contains(r#"js-packages-uninstall"#));

        Ok(())
    }

    #[test]
    fn test_no_origin_shows_set_origin_button() -> Result<()> {
        let no_origin_package = create_test_package_no_origin("test/noorigin")?;
        let no_origin_tmpl = TmplInstalledPackage::from(no_origin_package);
        let no_origin_html = no_origin_tmpl.to_string();

        // Should show error styling
        assert!(no_origin_html.contains("qui-installed-package-item error"));

        // Should show "Set origin" button
        assert!(no_origin_html.contains(r#"js-set-origin"#));
        assert!(no_origin_html.contains(r#"data-namespace="test/noorigin""#));

        // Should NOT show Login button
        assert!(!no_origin_html.contains(r#"href="login.html"#));

        Ok(())
    }

    #[test]
    fn test_local_with_origin_shows_push_and_disabled_catalog() -> Result<()> {
        let package = create_local_package_with_origin("test/localpush")?;
        let tmpl = TmplInstalledPackage::from(package);
        let html = tmpl.to_string();

        // Should NOT show error styling
        assert!(!html.contains("qui-installed-package-item error"));

        // Should show Push button (natural next step after set_remote)
        assert!(html.contains(r#"js-packages-push"#));

        // Should show "Open in Catalog" button but disabled
        let btn_start = html
            .find("js-open-in-web-browser")
            .expect("catalog button not found");
        let btn_end = html[btn_start..]
            .find("</button>")
            .expect("closing </button> not found");
        let catalog_btn = &html[btn_start..btn_start + btn_end];
        assert!(catalog_btn.contains("disabled"));

        // Should NOT show "Set Remote" button (remote is already set)
        assert!(!html.contains(r#"js-set-remote"#));

        // Should show commit button
        assert!(html.contains(r#"href="commit.html"#));

        Ok(())
    }
}
