use std::borrow::Cow;
use std::collections::HashMap;

use askama::Template;
use rust_i18n::t;

use crate::debug_tools;
use crate::error::Error;
use crate::model::QuiltModel;
use crate::quilt;
use crate::quilt::lineage::Change;
use crate::quilt::lineage::UpstreamState;
use crate::quilt::uri::Namespace;
use crate::quilt::uri::S3PackageUri;
use crate::routes::EntriesFilter;
use crate::routes::Paths;
use crate::telemetry::prelude::*;
use crate::ui::btn;
use crate::ui::crumbs;
use crate::ui::entry;
use crate::ui::layout::Layout;
use crate::ui::Icon;
use crate::Result;

#[derive(Debug)]
pub struct ViewInstalledPackage {
    entries_list: Vec<entry::ViewEntry>,
    filter: EntriesFilter,
    origin: Option<url::Url>,
    origin_host: Option<quilt::uri::Host>,
    status: UpstreamState,
    uri: S3PackageUri,
    ignored_count: usize,
    unmodified_count: usize,
}

#[derive(Template)]
#[template(path = "./components/status.html")]
struct TmplStatus<'a> {
    description: Cow<'a, str>,
    button: btn::TmplButton<'a>,
    secondary_button: Option<btn::TmplButton<'a>>,
}

impl TmplStatus<'_> {
    fn new(
        namespace: &Namespace,
        status: &UpstreamState,
        origin_host: Option<&quilt::uri::Host>,
    ) -> Option<Self> {
        match status {
            UpstreamState::Ahead => Some(TmplStatus {
                description: t!("installed_package_status.ahead"),
                button: btn::TmplButton::builder()
                    .set_data("namespace", namespace.to_string())
                    .set_js(btn::JsSelector::PackagesPush)
                    .set_label(t!("buttons.push_package"))
                    .set_color(btn::Color::Primary),
                secondary_button: None,
            }),
            UpstreamState::Behind => Some(TmplStatus {
                description: t!("installed_package_status.behind"),
                button: btn::TmplButton::builder()
                    .set_data("namespace", namespace.to_string())
                    .set_js(btn::JsSelector::PackagesPull)
                    .set_label(t!("buttons.pull_package"))
                    .set_color(btn::Color::Primary),
                secondary_button: None,
            }),
            UpstreamState::Diverged => Some(TmplStatus {
                description: t!("installed_package_status.diverged"),
                button: btn::TmplButton::builder()
                    .set_data("namespace", namespace.to_string())
                    .set_label(t!("buttons.merge_package"))
                    .set_color(btn::Color::Primary)
                    .set_href(Paths::Merge(namespace.clone())),
                secondary_button: None,
            }),
            UpstreamState::Error => match origin_host {
                Some(host) => Some(TmplStatus {
                    description: t!("installed_package_status.error"),
                    button: btn::TmplButton::builder()
                        .set_label(t!("error.login"))
                        .set_icon(Icon::Warning)
                        .set_color(btn::Color::Warning)
                        .set_href(Paths::Login(
                            host.clone(),
                            Paths::InstalledPackage(
                                namespace.clone(),
                                EntriesFilter::for_installed_package(),
                            )
                            .to_string(),
                        )),
                    secondary_button: Some(
                        btn::TmplButton::builder()
                            .set_data("namespace", namespace.to_string())
                            .set_data("origin", host.to_string())
                            .set_js(btn::JsSelector::SetOrigin)
                            .set_label(t!("buttons.change_origin")),
                    ),
                }),
                None => Some(TmplStatus {
                    description: t!("installed_package_status.no_origin"),
                    button: btn::TmplButton::builder()
                        .set_data("namespace", namespace.to_string())
                        .set_icon(Icon::Warning)
                        .set_js(btn::JsSelector::SetOrigin)
                        .set_label(t!("buttons.set_origin"))
                        .set_color(btn::Color::Warning),
                    secondary_button: None,
                }),
            },
            UpstreamState::Local | UpstreamState::UpToDate => None,
        }
    }
}

