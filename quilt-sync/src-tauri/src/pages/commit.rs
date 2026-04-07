use std::borrow::Cow;
use std::collections::HashMap;

use askama::Template;
use rust_i18n::t;

use crate::debug_tools;
use crate::error::Error;
use crate::model::QuiltModel;
use crate::quilt;
use crate::quilt::lineage::Change;
use crate::quilt::lineage::ChangeSet;
use crate::routes;
use crate::routes::EntriesFilter;
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
    entries_ignored: Vec<entry::ViewEntry>,
    filter: EntriesFilter,
    message: ViewCommitMessage,
    origin: Option<url::Url>,
    uri: quilt::uri::S3PackageUri,
    user_meta: ViewCommitUserMeta,
    workflow: Option<ViewCommitWorkflow>,
    ignored_count: usize,
    unmodified_count: usize,
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
    filter_unmodified_checked: bool,
    filter_unmodified_href: String,
    filter_ignored_checked: bool,
    filter_ignored_href: String,
    ignored_count: usize,
    unmodified_count: usize,
}

fn parse_commit_workflow(
    header: &quilt::manifest::ManifestHeader,
    host: &quilt::uri::Host,
) -> Result<Option<ViewCommitWorkflow>, Error> {
    match &header.workflow {
        Some(value) => Ok(Some(ViewCommitWorkflow {
            error: None,
            url: Some(value.config.display_for_host(host)?),
            id: value.id.as_ref().map(|id| id.id.clone()),
        })),
        None => Ok(None),
    }
}

fn parse_commit_user_meta(header: &quilt::manifest::ManifestHeader) -> ViewCommitUserMeta {
    match &header.user_meta {
        Some(meta) => match serde_json::to_string(&meta) {
            Ok(value) => ViewCommitUserMeta { value, error: None },
            Err(_) => ViewCommitUserMeta {
                value: "".to_string(),
                error: Some("Failed to stringify meta".to_string()),
            },
        },
        None => ViewCommitUserMeta {
            value: "".to_string(),
            error: None,
        },
    }
}

fn file_names(paths: &[&std::path::PathBuf]) -> String {
    paths
        .iter()
        .map(|p| {
            p.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| p.to_string_lossy().into_owned())
        })
        .collect::<Vec<_>>()
        .join(", ")
}

fn change_count(n: usize, verb: &str) -> String {
    if n == 1 {
        format!("{verb} 1 file")
    } else {
        format!("{verb} {n} files")
    }
}

/// Generates a concise, human-readable commit message from the set of changed files.
///
/// For three or fewer total changes, individual file names are listed.
/// For larger changesets, counts are used instead.
fn generate_commit_message(changes: &ChangeSet) -> ViewCommitMessage {
    let added: Vec<_> = changes
        .iter()
        .filter(|(_, c)| matches!(c, Change::Added(_)))
        .map(|(p, _)| p)
        .collect();
    let modified: Vec<_> = changes
        .iter()
        .filter(|(_, c)| matches!(c, Change::Modified(_)))
        .map(|(p, _)| p)
        .collect();
    let removed: Vec<_> = changes
        .iter()
        .filter(|(_, c)| matches!(c, Change::Removed(_)))
        .map(|(p, _)| p)
        .collect();

    let total = changes.len();
    if total == 0 {
        return ViewCommitMessage::default();
    }

    let mut parts = Vec::new();
    if total <= 3 {
        if !added.is_empty() {
            parts.push(format!("Add {}", file_names(&added)));
        }
        if !modified.is_empty() {
            parts.push(format!("Update {}", file_names(&modified)));
        }
        if !removed.is_empty() {
            parts.push(format!("Remove {}", file_names(&removed)));
        }
    } else {
        if !added.is_empty() {
            parts.push(change_count(added.len(), "Add"));
        }
        if !modified.is_empty() {
            parts.push(change_count(modified.len(), "Update"));
        }
        if !removed.is_empty() {
            parts.push(change_count(removed.len(), "Remove"));
        }
    }
    ViewCommitMessage {
        value: parts.join(", "),
        error: None,
    }
}

