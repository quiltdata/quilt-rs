//! `get_installed_package_data` — per-package entry listing for the Leptos UI.

use serde::Serialize;

use crate::Error;
use crate::model;
use crate::quilt;
use crate::routes;

// ── Installed Package data for Leptos UI ──

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackageEntryData {
    pub filename: String,
    pub size: u64,
    pub status: String,
    pub junky_pattern: Option<String>,
    pub ignored_by: Option<String>,
    pub namespace: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
pub struct InstalledPackageData {
    pub namespace: String,
    pub uri: Option<quilt_uri::S3PackageUri>,
    pub status: String,
    /// The currently-installed revision's top-hash (the `remote` hash of the
    /// four-hash lineage). Used for the version-mismatch banner tooltip.
    pub installed_hash: Option<String>,
    /// The installed revision's manifest commit message, shown in place of the
    /// top-hash on the version-mismatch banner. `None`/empty falls back to the
    /// short hash in the UI.
    pub installed_message: Option<String>,
    /// True when the package has been pushed — `lineage.remote_uri.hash` is
    /// non-empty and the remote is now pinned to that push history. The UI
    /// uses this to switch the remote button from "Change remote" to a
    /// read-only "Show remote" view.
    pub remote_locked: bool,
    /// True when the package has a local commit. Setting a remote only
    /// re-commits (creating a new revision) when there is a commit to
    /// re-commit, so the UI gates the "creates a new revision" notice on this.
    pub has_local_commit: bool,
    pub entries: Vec<InstalledPackageEntryData>,
    pub has_remote_entries: bool,
    pub ignored_count: usize,
    pub unmodified_count: usize,
    pub filter_unmodified: bool,
    pub filter_ignored: bool,
}

/// The installed revision's display message: the manifest header's commit
/// message, or `None` when absent. The short-hash fallback is UI-owned.
fn manifest_message(manifest: &quilt::manifest::Manifest) -> Option<String> {
    manifest.header.message.clone()
}

async fn get_installed_package_data_from_model(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
    namespace: &quilt_uri::Namespace,
    filter: routes::EntriesFilter,
) -> Result<InstalledPackageData, Error> {
    let installed_package = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(
            namespace.to_owned(),
        ))
    })?;

    let lineage = m.get_installed_package_lineage(&installed_package).await?;

    let installed_hash = lineage.remote_uri.as_ref().map(|u| u.hash.clone());
    let installed_message = match installed_package.manifest().await {
        Ok(manifest) => manifest_message(&manifest),
        Err(err) => {
            tracing::warn!("Failed to read installed manifest header: {err}");
            None
        }
    };

    let typed_uri = lineage
        .remote_uri
        .as_ref()
        .map(quilt_uri::S3PackageUri::from);
    let origin_host = typed_uri.as_ref().and_then(|u| u.catalog.as_ref());
    if let Some(host) = origin_host {
        tracing.add_host(host);
    }

    let pkg_status = if lineage.remote_uri.is_none() || origin_host.is_some() {
        match m
            .get_installed_package_status(&installed_package, None)
            .await
        {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!("Failed to get package status: {err}");
                quilt::lineage::InstalledPackageStatus::error()
            }
        }
    } else {
        quilt::lineage::InstalledPackageStatus::error()
    };

    let modified_entries = &pkg_status.changes;
    let installed_paths = &lineage.paths;
    let manifest_entries = m.get_installed_package_records(&installed_package).await?;

    let junky_map: std::collections::HashMap<_, _> = pkg_status
        .junky_changes
        .iter()
        .map(|(p, pat)| (p.clone(), pat.clone()))
        .collect();

    let mut entries_list = Vec::new();
    for (filename, change) in modified_entries {
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
    for filename in installed_paths.keys() {
        if modified_entries.contains_key(filename) {
            continue;
        }
        if let Some(row) = manifest_entries.get(filename) {
            entries_list.push(InstalledPackageEntryData {
                filename: filename.display().to_string(),
                size: row.size,
                status: "pristine".to_string(),
                junky_pattern: None,
                ignored_by: None,
                namespace: namespace.to_string(),
            });
        }
        if entries_list.len() > 1000 {
            break;
        }
    }
    for (filename, row) in &manifest_entries {
        if installed_paths.contains_key(filename) || modified_entries.contains_key(filename) {
            continue;
        }
        entries_list.push(InstalledPackageEntryData {
            filename: filename.display().to_string(),
            size: row.size,
            status: "remote".to_string(),
            junky_pattern: None,
            ignored_by: None,
            namespace: namespace.to_string(),
        });
        if entries_list.len() > 1000 {
            break;
        }
    }
    for (filename, pattern, size) in &pkg_status.ignored_files {
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
    let ignored_count = pkg_status.ignored_files.len();
    let unmodified_count = installed_paths
        .keys()
        .filter(|f| !modified_entries.contains_key(*f))
        .count()
        + manifest_entries
            .keys()
            .filter(|f| !installed_paths.contains_key(*f) && !modified_entries.contains_key(*f))
            .count();

    let has_remote_entries = manifest_entries
        .keys()
        .any(|f| !installed_paths.contains_key(f) && !modified_entries.contains_key(f));

    let status_str = match pkg_status.upstream_state {
        quilt::lineage::UpstreamState::UpToDate => "up_to_date",
        quilt::lineage::UpstreamState::Ahead => "ahead",
        quilt::lineage::UpstreamState::Behind => "behind",
        quilt::lineage::UpstreamState::Diverged => "diverged",
        quilt::lineage::UpstreamState::Local => "local",
        quilt::lineage::UpstreamState::Error => "error",
    };

    let remote_locked = lineage
        .remote_uri
        .as_ref()
        .is_some_and(|r| !r.hash.is_empty());
    let has_local_commit = lineage.commit.is_some();

    Ok(InstalledPackageData {
        namespace: namespace.to_string(),
        uri: typed_uri,
        status: status_str.to_string(),
        installed_hash,
        installed_message,
        remote_locked,
        has_local_commit,
        entries: entries_list,
        has_remote_entries,
        ignored_count,
        unmodified_count,
        filter_unmodified: filter.unmodified,
        filter_ignored: filter.ignored,
    })
}

#[tauri::command]
pub async fn get_installed_package_data(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
    filter: Option<String>,
) -> Result<InstalledPackageData, String> {
    let namespace: quilt_uri::Namespace = namespace
        .try_into()
        .map_err(|e: quilt_uri::UriError| e.to_string())?;
    let filter = filter
        .map(|f| routes::EntriesFilter::from_filter_str(&f))
        .unwrap_or_default();

    get_installed_package_data_from_model(&*m, &tracing, &namespace, filter)
        .await
        .map_err(|e| e.to_frontend_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::commands::test_support::*;
    use crate::model::mocks;

    // ── Installed package data tests ──
    // (Adapted from pages/installed_package.rs: test_view, test_view_entries,
    //  test_view_no_origin, test_view_status_failed, test_view_local_only,
    //  test_view_local_with_origin_disables_catalog_button)

    #[tokio::test]
    async fn test_get_installed_package_data() -> Result<(), String> {
        let mut model = mocks::create();
        mocks::mock_installed_package(&mut model);
        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_installed_package_data_from_model(
            &model,
            &tracing,
            &namespace,
            routes::EntriesFilter::default(),
        )
        .await
        .map_err(|e| e.to_string())?;

        assert_eq!(data.namespace, "foo/bar");
        let uri = data.uri.as_ref().expect("URI present");
        assert_eq!(uri.bucket, "quilt-example");
        assert_eq!(catalog_host(&data.uri).as_deref(), Some("test.quilt.dev"));
        // Mock has one record "NAME" — should appear as an entry
        assert!(!data.entries.is_empty());
        let entry = data.entries.iter().find(|e| e.filename == "NAME");
        assert!(entry.is_some(), "Entry 'NAME' should be present");
        Ok(())
    }

    #[tokio::test]
    async fn test_get_installed_package_data_not_installed() {
        let mut model = mocks::create();
        model.expect_get_installed_package().returning(|_| Ok(None));
        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("missing", "package").into();

        let result = get_installed_package_data_from_model(
            &model,
            &tracing,
            &namespace,
            routes::EntriesFilter::default(),
        )
        .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_get_installed_package_data_no_origin() -> Result<(), String> {
        let mut model = mocks::create();

        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri_no_origin(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_installed_package_data_from_model(
            &model,
            &tracing,
            &namespace,
            routes::EntriesFilter::default(),
        )
        .await
        .map_err(|e| e.to_string())?;

        assert_eq!(data.status, "error");
        // URI is exposed (so the Set Remote popup can pre-fill bucket)
        // but its catalog is unset.
        let uri = data.uri.as_ref().expect("URI present");
        assert_eq!(uri.bucket, "test");
        assert!(uri.catalog.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_get_installed_package_data_error_with_origin() -> Result<(), String> {
        let mut model = mocks::create();

        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });
        model
            .expect_get_installed_package_status()
            .returning(|_, _| Ok(quilt::lineage::InstalledPackageStatus::error()));
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_installed_package_data_from_model(
            &model,
            &tracing,
            &namespace,
            routes::EntriesFilter::default(),
        )
        .await
        .map_err(|e| e.to_string())?;

        assert_eq!(data.status, "error");
        assert_eq!(catalog_host(&data.uri).as_deref(), Some("test.quilt.dev"));
        Ok(())
    }

    #[tokio::test]
    async fn test_get_installed_package_data_local_only() -> Result<(), String> {
        let mut model = mocks::create();

        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        // No remote URI → local-only package
        model
            .expect_get_installed_package_lineage()
            .returning(|_| Ok(quilt::lineage::PackageLineage::default()));
        model
            .expect_get_installed_package_status()
            .returning(|_, _| Ok(quilt::lineage::InstalledPackageStatus::local()));
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_installed_package_data_from_model(
            &model,
            &tracing,
            &namespace,
            routes::EntriesFilter::default(),
        )
        .await
        .map_err(|e| e.to_string())?;

        assert_eq!(data.status, "local");
        assert!(data.uri.is_none());
        Ok(())
    }

    #[tokio::test]
    async fn test_get_installed_package_data_local_with_origin() -> Result<(), String> {
        let mut model = mocks::create();

        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = quilt_uri::ManifestUri {
                    origin: Some("test.quilt.dev".parse().unwrap()),
                    bucket: "test".to_string(),
                    namespace: pkg.namespace.clone(),
                    hash: String::new(),
                };
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    String::new(),
                ))
            });
        model
            .expect_get_installed_package_status()
            .returning(|_, _| Ok(quilt::lineage::InstalledPackageStatus::local()));
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_installed_package_data_from_model(
            &model,
            &tracing,
            &namespace,
            routes::EntriesFilter::default(),
        )
        .await
        .map_err(|e| e.to_string())?;

        assert_eq!(data.status, "local");
        // Has origin for Push button and disabled Catalog button.
        assert_eq!(catalog_host(&data.uri).as_deref(), Some("test.quilt.dev"));
        Ok(())
    }

    // (Adapted from pages/installed_package.rs: test_sizes)

    #[tokio::test]
    async fn test_get_installed_package_data_entry_sizes() -> Result<(), String> {
        let mut model = mocks::create();

        model
            .expect_get_installed_package()
            .returning(move |_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });
        model
            .expect_get_installed_package_status()
            .returning(|_, _| Ok(quilt::lineage::InstalledPackageStatus::default()));

        let expected_sizes: Vec<(&str, u64)> = vec![
            ("empty.csv", 0),
            ("small.csv", 12),
            ("kilobytes.csv", 1_234),
            ("megabytes.csv", 12_345_678),
            ("petabytes.csv", 1_234_567_890_123_456),
        ];
        let records: std::collections::BTreeMap<std::path::PathBuf, quilt::manifest::ManifestRow> =
            expected_sizes
                .iter()
                .map(|(name, size)| {
                    let row = quilt::manifest::ManifestRow {
                        logical_key: std::path::PathBuf::from(name),
                        size: *size,
                        ..Default::default()
                    };
                    (std::path::PathBuf::from(name), row)
                })
                .collect();
        model
            .expect_get_installed_package_records()
            .return_once(move |_| Ok(records));

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_installed_package_data_from_model(
            &model,
            &tracing,
            &namespace,
            routes::EntriesFilter::default(),
        )
        .await
        .map_err(|e| e.to_string())?;

        assert_eq!(data.entries.len(), expected_sizes.len());
        for (name, expected_size) in &expected_sizes {
            let entry = data
                .entries
                .iter()
                .find(|e| e.filename == *name)
                .unwrap_or_else(|| panic!("Entry '{name}' should be present"));
            assert_eq!(entry.size, *expected_size, "Size mismatch for '{name}'");
        }
        Ok(())
    }

    #[tokio::test]
    async fn includes_installed_identity() -> Result<(), String> {
        let mut model = mocks::create();
        mocks::mock_installed_package(&mut model);
        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_installed_package_data_from_model(
            &model,
            &tracing,
            &namespace,
            routes::EntriesFilter::default(),
        )
        .await
        .map_err(|e| e.to_string())?;

        // `installed_hash` comes from the model-mocked lineage's remote_uri,
        // which `mock_installed_package` sets to a concrete hash.
        assert_eq!(
            data.installed_hash.as_deref(),
            Some("6c3758a4d2bf8fe730be5d12f5e095950dc123c373f55f66ca4b3ced74772b22")
        );
        // `installed_message` is read from `installed_package.manifest()`, a
        // real (unmocked) `quilt::InstalledPackage` domain call. The package
        // built by `mock_installed_package` (via `LocalDomain::new(PathBuf::new())`)
        // has no on-disk lineage file backing it, so reading its manifest
        // fails and the best-effort fallback yields `None`.
        assert_eq!(data.installed_message, None);
        Ok(())
    }

    #[test]
    fn manifest_message_projects_header_message() {
        let mut manifest = quilt::manifest::Manifest::default();
        manifest.header.message = Some("Add benchling report".to_string());

        assert_eq!(
            manifest_message(&manifest),
            Some("Add benchling report".to_string())
        );
    }

    #[test]
    fn manifest_message_none_when_header_message_absent() {
        let mut manifest = quilt::manifest::Manifest::default();
        manifest.header.message = None;

        assert_eq!(manifest_message(&manifest), None);
    }
}
