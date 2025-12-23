use std::borrow::Cow;

use askama::Template;
use rust_i18n::t;

use crate::app::AppAssets;
use crate::app::Globals;
use crate::debug_tools;
use crate::error::Error;
use crate::model::QuiltModel;
use crate::quilt;
use crate::quilt::lineage::Change;
use crate::routes;
use crate::ui::btn;
use crate::ui::crumbs;
use crate::ui::entry;
use crate::ui::layout::Layout;
use crate::ui::strip_whitespace;
use crate::ui::Icon;

#[derive(Default)]
struct ViewCommitMessage {
    value: String,
    error: Option<quilt::Error>,
}

#[derive(Default)]
struct ViewCommitUserMeta {
    value: String,
    error: Option<String>,
}

#[derive(Default, Debug)]
struct ViewCommitWorkflow {
    error: Option<quilt::Error>,
    id: Option<String>,
    url: Option<url::Url>,
}

pub struct ViewCommit {
    entries_modified: Vec<entry::ViewEntry>,
    entries_rest: Vec<entry::ViewEntry>,
    globals: Globals,
    message: ViewCommitMessage,
    origin: url::Url,
    uri: quilt::uri::S3PackageUri,
    user_meta: ViewCommitUserMeta,
    workflow: Option<ViewCommitWorkflow>,
}

#[derive(Template)]
#[template(
    source = r#"class="js-workflow-value input" id="workflow" name="workflow""#,
    ext = "txt"
)]
struct TmplWorkflowFormId {}

#[derive(Template)]
#[template(source = r#"class="js-workflow-null""#, ext = "txt")]
struct TmplWorkflowFormNull {}

#[derive(Template)]
#[template(
    source = r#"
    {% if let Some(pre) = pre %}{{ pre }}{% endif %}
    <a class="js-open-in-web-browser link" data-url="{{ href }}">{{ link }}</a>
    "#,
    ext = "txt"
)]
struct TmplWorkflowLink<'a> {
    pre: Option<Cow<'a, str>>,
    link: &'a str,
    href: String,
}

#[derive(Template)]
#[template(path = "./components/commit-workflow.html")]
struct TmplWorkflow<'a> {
    js: Option<TmplWorkflowFormId>,
    value: Option<String>,
    disabled: bool,
    error: Option<quilt::Error>,
    null_checked: bool,
    null_disabled: bool,
    null_js: Option<TmplWorkflowFormNull>,
    link: Option<TmplWorkflowLink<'a>>,
}

#[derive(Template)]
#[template(path = "./pages/commit.html")]
struct TmplPageCommit<'a> {
    message: ViewCommitMessage,
    uri: quilt::uri::S3PackageUri,
    user_meta: ViewCommitUserMeta,
    entries: Vec<Vec<entry::TmplEntry<'a>>>,
    workflow: TmplWorkflow<'a>,
    layout: Layout<'a>,
}

fn parse_commit_workflow(
    header: &quilt::manifest::Header,
    host: &quilt::uri::Host,
) -> Result<Option<ViewCommitWorkflow>, Error> {
    match header.get_workflow() {
        Ok(Some(value)) => Ok(Some(ViewCommitWorkflow {
            error: None,
            url: Some(value.config.display_for_host(host)?),
            id: value.id.map(|id| id.id),
        })),
        Ok(None) => Ok(None),
        Err(err) => Ok(Some(ViewCommitWorkflow {
            error: Some(err),
            url: None,
            id: None,
        })),
    }
}

fn parse_commit_user_meta(header: &quilt::manifest::Header) -> ViewCommitUserMeta {
    match header.get_user_meta() {
        Ok(Some(meta)) => match serde_json::to_string(&meta) {
            Ok(value) => ViewCommitUserMeta { value, error: None },
            Err(_) => ViewCommitUserMeta {
                value: "".to_string(),
                error: Some("Failed to stringify meta".to_string()),
            },
        },
        Ok(None) => ViewCommitUserMeta {
            value: "".to_string(),
            error: None,
        },
        Err(err) => ViewCommitUserMeta {
            value: "".to_string(),
            error: Some(err.to_string()),
        },
    }
}