impl ViewCommit {
    pub async fn create(
        model: &impl QuiltModel,
        tracing: &crate::telemetry::Telemetry,
        namespace: &quilt::uri::Namespace,
        filter: &EntriesFilter,
    ) -> Result<ViewCommit, Error> {
        let installed_package = model
            .get_installed_package(namespace)
            .await?
            .ok_or_else(|| Error::Quilt(quilt::Error::Install(quilt::InstallError::NotInstalled(namespace.clone()))))?;
        let status = model
            .get_installed_package_status(&installed_package, None)
            .await?;

        let lineage = model
            .get_installed_package_lineage(&installed_package)
            .await?;

        let (uri, origin_host) =
            debug_tools::resolve_uri_and_host(lineage.remote_uri.as_ref(), namespace);
        if let Some(host) = &origin_host {
            tracing.add_host(host);
        }

        // Build lookup maps for junky files
        let junky_map: HashMap<_, _> = status
            .junky_changes
            .iter()
            .map(|(p, pat)| (p.clone(), pat.clone()))
            .collect();

        let mut entries_modified = Vec::new();
        for (filename, change) in &status.changes {
            let entry_uri = quilt::uri::S3PackageUri {
                path: Some(filename.clone()),
                ..uri.clone()
            };
            let origin = match &origin_host {
                Some(host) => Some(entry_uri.display_for_host(host)?),
                None => None,
            };

            entries_modified.push(entry::ViewEntry {
                filename: filename.clone(),
                size: match &change {
                    Change::Added(r) | Change::Modified(r) | Change::Removed(r) => r.size,
                },
                status: entry::EntryStatus::from(change),
                uri: entry_uri,
                origin,
                junky_pattern: junky_map.get(filename).cloned(),
                ignored_by: None,
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
            if status.changes.contains_key(&filename) {
                continue;
            }
            let entry_uri = quilt::uri::S3PackageUri {
                path: Some(filename.clone()),
                ..uri.clone()
            };
            let origin = match &origin_host {
                Some(host) => Some(entry_uri.display_for_host(host)?),
                None => None,
            };
            entries_rest.push(entry::ViewEntry {
                filename: filename.clone(),
                size: row.size,
                status: if lineage.paths.contains_key(&filename) {
                    entry::EntryStatus::Pristine
                } else {
                    entry::EntryStatus::Remote
                },
                uri: entry_uri,
                origin,
                junky_pattern: None,
                ignored_by: None,
            })
        }

        // Add ignored files
        let mut entries_ignored = Vec::new();
        for (filename, pattern, size) in &status.ignored_files {
            let entry_uri = quilt::uri::S3PackageUri {
                path: Some(filename.clone()),
                ..uri.clone()
            };
            entries_ignored.push(entry::ViewEntry {
                filename: filename.clone(),
                size: *size,
                status: entry::EntryStatus::Pristine,
                uri: entry_uri,
                origin: None,
                junky_pattern: None,
                ignored_by: Some(pattern.clone()),
            });
        }

        let ignored_count = entries_ignored.len();
        let unmodified_count = entries_rest.len();

        // Apply filter: skip entries that are hidden by the current filter
        let entries_rest = if filter.unmodified {
            entries_rest
        } else {
            Vec::new()
        };
        let entries_ignored = if filter.ignored {
            entries_ignored
        } else {
            Vec::new()
        };

        let origin = match &origin_host {
            Some(host) => Some(uri.display_for_host(host)?),
            None => None,
        };

        // Load remote manifest for user_meta and workflow (only if remote has a manifest hash)
        let (user_meta, workflow) = match lineage.remote_uri.as_ref().filter(|r| !r.hash.is_empty())
        {
            Some(remote_uri) => {
                let remote_manifest = model.browse_remote_manifest(remote_uri).await?;
                let user_meta = parse_commit_user_meta(&remote_manifest.header);
                let workflow = origin_host
                    .as_ref()
                    .and_then(|host| parse_commit_workflow(&remote_manifest.header, host).ok())
                    .flatten();
                (user_meta, workflow)
            }
            None => (ViewCommitUserMeta::default(), None),
        };

        Ok(ViewCommit {
            entries_modified,
            entries_rest,
            entries_ignored,
            filter: filter.clone(),
            message: generate_commit_message(&status.changes),
            user_meta,
            uri,
            origin,
            workflow,
            ignored_count,
            unmodified_count,
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
        origin: Option<&url::Url>,
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
                link: origin.and_then(|o| o.host()).map(|host| TmplWorkflowLink {
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
                    routes::Paths::InstalledPackage(
                        uri.namespace.to_owned(),
                        routes::EntriesFilter::for_installed_package(),
                    ),
                    uri.namespace.to_string(),
                ),
                crumbs::Current::create(t!("breadcrumbs.commit")),
            ],
        }
    }

    fn entries(
        modified: Vec<entry::ViewEntry>,
        rest: Vec<entry::ViewEntry>,
        ignored: Vec<entry::ViewEntry>,
    ) -> Vec<Vec<entry::TmplEntry<'a>>> {
        let mut entries_modified = Vec::new();
        let mut entries_rest = Vec::new();
        let mut entries_ignored = Vec::new();
        for entry in modified {
            entries_modified.push(entry::TmplEntry::from(entry));
        }
        for entry in rest {
            entries_rest.push(entry::TmplEntry::from(entry));
        }
        for entry in ignored {
            entries_ignored.push(entry::TmplEntry::from(entry));
        }
        let mut entries = Vec::new();
        if !entries_modified.is_empty() {
            entries.push(entries_modified);
        }
        if !entries_rest.is_empty() {
            entries.push(entries_rest);
        }
        if !entries_ignored.is_empty() {
            entries.push(entries_ignored);
        }
        entries
    }

    fn actions(
        origin: Option<&url::Url>,
        uri: &quilt::uri::S3PackageUri,
    ) -> Vec<btn::TmplButton<'a>> {
        let mut actions = vec![btn::TmplButton::builder()
            .set_data("namespace", uri.namespace.to_string())
            .set_icon(Icon::FolderOpen)
            .set_js(btn::JsSelector::OpenInFileBrowser)
            .set_label(t!("buttons.open_package_in_file_browser"))
            .set_size(btn::Size::Small)];
        if let Some(origin) = origin {
            actions.push(
                btn::TmplButton::builder()
                    .set_data("url", origin.to_string())
                    .set_icon(Icon::OpenInBrowser)
                    .set_js(btn::JsSelector::OpenInWebBrowser)
                    .set_label(t!("buttons.open_package_in_catalog"))
                    .set_size(btn::Size::Small),
            );
        }
        actions.push(
            btn::TmplButton::builder()
                .set_data("namespace", uri.namespace.to_string())
                .set_icon(Icon::Block)
                .set_js(btn::JsSelector::PackagesUninstall)
                .set_label(t!("buttons.uninstall_package"))
                .set_size(btn::Size::Small),
        );
        actions
    }
}

impl From<ViewCommit> for TmplPageCommit<'_> {
    fn from(view: ViewCommit) -> Self {
        let toggled_unmodified = view.filter.toggle_unmodified();
        let toggled_ignored = view.filter.toggle_ignored();
        let filter_unmodified_href =
            routes::Paths::Commit(view.uri.namespace.clone(), toggled_unmodified).to_string();
        let filter_ignored_href =
            routes::Paths::Commit(view.uri.namespace.clone(), toggled_ignored).to_string();

        TmplPageCommit {
            entries: Self::entries(
                view.entries_modified,
                view.entries_rest,
                view.entries_ignored,
            ),
            message: view.message,
            user_meta: view.user_meta,
            workflow: Self::workflow(view.workflow, view.origin.as_ref(), &view.uri),
            layout: Layout::builder()
                .set_actions(Self::actions(view.origin.as_ref(), &view.uri))
                .set_primary_action(Self::primary_button())
                .set_breadcrumbs(Self::breadcrumbs(&view.uri))
                .set_uri(Some(view.uri.clone())),
            uri: view.uri.clone(),
            filter_unmodified_checked: view.filter.unmodified,
            filter_unmodified_href,
            filter_ignored_checked: view.filter.ignored,
            filter_ignored_href,
            ignored_count: view.ignored_count,
            unmodified_count: view.unmodified_count,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use super::*;

    use crate::quilt::manifest::ManifestRow;

    #[test]
    fn test_view() -> Result<(), Error> {
        let html = ViewCommit {
            entries_modified: vec![],
            entries_rest: vec![],
            entries_ignored: vec![],
            filter: EntriesFilter::default(),
            uri: quilt::uri::S3PackageUri::try_from("quilt+s3://C#package=A/B")?,
            origin: Some(url::Url::parse("https://test.quilt.dev/C/packages/A/B")?),
            message: ViewCommitMessage {
                value: "".to_string(),
                error: None,
            },
            user_meta: ViewCommitUserMeta {
                value: "".to_string(),
                error: None,
            },
            ignored_count: 0,
            unmodified_count: 0,
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
            r#"<input autofocus class="input" id="message" name="message" value="" required />"#,
        );
        let has_metadata_input = html.contains(r#"<textarea class="textarea" id="metadata" name="metadata" placeholder="{ \"key\": \"value\" }" ></textarea>"#);
        let has_submit_button = html.contains(r##"<button class="qui-button primary js-packages-commit large" data-form="#form" type="button"><span>Commit</span><img class="qui-icon" src="/assets/img/icons/done.svg" /></button>"##);

        assert!(has_namespace_input);
        assert!(has_message_input);
        assert!(has_metadata_input);
        assert!(has_submit_button);
        Ok(())
    }

    fn make_changes(added: &[&str], modified: &[&str], removed: &[&str]) -> ChangeSet {
        let mut changes = BTreeMap::new();
        for name in added {
            changes.insert(PathBuf::from(name), Change::Added(ManifestRow::default()));
        }
        for name in modified {
            changes.insert(
                PathBuf::from(name),
                Change::Modified(ManifestRow::default()),
            );
        }
        for name in removed {
            changes.insert(PathBuf::from(name), Change::Removed(ManifestRow::default()));
        }
        changes
    }

    #[test]
    fn test_generate_commit_message_empty() {
        assert_eq!(generate_commit_message(&BTreeMap::new()).value, "");
    }

    #[test]
    fn test_generate_commit_message_single_add() {
        let changes = make_changes(&["results.csv"], &[], &[]);
        assert_eq!(generate_commit_message(&changes).value, "Add results.csv");
    }

    #[test]
    fn test_generate_commit_message_single_modify() {
        let changes = make_changes(&[], &["data.parquet"], &[]);
        assert_eq!(
            generate_commit_message(&changes).value,
            "Update data.parquet"
        );
    }

    #[test]
    fn test_generate_commit_message_single_remove() {
        let changes = make_changes(&[], &[], &["old.csv"]);
        assert_eq!(generate_commit_message(&changes).value, "Remove old.csv");
    }

    #[test]
    fn test_generate_commit_message_mixed_few() {
        let changes = make_changes(&["results.csv"], &[], &["old.csv"]);
        assert_eq!(
            generate_commit_message(&changes).value,
            "Add results.csv, Remove old.csv"
        );
    }

    #[test]
    fn test_generate_commit_message_three_files() {
        let changes = make_changes(&["a.csv", "b.csv"], &["c.csv"], &[]);
        assert_eq!(
            generate_commit_message(&changes).value,
            "Add a.csv, b.csv, Update c.csv"
        );
    }

    #[test]
    fn test_generate_commit_message_many_adds() {
        let names: Vec<String> = (1..=5).map(|i| format!("file{i}.csv")).collect();
        let name_refs: Vec<&str> = names.iter().map(|s| s.as_str()).collect();
        let changes = make_changes(&name_refs, &[], &[]);
        assert_eq!(generate_commit_message(&changes).value, "Add 5 files");
    }

    #[test]
    fn test_generate_commit_message_many_mixed() {
        let added: Vec<String> = (1..=3).map(|i| format!("add{i}.csv")).collect();
        let modified: Vec<String> = (1..=2).map(|i| format!("mod{i}.csv")).collect();
        let removed = ["old.csv".to_string()];
        let changes = make_changes(
            &added.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            &modified.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
            &removed.iter().map(|s| s.as_str()).collect::<Vec<_>>(),
        );
        assert_eq!(
            generate_commit_message(&changes).value,
            "Add 3 files, Update 2 files, Remove 1 file"
        );
    }

    #[test]
    fn test_generate_commit_message_uses_filename_not_full_path() {
        let changes = make_changes(&["subdir/data/results.csv"], &[], &[]);
        assert_eq!(generate_commit_message(&changes).value, "Add results.csv");
    }

    #[test]
    fn test_workflow_with_value() -> Result<(), Error> {
        let workflow_id = "test-workflow-123";
        let workflow_url = url::Url::parse("https://test.quilt.dev/workflows/config.yaml")?;

        let html = ViewCommit {
            entries_modified: vec![],
            entries_rest: vec![],
            entries_ignored: vec![],
            filter: EntriesFilter::default(),
            uri: quilt::uri::S3PackageUri::try_from("quilt+s3://C#package=A/B")?,
            origin: Some(url::Url::parse("https://test.quilt.dev/C/packages/A/B")?),
            message: ViewCommitMessage {
                value: "".to_string(),
                error: None,
            },
            user_meta: ViewCommitUserMeta {
                value: "".to_string(),
                error: None,
            },
            ignored_count: 0,
            unmodified_count: 0,
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
        let workflow_url = url::Url::parse("https://test.quilt.dev/workflows/config.yaml")?;

        let html = ViewCommit {
            entries_modified: vec![],
            entries_rest: vec![],
            entries_ignored: vec![],
            filter: EntriesFilter::default(),
            uri: quilt::uri::S3PackageUri::try_from("quilt+s3://C#package=A/B")?,
            origin: Some(url::Url::parse("https://test.quilt.dev/C/packages/A/B")?),
            message: ViewCommitMessage {
                value: "".to_string(),
                error: None,
            },
            user_meta: ViewCommitUserMeta {
                value: "".to_string(),
                error: None,
            },
            ignored_count: 0,
            unmodified_count: 0,
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
        let html = ViewCommit {
            entries_modified: vec![],
            entries_rest: vec![],
            entries_ignored: vec![],
            filter: EntriesFilter::default(),
            uri: quilt::uri::S3PackageUri::try_from("quilt+s3://C#package=A/B")?,
            origin: Some(url::Url::parse("https://test.quilt.dev/C/packages/A/B")?),
            message: ViewCommitMessage::default(),
            user_meta: ViewCommitUserMeta::default(),
            ignored_count: 0,
            unmodified_count: 0,
            workflow: None,
        }
        .render()?;

        assert!(html.contains(r#"<div class="workflow"> <p class="field"> <label class="label" for="workflow" >Workflow ID</label> <input class="input" value="Workflow not available" disabled /></p>"#));
        assert!(html.contains(r#"<div class="workflow-null"> <input id="workflow-null" type="checkbox" checked disabled /> <label class="workflow-null-label" for="workflow-null">No workflow</label> </div>"#));

        Ok(())
    }
}
