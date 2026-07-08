//! `get_merge_data` / `get_commit_data` — commit-workflow queries for the Leptos UI.

use serde::Serialize;

use crate::Error;
use crate::model;
use crate::quilt;

use super::package_data::InstalledPackageEntryData;

// ── Merge data for Leptos UI ──

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MergeData {
    pub namespace: String,
    pub uri: Option<quilt_uri::S3PackageUri>,
}

async fn get_merge_data_from_model(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
    namespace: &quilt_uri::Namespace,
) -> Result<MergeData, Error> {
    let installed_package = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(
            namespace.to_owned(),
        ))
    })?;

    let lineage = m.get_installed_package_lineage(&installed_package).await?;

    let uri = lineage
        .remote_uri
        .as_ref()
        .map(quilt_uri::S3PackageUri::from);
    if let Some(host) = uri.as_ref().and_then(|u| u.catalog.as_ref()) {
        tracing.add_host(host);
    }

    Ok(MergeData {
        namespace: namespace.to_string(),
        uri,
    })
}

#[tauri::command]
pub async fn get_merge_data(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<MergeData, String> {
    let namespace: quilt_uri::Namespace = namespace
        .try_into()
        .map_err(|e: quilt_uri::UriError| e.to_string())?;

    get_merge_data_from_model(&*m, &tracing, &namespace)
        .await
        .map_err(|e| e.to_frontend_string())
}

// ── Commit data for Leptos UI ──

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitData {
    pub namespace: String,
    pub uri: Option<quilt_uri::S3PackageUri>,
    pub status: String,
    pub message: String,
    pub user_meta: String,
    pub user_meta_error: Option<String>,
    pub workflow: Option<CommitWorkflowData>,
    pub workflows: Vec<CommitWorkflowInfo>,
    pub default_workflow: Option<String>,
    pub is_workflow_required: bool,
    pub entries: Vec<InstalledPackageEntryData>,
    pub ignored_count: usize,
    pub unmodified_count: usize,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitWorkflowData {
    pub id: Option<String>,
    pub url: Option<String>,
    pub config_url: Option<String>,
}

/// A workflow declared under `workflows:` in the bucket's config, surfaced to
/// the commit dialog so the user can pick one. Distinct from
/// [`CommitWorkflowData`], which is the previous revision's stamped selection.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitWorkflowInfo {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
}

async fn get_commit_data_from_model(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
    namespace: &quilt_uri::Namespace,
) -> Result<CommitData, Error> {
    let installed_package = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(
            namespace.to_owned(),
        ))
    })?;

    let status = m
        .get_installed_package_status(&installed_package, None)
        .await?;

    let pkg_status_str = match status.upstream_state {
        quilt::lineage::UpstreamState::UpToDate => "up_to_date",
        quilt::lineage::UpstreamState::Ahead => "ahead",
        quilt::lineage::UpstreamState::Behind => "behind",
        quilt::lineage::UpstreamState::Diverged => "diverged",
        quilt::lineage::UpstreamState::Local => "local",
        quilt::lineage::UpstreamState::Error => "error",
    };

    let lineage = m.get_installed_package_lineage(&installed_package).await?;

    let typed_uri = lineage
        .remote_uri
        .as_ref()
        .map(quilt_uri::S3PackageUri::from);
    let origin_host = typed_uri.as_ref().and_then(|u| u.catalog.as_ref());
    if let Some(host) = origin_host {
        tracing.add_host(host);
    }

    // Build lookup maps for junky files
    let junky_map: std::collections::HashMap<_, _> = status
        .junky_changes
        .iter()
        .map(|(p, pat)| (p.clone(), pat.clone()))
        .collect();

    // Modified entries
    let mut entries_list = Vec::new();
    for (filename, change) in &status.changes {
        let (status_str, size) = match change {
            quilt::lineage::Change::Added(r) => ("added", r.size),
            quilt::lineage::Change::Modified(r) => ("modified", r.size),
            quilt::lineage::Change::Removed(r) => ("deleted", r.size),
        };
        entries_list.push(InstalledPackageEntryData {
            filename: filename.display().to_string(),
            size,
            status: status_str.to_string(),
            junky_pattern: junky_map.get(filename).cloned(),
            ignored_by: None,
            namespace: namespace.to_string(),
        });
        if entries_list.len() > 1000 {
            break;
        }
    }

    // Unmodified entries (from manifest, not changed)
    let manifest_entries = m.get_installed_package_records(&installed_package).await?;
    for (filename, row) in &manifest_entries {
        if status.changes.contains_key(filename) {
            continue;
        }
        entries_list.push(InstalledPackageEntryData {
            filename: filename.display().to_string(),
            size: row.size,
            status: if lineage.paths.contains_key(filename) {
                "pristine"
            } else {
                "remote"
            }
            .to_string(),
            junky_pattern: None,
            ignored_by: None,
            namespace: namespace.to_string(),
        });
        if entries_list.len() > 1000 {
            break;
        }
    }

    // Ignored files
    for (filename, pattern, size) in &status.ignored_files {
        entries_list.push(InstalledPackageEntryData {
            filename: filename.display().to_string(),
            size: *size,
            status: "pristine".to_string(),
            junky_pattern: None,
            ignored_by: Some(pattern.clone()),
            namespace: namespace.to_string(),
        });
        if entries_list.len() > 1000 {
            break;
        }
    }

    entries_list.sort_by(|a, b| a.filename.cmp(&b.filename));

    // Compute counts from the full source data, not the capped entries_list,
    // so the filter toolbar is shown even when the list is truncated.
    let ignored_count = status.ignored_files.len();
    let unmodified_count = manifest_entries
        .keys()
        .filter(|f| !status.changes.contains_key(*f))
        .count();

    // Generate commit message from changes
    let message = crate::commit_message::generate(&status.changes);

    // Load remote manifest for user_meta and workflow
    let (user_meta, user_meta_error, workflow) =
        match lineage.remote_uri.as_ref().filter(|r| !r.hash.is_empty()) {
            Some(remote_uri) => {
                let remote_manifest = m.browse_remote_manifest(remote_uri).await?;
                let (meta_value, meta_error) = match &remote_manifest.header.user_meta {
                    Some(meta) => match serde_json::to_string(meta) {
                        Ok(v) => (v, None),
                        Err(_) => (String::new(), Some("Failed to stringify meta".to_string())),
                    },
                    None => (String::new(), None),
                };
                let workflow = origin_host.and_then(|host| {
                    remote_manifest
                        .header
                        .workflow
                        .as_ref()
                        .map(|w| CommitWorkflowData {
                            id: w.id.as_ref().map(|id| id.id.clone()),
                            url: w.config.display_for_host(host).ok().map(|u| u.to_string()),
                            config_url: None,
                        })
                });
                (meta_value, meta_error, workflow)
            }
            None => (String::new(), None, None),
        };

    // Fetch the bucket's declared workflows for the commit dialog. A fetch
    // failure (or a package with no remote) must NOT fail the command — the
    // dialog still opens with the permissive/degraded default and the UI falls
    // back to today's control.
    let (workflows, default_workflow, is_workflow_required) =
        match m.get_workflows_config(&installed_package).await {
            Ok(Some(config)) => (
                config
                    .workflows
                    .into_iter()
                    .map(|w| CommitWorkflowInfo {
                        id: w.id,
                        name: w.name,
                        description: w.description,
                    })
                    .collect(),
                config.default_workflow,
                config.is_workflow_required,
            ),
            Ok(None) | Err(_) => (Vec::new(), None, false),
        };

    Ok(CommitData {
        namespace: namespace.to_string(),
        uri: typed_uri,
        status: pkg_status_str.to_string(),
        message,
        user_meta,
        user_meta_error,
        workflow,
        workflows,
        default_workflow,
        is_workflow_required,
        entries: entries_list,
        ignored_count,
        unmodified_count,
    })
}