#[derive(Template)]
#[template(path = "./components/entries-toolbar.html")]
struct TmplEntriesToolbar<'a> {
    button: btn::TmplButton<'a>,
    show_button: bool,
    with_status: bool,
    filter_unmodified_checked: bool,
    filter_unmodified_href: String,
    filter_ignored_checked: bool,
    filter_ignored_href: String,
    ignored_count: usize,
    unmodified_count: usize,
}

#[derive(Template)]
#[template(path = "./pages/installed-package.html")]
struct TmplPageInstalledPackage<'a> {
    entries: Vec<entry::TmplEntry<'a>>,
    status: Option<TmplStatus<'a>>,
    toolbar: Option<TmplEntriesToolbar<'a>>,
    uri: quilt::uri::S3PackageUri,
    layout: Layout<'a>,
}

impl<'a> TmplPageInstalledPackage<'a> {
    pub fn primary_button(uri: &S3PackageUri, status: &UpstreamState) -> btn::TmplButton<'a> {
        let btn = btn::TmplButton::builder()
            .set_icon(Icon::ArrowForward)
            .set_href(Paths::Commit(
                uri.namespace.clone(),
                EntriesFilter::default(),
            ))
            .set_label(t!("installed_package.commit"))
            .set_size(btn::Size::Large)
            .set_direction(btn::Direction::RightToLeft);

        match status {
            UpstreamState::UpToDate => btn.set_color(btn::Color::Primary),
            _ => btn,
        }
    }

    fn breadcrumbs(uri: &S3PackageUri) -> crumbs::TmplBreadcrumbs<'a> {
        crumbs::TmplBreadcrumbs {
            list: vec![
                crumbs::Link::home(),
                crumbs::Current::create(t!("breadcrumbs.installed_package", s => uri.namespace)),
            ],
        }
    }

    fn actions(uri: &S3PackageUri, origin: Option<&url::Url>) -> Vec<btn::TmplButton<'a>> {
        let mut actions = vec![btn::TmplButton::builder()
            .set_data("namespace", uri.namespace.to_string())
            .set_icon(Icon::FolderOpen)
            .set_js(btn::JsSelector::OpenInFileBrowser)
            .set_label(t!("buttons.open_package_in_file_browser"))];
        if let Some(origin) = origin {
            actions.push(
                btn::TmplButton::builder()
                    .set_data("url", origin.to_string())
                    .set_icon(Icon::OpenInBrowser)
                    .set_js(btn::JsSelector::OpenInWebBrowser)
                    .set_label(t!("buttons.open_package_in_catalog")),
            );
        }
        actions.push(
            btn::TmplButton::builder()
                .set_data("namespace", uri.namespace.to_string())
                .set_icon(Icon::Block)
                .set_js(btn::JsSelector::PackagesUninstall)
                .set_label(t!("buttons.uninstall_package")),
        );
        actions
    }
}

