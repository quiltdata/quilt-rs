use askama::Template;
use rust_i18n::t;

use crate::debug_tools;
use crate::error::Error;
use crate::model::QuiltModel;
use crate::quilt;
use crate::routes;
use crate::ui::btn;
use crate::ui::crumbs;
use crate::ui::layout::Layout;
use crate::ui::Icon;
use quilt::uri::S3PackageUri;

pub struct ViewMerge {
    origin: url::Url,
    uri: quilt::uri::S3PackageUri,
}

#[derive(Template)]
#[template(path = "./pages/merge.html")]
pub struct TmplPageMerge<'a> {
    certify_button: btn::TmplButton<'a>,
    reset_button: btn::TmplButton<'a>,
    layout: Layout<'a>,
}

impl<'a> TmplPageMerge<'a> {
    fn actions(origin: &url::Url, uri: &quilt::uri::S3PackageUri) -> Vec<btn::TmplButton<'a>> {
        vec![
            btn::TmplButton::builder()
                .set_data("namespace", uri.namespace.to_string())
                .set_icon(Icon::FolderOpen)
                .set_js(btn::JsSelector::OpenInFileBrowser)
                .set_label(t!("buttons.open_package_in_file_browser"))
                .set_size(btn::Size::Small),
            btn::TmplButton::builder()
                .set_data("url", origin.to_string())
                .set_icon(Icon::OpenInBrowser)
                .set_js(btn::JsSelector::OpenInWebBrowser)
                .set_label(t!("buttons.open_package_in_catalog"))
                .set_size(btn::Size::Small),
            btn::TmplButton::builder()
                .set_data("namespace", uri.namespace.to_string())
                .set_icon(Icon::Block)
                .set_js(btn::JsSelector::PackagesUninstall)
                .set_label(t!("buttons.uninstall_package"))
                .set_size(btn::Size::Small),
        ]
    }

    fn breadcrumbs(uri: &S3PackageUri) -> crumbs::TmplBreadcrumbs<'a> {
        crumbs::TmplBreadcrumbs {
            list: vec![
                crumbs::Link::home(),
                crumbs::Link::create(
                    routes::Paths::InstalledPackage(
                        uri.namespace.to_owned(),
                        routes::EntriesFilter::for_installed_package(),
                    ),
                    uri.namespace.to_string(),
                ),
                crumbs::Current::create(t!("breadcrumbs.merge", s => uri.namespace)),
            ],
        }
    }

    fn certify_button(uri: &S3PackageUri) -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_label(t!("merge.certify"))
            .set_js(btn::JsSelector::CertifyLatest)
            .set_data("namespace", uri.namespace.to_string())
    }

    fn reset_button(uri: &S3PackageUri) -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_label(t!("merge.reset"))
            .set_js(btn::JsSelector::ResetLocal)
            .set_data("namespace", uri.namespace.to_string())
    }
}

impl From<ViewMerge> for TmplPageMerge<'_> {
    fn from(view: ViewMerge) -> Self {
        TmplPageMerge {
            certify_button: Self::certify_button(&view.uri),
            reset_button: Self::reset_button(&view.uri),
            layout: Layout::builder()
                .set_breadcrumbs(Self::breadcrumbs(&view.uri))
                .set_actions(Self::actions(&view.origin, &view.uri)),
        }
    }
}

impl ViewMerge {
    pub async fn create(
        model: &impl QuiltModel,
        tracing: &crate::telemetry::Telemetry,
        namespace: &quilt::uri::Namespace,
    ) -> Result<ViewMerge, Error> {
        let installed_package = model
            .get_installed_package(namespace)
            .await?
            .ok_or_else(|| {
                Error::from(quilt::InstallPackageError::NotInstalled(
                    namespace.to_owned(),
                ))
            })?;
        let lineage = model
            .get_installed_package_lineage(&installed_package)
            .await?;
        let remote_uri = lineage.remote()?;
        let uri = quilt::uri::S3PackageUri::from(remote_uri);
        let origin_host = debug_tools::try_remote_origin_host(remote_uri)?;

        tracing.add_host(&origin_host);

        Ok(ViewMerge {
            uri: uri.clone(),
            origin: uri.display_for_host(&origin_host)?,
        })
    }

    pub fn render(self) -> Result<String, Error> {
        Ok(TmplPageMerge::from(self)
            .render()?
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Result;

    #[test]
    fn test_merge_page_rendering() -> Result<()> {
        // Create a test namespace and URI
        let uri = quilt::uri::S3PackageUri::try_from("quilt+s3://test#package=test/package")?;
        let origin = url::Url::parse("https://test.quilt.dev/b/test/packages/test/package")?;

        // Create the view
        let view = ViewMerge {
            uri: uri.clone(),
            origin,
        };

        // Render the view to HTML
        let html = view.render()?;

        // Check for certify button with exact HTML
        assert!(html.contains(r#"<button class="qui-button js-packages-certify-latest" data-namespace="test/package" type="button"><span>Certify latest</span></button>"#));

        // Check for reset button with exact HTML
        assert!(html.contains(r#"<button class="qui-button js-packages-reset-local" data-namespace="test/package" type="button"><span>Reset local</span></button>"#));

        // Check for action buttons with exact HTML
        assert!(html.contains(r#"<button class="qui-button js-open-in-file-browser small" data-namespace="test/package" type="button"><img class="qui-icon" src="/assets/img/icons/folder_open.svg" /><span>Open</span></button>"#));
        assert!(html.contains(r#"<button class="qui-button js-open-in-web-browser small" data-url="https://test.quilt.dev/b/test/packages/test/package" type="button"><img class="qui-icon" src="/assets/img/icons/open_in_browser.svg" /><span>Open in Catalog</span></button>"#));
        assert!(html.contains(r#"<button class="qui-button js-packages-uninstall small" data-namespace="test/package" type="button"><img class="qui-icon" src="/assets/img/icons/block.svg" /><span>Remove</span></button>"#));

        // Check for breadcrumbs
        assert!(html.contains(
            r#"<a class="qui-breadcrumb-link" title="test/package" href="/installed-package?namespace=test/package&#38;filter=unmodified">test/package</a>"#
        ));
        assert!(
            html.contains(r#"<strong class="qui-breadcrumb-current" title="Merge">Merge</strong>"#)
        );

        Ok(())
    }
}