#[tauri::command]
pub async fn get_commit_data(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<CommitData, String> {
    let namespace: quilt_uri::Namespace = namespace
        .try_into()
        .map_err(|e: quilt_uri::UriError| e.to_string())?;

    get_commit_data_from_model(&*m, &tracing, &namespace)
        .await
        .map_err(|e| e.to_frontend_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::commands::test_support::*;
    use crate::model::mocks;

    #[tokio::test]
    async fn test_get_merge_data() -> Result<(), String> {
        let mut model = mocks::create();
        mocks::mock_installed_package(&mut model);
        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_merge_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(data.namespace, "foo/bar");
        let uri = data.uri.as_ref().expect("URI present");
        assert_eq!(uri.bucket, "quilt-example");
        assert_eq!(catalog_host(&data.uri).as_deref(), Some("test.quilt.dev"));
        Ok(())
    }

    #[tokio::test]
    async fn test_get_merge_data_not_installed() {
        let mut model = mocks::create();
        model.expect_get_installed_package().returning(|_| Ok(None));
        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("missing", "package").into();

        let result = get_merge_data_from_model(&model, &tracing, &namespace).await;
        assert!(result.is_err());
    }

    // ── Commit data tests ──

    #[tokio::test]
    async fn test_get_commit_data() -> Result<(), String> {
        let mut model = mocks::create();
        mocks::mock_installed_package(&mut model);
        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(data.namespace, "foo/bar");
        let uri = data.uri.as_ref().expect("URI present");
        assert_eq!(uri.bucket, "quilt-example");
        assert_eq!(catalog_host(&data.uri).as_deref(), Some("test.quilt.dev"));
        Ok(())
    }

    #[tokio::test]
    async fn test_get_commit_data_not_installed() {
        let mut model = mocks::create();
        model.expect_get_installed_package().returning(|_| Ok(None));
        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("missing", "package").into();

        let result = get_commit_data_from_model(&model, &tracing, &namespace).await;
        assert!(result.is_err());
    }

    // (Adapted from pages/commit.rs: test_workflow_with_value)

    #[tokio::test]
    async fn test_get_commit_data_with_workflow() -> Result<(), String> {
        let mut model = mocks::create();

        let remote_manifest = quilt_uri::ManifestUri {
            bucket: "quilt-example".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "abcdef".to_string(),
            origin: Some("test.quilt.dev".parse().unwrap()),
        };
        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(move |_| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    remote_manifest.clone(),
                    remote_manifest.hash.clone(),
                ))
            });
        let status = Ok(quilt::lineage::InstalledPackageStatus::default());
        model
            .expect_get_installed_package_status()
            .return_once(move |_, _| status);
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));
        model
            .expect_get_workflows_config()
            .returning(|_| Ok(None));
        // Return a manifest with workflow data
        model.expect_browse_remote_manifest().returning(|_| {
            let config_uri = quilt_uri::S3Uri {
                bucket: "quilt-example".to_string(),
                key: ".quilt/workflows/config.yaml".to_string(),
                version: None,
            };
            Ok(quilt::manifest::Manifest {
                header: quilt::manifest::ManifestHeader {
                    version: "v0".to_string(),
                    message: None,
                    user_meta: None,
                    workflow: Some(quilt::manifest::Workflow {
                        config: config_uri,
                        id: Some(quilt::manifest::WorkflowId {
                            id: "gamma".to_string(),
                            metadata: None,
                        }),
                    }),
                },
                rows: Vec::new(),
            })
        });

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert!(data.workflow.is_some());
        let workflow = data.workflow.unwrap();
        assert_eq!(workflow.id, Some("gamma".to_string()));
        assert!(workflow.url.is_some());
        Ok(())
    }

    // (Adapted from pages/commit.rs: test_workflow_null_checked)

    #[tokio::test]
    async fn test_get_commit_data_workflow_null_id() -> Result<(), String> {
        let mut model = mocks::create();

        let remote_manifest = quilt_uri::ManifestUri {
            bucket: "quilt-example".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "abcdef".to_string(),
            origin: Some("test.quilt.dev".parse().unwrap()),
        };
        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(move |_| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    remote_manifest.clone(),
                    remote_manifest.hash.clone(),
                ))
            });
        let status = Ok(quilt::lineage::InstalledPackageStatus::default());
        model
            .expect_get_installed_package_status()
            .return_once(move |_, _| status);
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));
        model
            .expect_get_workflows_config()
            .returning(|_| Ok(None));
        // Workflow exists but has no ID (null/checked state)
        model.expect_browse_remote_manifest().returning(|_| {
            let config_uri = quilt_uri::S3Uri {
                bucket: "quilt-example".to_string(),
                key: ".quilt/workflows/config.yaml".to_string(),
                version: None,
            };
            Ok(quilt::manifest::Manifest {
                header: quilt::manifest::ManifestHeader {
                    version: "v0".to_string(),
                    message: None,
                    user_meta: None,
                    workflow: Some(quilt::manifest::Workflow {
                        config: config_uri,
                        id: None,
                    }),
                },
                rows: Vec::new(),
            })
        });

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert!(data.workflow.is_some());
        let workflow = data.workflow.unwrap();
        assert!(workflow.id.is_none());
        assert!(workflow.url.is_some());
        Ok(())
    }

    // (Adapted from pages/commit.rs: test_workflow_not_available)

    #[tokio::test]
    async fn test_get_commit_data_no_workflow() -> Result<(), String> {
        let mut model = mocks::create();

        let remote_manifest = quilt_uri::ManifestUri {
            bucket: "quilt-example".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "abcdef".to_string(),
            origin: Some("test.quilt.dev".parse().unwrap()),
        };
        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(move |_| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    remote_manifest.clone(),
                    remote_manifest.hash.clone(),
                ))
            });
        let status = Ok(quilt::lineage::InstalledPackageStatus::default());
        model
            .expect_get_installed_package_status()
            .return_once(move |_, _| status);
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));
        model
            .expect_get_workflows_config()
            .returning(|_| Ok(None));
        // No workflow in manifest
        model.expect_browse_remote_manifest().returning(|_| {
            Ok(quilt::manifest::Manifest {
                header: quilt::manifest::ManifestHeader {
                    version: "v0".to_string(),
                    message: None,
                    user_meta: None,
                    workflow: None,
                },
                rows: Vec::new(),
            })
        });

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert!(data.workflow.is_none());
        Ok(())
    }

    // ── Bucket workflow list (the `workflows` / `default_workflow` /
    //    `is_workflow_required` fields) ──

    /// Set up the model mocks shared by the workflow-list tests: an installed
    /// package with a remote whose manifest carries no workflow stamp. The
    /// caller wires up `expect_get_workflows_config`.
    fn base_commit_model() -> crate::model::MockQuiltModel {
        let mut model = mocks::create();

        let remote_manifest = quilt_uri::ManifestUri {
            bucket: "quilt-example".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "abcdef".to_string(),
            origin: Some("test.quilt.dev".parse().unwrap()),
        };
        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(move |_| {
                Ok(quilt::lineage::PackageLineage::from_remote(
                    remote_manifest.clone(),
                    remote_manifest.hash.clone(),
                ))
            });
        model
            .expect_get_installed_package_status()
            .returning(|_, _| Ok(quilt::lineage::InstalledPackageStatus::default()));
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));
        model.expect_browse_remote_manifest().returning(|_| {
            Ok(quilt::manifest::Manifest {
                header: quilt::manifest::ManifestHeader {
                    version: "v0".to_string(),
                    message: None,
                    user_meta: None,
                    workflow: None,
                },
                rows: Vec::new(),
            })
        });
        model
    }

    #[tokio::test]
    async fn test_get_commit_data_workflows_list() -> Result<(), String> {
        let mut model = base_commit_model();
        // A sandbox-shaped config: multiple workflows, a declared default, and
        // the required flag set true.
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
version: "1"
is_workflow_required: true
default_workflow: dummy
workflows:
  dummy:
    name: Dummy workflow
    description: Do nothing.
  alpha:
    name: Alpha
    description: First workflow.
"#,
        )
        .map_err(|e| e.to_string())?;
        let config =
            quilt::io::remote::WorkflowsConfig::from_yaml(&yaml).map_err(|e| e.to_string())?;
        model
            .expect_get_workflows_config()
            .return_once(move |_| Ok(Some(config)));

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(data.default_workflow, Some("dummy".to_string()));
        assert!(data.is_workflow_required);
        let ids: Vec<_> = data.workflows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, vec!["dummy", "alpha"]);
        assert_eq!(data.workflows[0].name.as_deref(), Some("Dummy workflow"));
        assert_eq!(data.workflows[0].description.as_deref(), Some("Do nothing."));
        Ok(())
    }

    #[tokio::test]
    async fn test_get_commit_data_no_workflows_config() -> Result<(), String> {
        // No config in the bucket → degraded/permissive default.
        let mut model = base_commit_model();
        model
            .expect_get_workflows_config()
            .returning(|_| Ok(None));

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert!(data.workflows.is_empty());
        assert_eq!(data.default_workflow, None);
        assert!(!data.is_workflow_required);
        Ok(())
    }

    #[tokio::test]
    async fn test_get_commit_data_workflows_config_fetch_fails() -> Result<(), String> {
        // A config-fetch failure must NOT fail get_commit_data: the dialog still
        // opens with the degraded default so the UI can fall back.
        let mut model = base_commit_model();
        model.expect_get_workflows_config().returning(|_| {
            Err(Error::from(quilt::InstallPackageError::NotInstalled(
                ("foo", "bar").into(),
            )))
        });

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert!(data.workflows.is_empty());
        assert_eq!(data.default_workflow, None);
        assert!(!data.is_workflow_required);
        Ok(())
    }
}