fn parse_commit_message(header: &quilt::manifest::Header) -> ViewCommitMessage {
    match header.get_message() {
        Ok(Some(value)) => ViewCommitMessage { value, error: None },
        Ok(None) => ViewCommitMessage {
            value: "".to_string(),
            error: None,
        },
        Err(error) => ViewCommitMessage {
            value: "".to_string(),
            error: Some(error),
        },
    }
}

impl ViewCommit {
    pub async fn create(
        model: &impl QuiltModel,
        app: &impl AppAssets,
        tracing: &crate::telemetry::Telemetry,
        namespace: &quilt::uri::Namespace,
    ) -> Result<ViewCommit, Error> {
        let installed_package = model
            .get_installed_package(namespace)
            .await?
            .unwrap_or_else(|| panic!("Package not found, {}", &namespace));
        let status = model
            .get_installed_package_status(&installed_package, None)
            .await?;

        let lineage = model
            .get_installed_package_lineage(&installed_package)
            .await?;

        let origin_host = debug_tools::try_remote_origin_host(&lineage.remote)?;

        tracing.add_host(&origin_host);

        let remote_manifest = model.browse_remote_manifest(&lineage.remote).await?;
        let mut entries_modified = Vec::new();
        for (filename, change) in &status.changes {
            let mut uri = quilt::uri::S3PackageUri::from(&lineage.remote);
            uri.path = Some(filename.clone());
            let origin = uri.display_for_host(&origin_host)?;

            entries_modified.push(entry::ViewEntry {
                filename: filename.clone(),
                size: match &change {
                    Change::Added(r) | Change::Modified(r) | Change::Removed(r) => r.size,
                },
                status: entry::EntryStatus::from(change),
                uri,
                origin,
            });
            if entries_modified.len() > 1000 {
                break;
            }
        }

        let manifest_entries = model
            .get_installed_package_records(&installed_package)
            .await?;
        let mut entries_rest = Vec::new();
        for (filename, row) in manifest_entries {
            let uri = quilt::uri::S3PackageUri::from(&lineage.remote);
            if status.changes.contains_key(&filename) {
                continue;
            }
            let entry_uri = quilt::uri::S3PackageUri {
                path: Some(filename.clone()),
                ..uri.clone()
            };
            let origin = entry_uri.display_for_host(&origin_host)?;
            entries_rest.push(entry::ViewEntry {
                filename: filename.clone(),
                size: row.size,
                status: if lineage.paths.contains_key(&filename) {
                    entry::EntryStatus::Pristine
                } else {
                    entry::EntryStatus::Remote
                },
                uri,
                origin,
            })
        }

        // TODO: just use remote_manifest?
        let uri = quilt::uri::S3PackageUri::from(&lineage.remote);

        Ok(ViewCommit {
            globals: app.globals(),
            entries_modified,
            entries_rest,
            message: parse_commit_message(&remote_manifest.header),
            user_meta: parse_commit_user_meta(&remote_manifest.header),
            uri: uri.clone(),
            origin: uri.display_for_host(&origin_host)?,
            workflow: parse_commit_workflow(&remote_manifest.header, &origin_host)?,
        })
    }

    pub fn render(self) -> Result<String, Error> {
        Ok(strip_whitespace(TmplPageCommit::from(self).render()?))
    }
}