impl ViewInstalledPackage {
    pub async fn create(
        model: &impl QuiltModel,
        tracing: &crate::telemetry::Telemetry,
        namespace: &quilt::uri::Namespace,
        filter: &EntriesFilter,
    ) -> Result<ViewInstalledPackage> {
        let installed_package = model
            .get_installed_package(namespace)
            .await?
            .ok_or_else(|| Error::Quilt(quilt::Error::PackageNotInstalled(namespace.clone())))?;

        let lineage = model
            .get_installed_package_lineage(&installed_package)
            .await?;

        // TODO: just use remote_manifest?
        let (uri, origin_host) =
            debug_tools::resolve_uri_and_host(lineage.remote_uri.as_ref(), namespace);
        if let Some(host) = &origin_host {
            tracing.add_host(host);
        }

        let status = if lineage.remote_uri.is_none() || origin_host.is_some() {
            match model
                .get_installed_package_status(&installed_package, None)
                .await
            {
                Ok(status) => status,
                Err(err) => {
                    warn!("Failed to get status for {namespace}: {err}");
                    quilt::lineage::InstalledPackageStatus::error()
                }
            }
        } else {
            quilt::lineage::InstalledPackageStatus::error()
        };

        let modified_entries = &status.changes;
        let installed_paths = &lineage.paths;
        let manifest_entries = model
            .get_installed_package_records(&installed_package)
            .await?;

        // Build lookup maps for junky and ignored files
        let junky_map: HashMap<_, _> = status
            .junky_changes
            .iter()
            .map(|(p, pat)| (p.clone(), pat.clone()))
            .collect();

        let mut entries_list = Vec::new();
        for (filename, change) in modified_entries {
            let entry_uri = quilt::uri::S3PackageUri {
                path: Some(filename.to_owned()),
                ..uri.clone()
            };
            let origin = match &origin_host {
                Some(host) => Some(entry_uri.display_for_host(host)?),
                None => None,
            };
            entries_list.push(entry::ViewEntry {
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
            if entries_list.len() > 1000 {
                break;
            }
        }
        for filename in installed_paths.keys() {
            if modified_entries.contains_key(filename) {
                continue;
            }
            let row = manifest_entries.get(filename);
            if let Some(row) = row {
                let entry_uri = quilt::uri::S3PackageUri {
                    path: Some(filename.to_owned()),
                    ..uri.clone()
                };
                let origin = match &origin_host {
                    Some(host) => Some(entry_uri.display_for_host(host)?),
                    None => None,
                };
                entries_list.push(entry::ViewEntry {
                    filename: filename.clone(),
                    origin,
                    size: row.size,
                    status: entry::EntryStatus::Pristine,
                    uri: entry_uri,
                    junky_pattern: None,
                    ignored_by: None,
                });
            } else {
                error!(
                    "Installed filename {:?} doesn't exist in manifest",
                    filename
                );
                continue;
            }
            if entries_list.len() > 1000 {
                break;
            }
        }
        for (filename, row) in manifest_entries {
            if installed_paths.contains_key(&filename) {
                continue;
            }
            if modified_entries.contains_key(&filename) {
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
            entries_list.push(entry::ViewEntry {
                filename,
                size: row.size,
                status: entry::EntryStatus::Remote,
                uri: entry_uri,
                origin,
                junky_pattern: None,
                ignored_by: None,
            });
            if entries_list.len() > 1000 {
                break;
            }
        }

        // Add ignored files as separate entries
        for (filename, pattern) in &status.ignored_files {
            let entry_uri = quilt::uri::S3PackageUri {
                path: Some(filename.clone()),
                ..uri.clone()
            };
            entries_list.push(entry::ViewEntry {
                filename: filename.clone(),
                size: 0,
                status: entry::EntryStatus::Pristine,
                uri: entry_uri,
                origin: None,
                junky_pattern: None,
                ignored_by: Some(pattern.clone()),
            });
            if entries_list.len() > 1000 {
                break;
            }
        }

        // Sort entries by filename
        entries_list.sort_by(|a, b| a.filename.cmp(&b.filename));

        let origin = match &origin_host {
            Some(host) => Some(uri.display_for_host(host)?),
            None => None,
        };

        let ignored_count = entries_list
            .iter()
            .filter(|e| e.ignored_by.is_some())
            .count();
        let unmodified_count = entries_list
            .iter()
            .filter(|e| {
                e.ignored_by.is_none()
                    && (matches!(e.status, entry::EntryStatus::Pristine)
                        || matches!(e.status, entry::EntryStatus::Remote))
            })
            .count();

        // Apply filter: skip entries that are hidden by the current filter
        let entries_list: Vec<_> = entries_list
            .into_iter()
            .filter(|e| {
                if e.ignored_by.is_some() {
                    return filter.ignored;
                }
                if matches!(e.status, entry::EntryStatus::Pristine)
                    || matches!(e.status, entry::EntryStatus::Remote)
                {
                    return filter.unmodified;
                }
                true
            })
            .collect();

        Ok(ViewInstalledPackage {
            entries_list,
            filter: filter.clone(),
            origin,
            origin_host,
            status: status.upstream_state,
            uri: uri.clone(),
            ignored_count,
            unmodified_count,
        })
    }

    pub fn render(self) -> Result<String> {
        Ok(TmplPageInstalledPackage::from(self)
            .render()?
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "))
    }
}

impl From<ViewInstalledPackage> for TmplPageInstalledPackage<'_> {
    fn from(view: ViewInstalledPackage) -> Self {
        let ViewInstalledPackage {
            entries_list,
            filter,
            origin,
            origin_host,
            status,
            uri,
            ignored_count,
            unmodified_count,
        } = view;
        let has_remote_entries = entries_list
            .iter()
            .any(|entry| matches!(entry.status, entry::EntryStatus::Remote));
        let mut entries = Vec::new();
        for entry in entries_list {
            entries.push(entry::TmplEntry::from(entry).set_checkbox(false));
        }

        let toggled_unmodified = filter.toggle_unmodified();
        let toggled_ignored = filter.toggle_ignored();
        let filter_unmodified_href =
            Paths::InstalledPackage(uri.namespace.clone(), toggled_unmodified).to_string();
        let filter_ignored_href =
            Paths::InstalledPackage(uri.namespace.clone(), toggled_ignored).to_string();

        let layout = Layout::builder()
            .set_breadcrumbs(Self::breadcrumbs(&uri))
            .set_actions(Self::actions(&uri, origin.as_ref()))
            .set_uri(Some(uri.clone()));
        let layout = if matches!(status, UpstreamState::Error) {
            layout
        } else {
            layout.set_primary_action(Self::primary_button(&uri, &status))
        };
        TmplPageInstalledPackage {
            layout,
            status: TmplStatus::new(&uri.namespace, &status, origin_host.as_ref()),
            entries,
            toolbar: {
                if has_remote_entries || ignored_count > 0 || unmodified_count > 0 {
                    Some(TmplEntriesToolbar {
                        button: btn::TmplButton::builder()
                            .set_js(btn::JsSelector::EntriesInstall)
                            .set_color(btn::Color::Primary)
                            .set_disabled()
                            .set_type(btn::ButtonType::Submit)
                            .set_label(t!("buttons.install_selected_paths")),
                        show_button: has_remote_entries,
                        with_status: !matches!(
                            status,
                            quilt::lineage::UpstreamState::UpToDate
                                | quilt::lineage::UpstreamState::Local
                        ),
                        filter_unmodified_checked: filter.unmodified,
                        filter_unmodified_href,
                        filter_ignored_checked: filter.ignored,
                        filter_ignored_href,
                        ignored_count,
                        unmodified_count,
                    })
                } else {
                    None
                }
            },
            uri: uri.clone(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use crate::model::mocks as model_mocks;

    #[test]
    fn test_view() -> Result {
        let html = (ViewInstalledPackage {
            entries_list: vec![],
            filter: EntriesFilter::for_installed_package(),
            origin: Some(url::Url::parse("https://test.quilt.dev/b/C/packages/A/B")?),
            origin_host: Some("test.quilt.dev".parse().unwrap()),
            status: quilt::lineage::UpstreamState::UpToDate,
            uri: quilt::uri::S3PackageUri::try_from("quilt+s3://C#package=A/B")?,
            ignored_count: 0,
            unmodified_count: 0,
        })
        .render()?;

        assert!(html.contains(r#"data-testid="installed-package-entries""#));
        Ok(())
    }

    #[tokio::test]
    async fn test_view_entries() -> Result {
        let mut model = model_mocks::create();
        model_mocks::mock_installed_package(&mut model);

        let installed_package = ViewInstalledPackage::create(
            &model,
            &crate::telemetry::Telemetry::default(),
            &("foo", "bar").into(),
            &EntriesFilter::for_installed_package(),
        )
        .await?;
        let html = installed_package.render()?;

        let page_has_entry = html.contains(r#"data-testid="entry-name" >NAME"#);
        assert!(page_has_entry);

        Ok(())
    }

    #[test]
    fn test_sizes() -> Result {
        // Create entries with different sizes
        let mut entries_list = Vec::new();

        // Create entries with different sizes as mentioned in the comment
        let sizes = [
            (0, "0 B"),
            (12, "12 B"),
            (1234, "1.23 kB"),
            (12345678, "12.35 MB"),
            (1234567890123456, "1.23 PB"),
            (12345678901234567890, "12.35 EB"),
        ];

        for (i, (size_bytes, _)) in sizes.iter().enumerate() {
            let filename = PathBuf::from(format!("test_file_{i}"));
            entries_list.push(entry::ViewEntry {
                filename,
                size: *size_bytes,
                status: entry::EntryStatus::Pristine,
                uri: quilt::uri::S3PackageUri::try_from("quilt+s3://C#package=A/B")?,
                origin: Some(url::Url::parse("https://test.quilt.dev/b/C/packages/A/B")?),
                junky_pattern: None,
                ignored_by: None,
            });
        }

        // Create the ViewInstalledPackage with our test entries
        let view = ViewInstalledPackage {
            entries_list,
            filter: EntriesFilter::for_installed_package(),
            origin: Some(url::Url::parse("https://test.quilt.dev/b/C/packages/A/B")?),
            origin_host: Some("test.quilt.dev".parse().unwrap()),
            status: quilt::lineage::UpstreamState::UpToDate,
            uri: quilt::uri::S3PackageUri::try_from("quilt+s3://C#package=A/B")?,
            ignored_count: 0,
            unmodified_count: 0,
        };

        // Render the view to HTML
        let html = view.render()?;

        // Verify that each size appears in the rendered HTML with the correct format
        for (_, size_str) in sizes.iter() {
            assert!(html.contains(&format!(
                "<p class=\"text-secondary\">Downloaded, {size_str}</p>"
            )));
        }

        Ok(())
    }

    #[test]
    fn test_view_no_origin() -> Result {
        let html = (ViewInstalledPackage {
            entries_list: vec![],
            filter: EntriesFilter::for_installed_package(),
            origin: None,
            origin_host: None,
            status: quilt::lineage::UpstreamState::Error,
            uri: quilt::uri::S3PackageUri::try_from("quilt+s3://C#package=A/B")?,
            ignored_count: 0,
            unmodified_count: 0,
        })
        .render()?;

        // Should show "Set origin" button
        assert!(html.contains(r#"js-set-origin"#));
        assert!(html.contains(r#"warning"#));

        // Should not show commit button
        assert!(!html.contains(r#"href="commit.html"#));

        // Should not show "Open in Catalog" action
        assert!(!html.contains("Open in Catalog"));

        // Should still show basic page structure
        assert!(html.contains(r#"data-testid="installed-package-entries""#));

        Ok(())
    }

    #[test]
    fn test_view_status_failed() -> Result {
        let html = (ViewInstalledPackage {
            entries_list: vec![],
            filter: EntriesFilter::for_installed_package(),
            origin: Some(url::Url::parse("https://test.quilt.dev/b/C/packages/A/B")?),
            origin_host: Some("test.quilt.dev".parse().unwrap()),
            status: quilt::lineage::UpstreamState::Error,
            uri: quilt::uri::S3PackageUri::try_from("quilt+s3://C#package=A/B")?,
            ignored_count: 0,
            unmodified_count: 0,
        })
        .render()?;

        // Should show Login button
        assert!(html.contains(r#"href="login.html#host=test.quilt.dev&#38;back=installed-package.html%23namespace%3DA%2FB%26filter%3Dunmodified""#));

        // Should not show commit button
        assert!(!html.contains(r#"href="commit.html"#));

        // Should still show "Open in Catalog" since origin is valid
        assert!(html.contains("Open in Catalog"));

        Ok(())
    }

    #[test]
    fn test_view_local_only() -> Result {
        let html = (ViewInstalledPackage {
            entries_list: vec![],
            filter: EntriesFilter::for_installed_package(),
            origin: None,
            origin_host: None,
            status: quilt::lineage::UpstreamState::Local,
            uri: quilt::uri::S3PackageUri::try_from("quilt+s3://C#package=A/B")?,
            ignored_count: 0,
            unmodified_count: 0,
        })
        .render()?;

        // Should not show any status banner (no "Set origin", no error)
        assert!(!html.contains(r#"js-set-origin"#));
        assert!(!html.contains(r#"warning"#));

        // Should show commit button (local packages can be committed)
        assert!(html.contains(r#"href="commit.html"#));

        // Should still show basic page structure
        assert!(html.contains(r#"data-testid="installed-package-entries""#));

        Ok(())
    }
}
