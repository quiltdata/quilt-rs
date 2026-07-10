//! `get_merge_data` / `get_commit_data` — commit-workflow queries for the Leptos UI.

use std::str::FromStr;

use serde::Serialize;

use quilt_rs::io::remote::WORKFLOWS_CONFIG_KEY;
use quilt_rs::io::remote::WorkflowsConfig;
use quilt_uri::Host;
use quilt_uri::S3Uri;

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
    pub workflows: CommitWorkflows,
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
///
/// `metadata_schema_url` / `entries_schema_url` are catalog HTTPS links to the
/// schema objects the workflow declares, pre-formatted for the currently-known
/// catalog host — `None` when the workflow declares no such schema, or when
/// there is no catalog host to link against.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CommitWorkflowInfo {
    pub id: String,
    pub name: Option<String>,
    pub description: Option<String>,
    pub metadata_schema_url: Option<String>,
    pub entries_schema_url: Option<String>,
}

/// The bucket's workflow-selection situation, as the commit dialog should
/// present it. Splits the cases the backend can distinguish so the UI never
/// conflates "ungoverned bucket", "couldn't load the config", and "the config
/// is broken":
/// - `Available` — the bucket has a workflows config; carry its choices.
/// - `NotConfigured` — no config (or no remote); the bucket is ungoverned.
/// - `Unavailable` — a transient/network failure loading the config; a commit
///   will retry resolving the bucket default.
/// - `Invalid` — the config exists but is malformed (schema violation). Every
///   commit to this bucket will FAIL until it is fixed, so the UI must not
///   promise a fallback; `reason` names the violation succinctly.
#[derive(Serialize)]
#[serde(
    tag = "state",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum CommitWorkflows {
    Available {
        workflows: Vec<CommitWorkflowInfo>,
        default_workflow: Option<String>,
        is_workflow_required: bool,
        /// Catalog HTTPS link to the bucket's `.quilt/workflows/config.yml`
        /// object, or `None` when there is no catalog host to link against.
        config_url: Option<String>,
    },
    NotConfigured,
    Unavailable,
    Invalid {
        reason: String,
        /// Catalog HTTPS link to the bucket's `.quilt/workflows/config.yml`
        /// object, so the user can open the broken config to fix it. `None`
        /// when there is no catalog host (or bucket) to link against.
        config_url: Option<String>,
    },
}

/// Format the catalog HTTPS link for an S3 object, or `None` when there is no
/// catalog host to link against. Reuses [`S3Uri::display_for_host`] — the same
/// on-demand catalog-link formatting the rest of the app uses.
fn catalog_object_url(uri: &S3Uri, host: Option<&Host>) -> Option<String> {
    let host = host?;
    uri.display_for_host(host).ok().map(|u| u.to_string())
}

/// Catalog HTTPS link to a bucket's `.quilt/workflows/config.yml`, or `None`
/// when there is no catalog host or bucket to link against. Shared by the
/// `Available` and `Invalid` states so the config link is built one way.
fn config_object_url(host: Option<&Host>, bucket: Option<&str>) -> Option<String> {
    let bucket = bucket?;
    catalog_object_url(
        &S3Uri {
            bucket: bucket.to_string(),
            key: WORKFLOWS_CONFIG_KEY.to_string(),
            version: None,
        },
        host,
    )
}