impl<'a> TmplPageCommit<'a> {
    fn primary_button() -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_icon(Icon::Done)
            .set_js(btn::JsSelector::PackagesCommit)
            .set_data("form", "#form")
            .set_label(t!("commit.submit"))
            .set_color(btn::Color::Primary)
            .set_size(btn::Size::Large)
            .set_direction(btn::Direction::RightToLeft)
    }

    fn workflow(
        value: Option<ViewCommitWorkflow>,
        origin: &url::Url,
        uri: &quilt::uri::S3PackageUri,
    ) -> TmplWorkflow<'a> {
        match value {
            Some(w) => {
                let is_null = w.id.is_none();
                TmplWorkflow {
                    disabled: is_null,
                    error: w.error,
                    js: Some(TmplWorkflowFormId {}),
                    null_checked: is_null,
                    null_disabled: false,
                    null_js: Some(TmplWorkflowFormNull {}),
                    value: w.id,
                    link: w.url.map(|url| TmplWorkflowLink {
                        pre: Some(t!("commit.workflow_source")),
                        link: ".quilt/workflows/config.yaml",
                        href: url.to_string(),
                    }),
                }
            }
            None => TmplWorkflow {
                disabled: true,
                error: None,
                js: None,
                null_checked: true,
                null_disabled: true,
                null_js: None,
                value: Some("Workflow not available".to_string()),
                link: origin.host().map(|host| TmplWorkflowLink {
                    pre: None,
                    link: "Create workflows in .quilt/workflows.yaml",
                    href: format!(
                        "https://{}/b/{}/tree/.config/workflows.yml",
                        host, uri.bucket
                    ),
                }),
            },
        }
    }

    fn breadcrumbs(uri: &quilt::uri::S3PackageUri) -> crumbs::TmplBreadcrumbs<'a> {
        crumbs::TmplBreadcrumbs {
            list: vec![
                crumbs::Link::home(),
                crumbs::Link::create(
                    routes::Paths::InstalledPackage(uri.namespace.to_owned()),
                    uri.namespace.to_string(),
                ),
                crumbs::Current::create(t!("breadcrumbs.commit")),
            ],
        }
    }

    fn entries(
        modified: Vec<entry::ViewEntry>,
        rest: Vec<entry::ViewEntry>,
    ) -> Vec<Vec<entry::TmplEntry<'a>>> {
        let mut entries_modified = Vec::new();
        let mut entries_rest = Vec::new();
        for entry in modified {
            entries_modified.push(entry::TmplEntry::from(entry));
        }
        for entry in rest {
            entries_rest.push(entry::TmplEntry::from(entry));
        }
        let mut entries = Vec::new();
        if !entries_modified.is_empty() {
            entries.push(entries_modified);
        }
        if !entries_rest.is_empty() {
            entries.push(entries_rest);
        }
        entries
    }

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
}