/// Map a fetched workflows config to the UI's state. Shared by the commit
/// dialog and the pre-set-remote bucket preview so both present the same shape:
/// `Ok(Some)` → `Available`, `Ok(None)` → `NotConfigured`; an `Err` splits by
/// variant — a malformed config (`InvalidWorkflowsConfig`) → `Invalid` (commits
/// will fail until fixed), any other error → `Unavailable` (transient; logged,
/// never fatal, so the caller degrades gracefully).
fn workflows_config_to_commit_workflows(
    config: Result<Option<WorkflowsConfig>, Error>,
    host: Option<&Host>,
    bucket: Option<&str>,
) -> CommitWorkflows {
    match config {
        Ok(Some(config)) => {
            let config_url = config_object_url(host, bucket);
            // Resolve each workflow's schema links first — this borrow of
            // `config` ends here — so the strings can then be *moved* out of
            // `config.workflows` instead of cloned. A misconfigured `schemas`
            // section degrades to no links rather than dropping the dialog.
            let schema_urls: Vec<(Option<String>, Option<String>)> = config
                .workflows
                .iter()
                .map(|w| {
                    let schemas = config.schema_uris(&w.id);
                    (
                        schemas
                            .metadata_schema
                            .and_then(|uri| catalog_object_url(&uri, host)),
                        schemas
                            .entries_schema
                            .and_then(|uri| catalog_object_url(&uri, host)),
                    )
                })
                .collect();
            let workflows = config
                .workflows
                .into_iter()
                .zip(schema_urls)
                .map(
                    |(w, (metadata_schema_url, entries_schema_url))| CommitWorkflowInfo {
                        id: w.id,
                        name: w.name,
                        description: w.description,
                        metadata_schema_url,
                        entries_schema_url,
                    },
                )
                .collect();
            CommitWorkflows::Available {
                workflows,
                default_workflow: config.default_workflow,
                is_workflow_required: config.is_workflow_required,
                config_url,
            }
        }
        Ok(None) => CommitWorkflows::NotConfigured,
        // A malformed config is not a load failure — the file is present and
        // readable, it just violates the schema. Distinguish it by variant (not
        // by message text) so the UI can tell the user commits will fail until
        // the config is fixed, instead of promising a bucket-default fallback
        // that would actually error.
        Err(Error::Quilt(quilt::Error::RemoteCatalog(
            quilt::RemoteCatalogError::InvalidWorkflowsConfig(reason),
        ))) => CommitWorkflows::Invalid {
            reason,
            config_url: config_object_url(host, bucket),
        },
        Err(e) => {
            tracing::warn!(
                "Failed to load the bucket's workflows config; the caller will fall back to the bucket default: {e}"
            );
            CommitWorkflows::Unavailable
        }
    }
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
                            url: catalog_object_url(&w.config, Some(host)),
                            config_url: None,
                        })
                });
                (meta_value, meta_error, workflow)
            }
            None => (String::new(), None, None),
        };

    // Fetch the bucket's declared workflows for the commit dialog. A fetch
    // failure (or a package with no remote) must NOT fail the command — the
    // dialog still opens, and the UI presents the honest fallback state.
    let workflows = workflows_config_to_commit_workflows(
        m.get_workflows_config(&installed_package).await,
        origin_host,
        typed_uri.as_ref().map(|u| u.bucket.as_str()),
    );

    Ok(CommitData {
        namespace: namespace.to_string(),
        uri: typed_uri,
        status: pkg_status_str.to_string(),
        message,
        user_meta,
        user_meta_error,
        workflow,
        workflows,
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

// ── Bucket workflows preview for the set-remote popup ──

async fn get_bucket_workflows_from_model(
    m: &impl model::QuiltModel,
    host: Option<Host>,
    bucket: &str,
) -> CommitWorkflows {
    let config = m.get_bucket_workflows_config(host.clone(), bucket).await;
    workflows_config_to_commit_workflows(config, host.as_ref(), Some(bucket))
}

/// Fetch a bucket's declared workflows before its remote is set, so the popup
/// can present the same tri-state (`Available` / `NotConfigured` /
/// `Unavailable`) the commit dialog uses. A fetch failure maps to
/// `Unavailable` rather than an error, so the popup still opens.
#[tauri::command]
pub async fn get_bucket_workflows(
    m: tauri::State<'_, model::Model>,
    host: Option<String>,
    bucket: String,
) -> Result<CommitWorkflows, String> {
    let host = match host {
        Some(host) => Some(quilt_uri::Host::from_str(&host).map_err(|e| e.to_string())?),
        None => None,
    };
    Ok(get_bucket_workflows_from_model(&*m, host, &bucket).await)
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
        model.expect_get_workflows_config().returning(|_| Ok(None));
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
        model.expect_get_workflows_config().returning(|_| Ok(None));
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
        model.expect_get_workflows_config().returning(|_| Ok(None));
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
    async fn test_get_commit_data_workflows_available() -> Result<(), String> {
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

        // `Ok(Some(cfg))` → Available, carrying the list/default/required flag.
        let CommitWorkflows::Available {
            workflows,
            default_workflow,
            is_workflow_required,
            config_url,
        } = data.workflows
        else {
            return Err("expected Available".to_string());
        };
        assert_eq!(default_workflow, Some("dummy".to_string()));
        assert!(is_workflow_required);
        let ids: Vec<_> = workflows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, vec!["dummy", "alpha"]);
        assert_eq!(workflows[0].name.as_deref(), Some("Dummy workflow"));
        assert_eq!(workflows[0].description.as_deref(), Some("Do nothing."));
        // The package has a catalog host (test.quilt.dev) and remote bucket
        // (quilt-example), so the config object gets a catalog link. These
        // workflows declare no schemas, so their schema links stay None.
        assert_eq!(
            config_url.as_deref(),
            Some("https://test.quilt.dev/b/quilt-example/tree/.quilt/workflows/config.yml")
        );
        assert!(
            workflows
                .iter()
                .all(|w| w.metadata_schema_url.is_none() && w.entries_schema_url.is_none())
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_get_commit_data_workflows_not_configured() -> Result<(), String> {
        // `Ok(None)` (no config in the bucket) → NotConfigured: the bucket is
        // ungoverned, distinct from a fetch failure.
        let mut model = base_commit_model();
        model.expect_get_workflows_config().returning(|_| Ok(None));

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert!(matches!(data.workflows, CommitWorkflows::NotConfigured));
        Ok(())
    }

    #[tokio::test]
    async fn test_get_commit_data_workflows_unavailable() -> Result<(), String> {
        // `Err(_)` must NOT fail get_commit_data: the dialog still opens, and
        // the state is Unavailable so the UI falls back to the bucket default.
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

        assert!(matches!(data.workflows, CommitWorkflows::Unavailable));
        Ok(())
    }

    /// The tagged JSON `CommitWorkflows` serializes to is the wire contract the
    /// UI mirror (`quilt_sync_ui::commands::CommitWorkflows`) deserializes. If
    /// these strings drift, the commit dialog silently loses the workflow list.
    #[test]
    fn commit_workflows_wire_form_is_verbatim() {
        let available = CommitWorkflows::Available {
            workflows: vec![CommitWorkflowInfo {
                id: "alpha".to_string(),
                name: Some("Alpha".to_string()),
                description: None,
                metadata_schema_url: Some("https://catalog/b/bucket/tree/meta.json".to_string()),
                entries_schema_url: None,
            }],
            default_workflow: Some("alpha".to_string()),
            is_workflow_required: true,
            config_url: Some(
                "https://catalog/b/bucket/tree/.quilt/workflows/config.yml".to_string(),
            ),
        };
        assert_eq!(
            serde_json::to_value(&available).unwrap(),
            serde_json::json!({
                "state": "available",
                "workflows": [{
                    "id": "alpha",
                    "name": "Alpha",
                    "description": null,
                    "metadataSchemaUrl": "https://catalog/b/bucket/tree/meta.json",
                    "entriesSchemaUrl": null,
                }],
                "defaultWorkflow": "alpha",
                "isWorkflowRequired": true,
                "configUrl": "https://catalog/b/bucket/tree/.quilt/workflows/config.yml",
            })
        );
        assert_eq!(
            serde_json::to_value(CommitWorkflows::NotConfigured).unwrap(),
            serde_json::json!({"state": "notConfigured"})
        );
        assert_eq!(
            serde_json::to_value(CommitWorkflows::Unavailable).unwrap(),
            serde_json::json!({"state": "unavailable"})
        );
        assert_eq!(
            serde_json::to_value(CommitWorkflows::Invalid {
                reason: "bad schema".to_string(),
                config_url: Some(
                    "https://catalog/b/bucket/tree/.quilt/workflows/config.yml".to_string(),
                ),
            })
            .unwrap(),
            serde_json::json!({
                "state": "invalid",
                "reason": "bad schema",
                "configUrl": "https://catalog/b/bucket/tree/.quilt/workflows/config.yml",
            })
        );
    }

    /// A malformed workflows config (`InvalidWorkflowsConfig`) must map to the
    /// distinct `Invalid` case carrying the reason — NOT `Unavailable` — so the
    /// dialog can warn that commits will fail until the config is fixed.
    #[tokio::test]
    async fn test_get_commit_data_workflows_invalid() -> Result<(), String> {
        let mut model = base_commit_model();
        model.expect_get_workflows_config().returning(|_| {
            Err(Error::from(quilt::Error::RemoteCatalog(
                quilt::RemoteCatalogError::InvalidWorkflowsConfig(
                    "workflows/config.yml does not satisfy the workflows config schema".to_string(),
                ),
            )))
        });

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        let CommitWorkflows::Invalid { reason, config_url } = data.workflows else {
            return Err("expected Invalid".to_string());
        };
        assert!(
            reason.contains("does not satisfy the workflows config schema"),
            "reason must carry the violation, got: {reason}"
        );
        // The malformed-config notice carries a link to the config object so
        // the user can open and fix it (catalog host + remote bucket in scope).
        assert_eq!(
            config_url.as_deref(),
            Some("https://test.quilt.dev/b/quilt-example/tree/.quilt/workflows/config.yml")
        );
        Ok(())
    }

    /// A transient/network failure (an S3 error) keeps mapping to `Unavailable`
    /// — a soft "couldn't load", distinct from the malformed-config case.
    #[tokio::test]
    async fn test_get_commit_data_workflows_unavailable_on_s3_error() -> Result<(), String> {
        let mut model = base_commit_model();
        model.expect_get_workflows_config().returning(|_| {
            Err(Error::from(quilt::Error::S3(quilt::S3Error::new(
                quilt::S3ErrorKind::GetObject("timeout".to_string()),
            ))))
        });

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_commit_data_from_model(&model, &tracing, &namespace)
            .await
            .map_err(|e| e.to_string())?;

        assert!(matches!(data.workflows, CommitWorkflows::Unavailable));
        Ok(())
    }

    /// The same malformed-config discrimination on the pre-set-remote preview
    /// path (`get_bucket_workflows`): `Invalid`, not `Unavailable`.
    #[tokio::test]
    async fn test_get_bucket_workflows_invalid() -> Result<(), String> {
        let mut model = mocks::create();
        model
            .expect_get_bucket_workflows_config()
            .returning(|_, _| {
                Err(Error::from(quilt::Error::RemoteCatalog(
                    quilt::RemoteCatalogError::InvalidWorkflowsConfig(
                        "workflows/config.yml does not satisfy the workflows config schema"
                            .to_string(),
                    ),
                )))
            });

        let workflows = get_bucket_workflows_from_model(&model, None, "my-bucket").await;
        let CommitWorkflows::Invalid { reason, config_url } = workflows else {
            return Err("expected Invalid".to_string());
        };
        assert!(reason.contains("does not satisfy the workflows config schema"));
        // No catalog host was passed for this preview, so there is nothing to
        // link against and the config link is absent.
        assert!(config_url.is_none());
        Ok(())
    }

    // ── Bucket workflows preview (set-remote popup) ──

    #[tokio::test]
    async fn test_get_bucket_workflows_available() -> Result<(), String> {
        // `Ok(Some(cfg))` → Available, carrying the bucket's declared workflows.
        let mut model = mocks::create();
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
            .expect_get_bucket_workflows_config()
            .return_once(move |_, _| Ok(Some(config)));

        let workflows = get_bucket_workflows_from_model(&model, None, "my-bucket").await;

        let CommitWorkflows::Available {
            workflows,
            default_workflow,
            is_workflow_required,
            config_url,
        } = workflows
        else {
            return Err("expected Available".to_string());
        };
        assert_eq!(default_workflow, Some("dummy".to_string()));
        assert!(is_workflow_required);
        let ids: Vec<_> = workflows.iter().map(|w| w.id.as_str()).collect();
        assert_eq!(ids, vec!["dummy", "alpha"]);
        // No catalog host was passed for this preview, so there is nothing to
        // link against and the config link is absent.
        assert!(config_url.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_get_bucket_workflows_available_formats_catalog_links() -> Result<(), String> {
        // A config declaring per-workflow schemas, previewed against a catalog
        // host: the config object and each declared schema get a catalog HTTPS
        // link. The schema objects live under their own bucket, taken from the
        // `schemas` section's URLs.
        let mut model = mocks::create();
        let yaml: serde_yaml::Value = serde_yaml::from_str(
            r#"
version: "1"
default_workflow: alpha
workflows:
  alpha:
    name: Alpha
    metadata_schema: meta
    entries_schema: entries
  bare:
    name: Bare
schemas:
  meta:
    url: s3://schemas-bucket/meta.json
  entries:
    url: s3://schemas-bucket/entries.json
"#,
        )
        .map_err(|e| e.to_string())?;
        let config =
            quilt::io::remote::WorkflowsConfig::from_yaml(&yaml).map_err(|e| e.to_string())?;
        model
            .expect_get_bucket_workflows_config()
            .return_once(move |_, _| Ok(Some(config)));

        let host: quilt_uri::Host = "test.quilt.dev".parse().map_err(|_| "bad host")?;
        let workflows = get_bucket_workflows_from_model(&model, Some(host), "my-bucket").await;

        let CommitWorkflows::Available {
            workflows,
            config_url,
            ..
        } = workflows
        else {
            return Err("expected Available".to_string());
        };
        assert_eq!(
            config_url.as_deref(),
            Some("https://test.quilt.dev/b/my-bucket/tree/.quilt/workflows/config.yml")
        );
        // `alpha` declares both schemas; each is linked against its own bucket.
        let alpha = &workflows[0];
        assert_eq!(
            alpha.metadata_schema_url.as_deref(),
            Some("https://test.quilt.dev/b/schemas-bucket/tree/meta.json")
        );
        assert_eq!(
            alpha.entries_schema_url.as_deref(),
            Some("https://test.quilt.dev/b/schemas-bucket/tree/entries.json")
        );
        // `bare` declares neither schema, so it has no schema links.
        let bare = &workflows[1];
        assert!(bare.metadata_schema_url.is_none() && bare.entries_schema_url.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_get_bucket_workflows_not_configured() {
        // `Ok(None)` (ungoverned bucket) → NotConfigured.
        let mut model = mocks::create();
        model
            .expect_get_bucket_workflows_config()
            .returning(|_, _| Ok(None));

        let workflows = get_bucket_workflows_from_model(&model, None, "my-bucket").await;
        assert!(matches!(workflows, CommitWorkflows::NotConfigured));
    }

    #[tokio::test]
    async fn test_get_bucket_workflows_unavailable() {
        // `Err(_)` must NOT fail the command: the popup still opens with the
        // Unavailable fallback state.
        let mut model = mocks::create();
        model
            .expect_get_bucket_workflows_config()
            .returning(|_, _| {
                Err(Error::from(quilt::InstallPackageError::NotInstalled(
                    ("foo", "bar").into(),
                )))
            });

        let workflows = get_bucket_workflows_from_model(&model, None, "my-bucket").await;
        assert!(matches!(workflows, CommitWorkflows::Unavailable));
    }
}