impl From<ViewCommit> for TmplPageCommit<'_> {
    fn from(view: ViewCommit) -> Self {
        TmplPageCommit {
            entries: Self::entries(view.entries_modified, view.entries_rest),
            message: view.message,
            user_meta: view.user_meta,
            workflow: Self::workflow(view.workflow, &view.origin, &view.uri),
            layout: Layout::builder(view.globals)
                .set_actions(Self::actions(&view.origin, &view.uri))
                .set_primary_action(Self::primary_button())
                .set_breadcrumbs(Self::breadcrumbs(&view.uri))
                .set_uri(Some(view.uri.clone())),
            uri: view.uri,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::app::mocks as app_mocks;

    #[test]
    fn test_view() -> Result<(), Error> {
        let app = app_mocks::create();
        let html = ViewCommit {
            globals: app.globals(),
            entries_modified: vec![],
            entries_rest: vec![],
            uri: quilt::uri::S3PackageUri::try_from("quilt+s3://C#package=A/B")?,
            origin: url::Url::parse("https://test.quilt.dev/C/packages/A/B")?,
            message: ViewCommitMessage {
                value: "".to_string(),
                error: None,
            },
            user_meta: ViewCommitUserMeta {
                value: "".to_string(),
                error: None,
            },
            workflow: Some(ViewCommitWorkflow {
                error: None,
                id: None,
                url: None,
            }),
        }
        .render()?;

        let has_namespace_input = html.contains(
            r#"<input class="input" id="namespace" name="namespace" value="A/B" readonly />"#,
        );
        let has_message_input = html.contains(
            r#"<input class="input" id="message" name="message" placeholder="" required />"#,
        );
        let has_metadata_input = html.contains(r#"<textarea class="textarea" id="metadata" name="metadata" placeholder="{ \"key\": \"value\" }" ></textarea>"#);
        let has_submit_button = html.contains(r##"<button class="qui-button primary js-packages-commit large" data-form="#form" type="button"><span>Commit</span><img class="qui-icon" src="/assets/img/icons/done.svg" /></button>"##);

        assert!(has_namespace_input);
        assert!(has_message_input);
        assert!(has_metadata_input);
        assert!(has_submit_button);
        Ok(())
    }

    #[test]
    fn test_workflow_with_value() -> Result<(), Error> {
        let app = app_mocks::create();
        let workflow_id = "test-workflow-123";
        let workflow_url = url::Url::parse("https://test.quilt.dev/workflows/config.yaml")?;

        let html = ViewCommit {
            globals: app.globals(),
            entries_modified: vec![],
            entries_rest: vec![],
            uri: quilt::uri::S3PackageUri::try_from("quilt+s3://C#package=A/B")?,
            origin: url::Url::parse("https://test.quilt.dev/C/packages/A/B")?,
            message: ViewCommitMessage {
                value: "".to_string(),
                error: None,
            },
            user_meta: ViewCommitUserMeta {
                value: "".to_string(),
                error: None,
            },
            workflow: Some(ViewCommitWorkflow {
                error: None,
                id: Some(workflow_id.to_string()),
                url: Some(workflow_url.clone()),
            }),
        }
        .render()?;

        assert!(html.contains(r#"<div class="workflow"> <p class="field"> <label class="label" for="workflow" >Workflow ID</label> <input class="js-workflow-value input" id="workflow" name="workflow" value="test-workflow-123" /></p>"#));
        assert!(html.contains(r#"<div class="workflow-null"> <input id="workflow-null" type="checkbox" class="js-workflow-null" /> <label class="workflow-null-label" for="workflow-null">No workflow</label> </div>"#));

        Ok(())
    }

    #[test]
    fn test_workflow_null_checked() -> Result<(), Error> {
        let app = app_mocks::create();
        let workflow_url = url::Url::parse("https://test.quilt.dev/workflows/config.yaml")?;

        let html = ViewCommit {
            globals: app.globals(),
            entries_modified: vec![],
            entries_rest: vec![],
            uri: quilt::uri::S3PackageUri::try_from("quilt+s3://C#package=A/B")?,
            origin: url::Url::parse("https://test.quilt.dev/C/packages/A/B")?,
            message: ViewCommitMessage {
                value: "".to_string(),
                error: None,
            },
            user_meta: ViewCommitUserMeta {
                value: "".to_string(),
                error: None,
            },
            workflow: Some(ViewCommitWorkflow {
                error: None,
                id: None,
                url: Some(workflow_url.clone()),
            }),
        }
        .render()?;

        assert!(html.contains(r#"<div class="workflow"> <p class="field"> <label class="label" for="workflow" >Workflow ID</label> <input class="js-workflow-value input" id="workflow" name="workflow" disabled /></p>"#));
        assert!(html.contains(r#"<div class="workflow-null"> <input id="workflow-null" type="checkbox" class="js-workflow-null" checked /> <label class="workflow-null-label" for="workflow-null">No workflow</label> </div>"#));
        assert!(html.contains(&format!(r#"data-url="{workflow_url}""#)));

        Ok(())
    }

    #[test]
    fn test_workflow_not_available() -> Result<(), Error> {
        let app = app_mocks::create();

        let html = ViewCommit {
            globals: app.globals(),
            entries_modified: vec![],
            entries_rest: vec![],
            uri: quilt::uri::S3PackageUri::try_from("quilt+s3://C#package=A/B")?,
            origin: url::Url::parse("https://test.quilt.dev/C/packages/A/B")?,
            message: ViewCommitMessage::default(),
            user_meta: ViewCommitUserMeta::default(),
            workflow: None,
        }
        .render()?;

        assert!(html.contains(r#"<div class="workflow"> <p class="field"> <label class="label" for="workflow" >Workflow ID</label> <input class="input" value="Workflow not available" disabled /></p>"#));
        assert!(html.contains(r#"<div class="workflow-null"> <input id="workflow-null" type="checkbox" checked disabled /> <label class="workflow-null-label" for="workflow-null">No workflow</label> </div>"#));

        Ok(())
    }
}
