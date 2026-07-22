//! Installed-packages list (light phase) and per-package status refresh
//! (heavy phase) for the Leptos UI.

use std::collections::HashMap;

use serde::Serialize;

use crate::Error;
use crate::autopull::Watcher;
use crate::model;
use crate::quilt;

// ── Installed Packages List data for Leptos UI ──

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackagesListData {
    pub packages: Vec<InstalledPackageListItem>,
}

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct InstalledPackageListItem {
    pub namespace: String,
    pub status: String,
    pub has_changes: bool,
    /// True when the package has a local commit. Setting a remote only
    /// re-commits (creating a new revision) when there is a commit to
    /// re-commit, so the UI gates the "creates a new revision" notice on this.
    pub has_local_commit: bool,
    pub uri: Option<quilt_uri::S3PackageUri>,
    /// Raw `lineage.remote_uri` rendering, kept separate from `uri` so
    /// the UI can still surface a misconfigured remote when origin
    /// resolution fails (status: "error" branch).
    pub remote_display: Option<String>,
    /// The autosync watcher's `Other` pause message for this namespace,
    /// if it is currently paused for a reason the status string cannot
    /// carry (workflow refusal, hash mismatch, etc.); `None` otherwise.
    ///
    /// Read straight from the watcher's paused map (the single source of
    /// truth) at fetch time, so the UI derives the red/hint state from
    /// authoritative data instead of a reconciled frontend cache.
    pub paused_reason: Option<String>,
}

async fn get_installed_packages_list_data_from_model(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
    paused_reasons: &HashMap<String, String>,
) -> Result<InstalledPackagesListData, Error> {
    let list = m.get_installed_packages_list().await?;
    let mut packages = Vec::new();
    for installed_package in list {
        match load_package_item(m, tracing, &installed_package, paused_reasons).await {
            Ok(item) => packages.push(item),
            Err(err) => {
                tracing::warn!(
                    "Failed to load package {}: {err}",
                    installed_package.namespace,
                );
            }
        }
    }
    Ok(InstalledPackagesListData { packages })
}

async fn load_package_item(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
    installed_package: &quilt::InstalledPackage,
    paused_reasons: &HashMap<String, String>,
) -> Result<InstalledPackageListItem, Error> {
    let namespace = installed_package.namespace.to_string();
    let paused_reason = paused_reasons.get(&namespace).cloned();
    let lineage = m.get_installed_package_lineage(installed_package).await?;
    // Computed before `lineage` is moved by the `into()` below.
    let has_local_commit = lineage.commit.is_some();

    let Some(remote_uri) = lineage.remote_uri.as_ref() else {
        return Ok(InstalledPackageListItem {
            namespace,
            status: "local".to_string(),
            has_changes: false,
            has_local_commit,
            uri: None,
            remote_display: None,
            paused_reason,
        });
    };

    let typed_uri = quilt_uri::S3PackageUri::from(remote_uri);

    if remote_uri.origin.is_none() {
        return Ok(InstalledPackageListItem {
            namespace,
            status: "error".to_string(),
            has_changes: false,
            has_local_commit,
            uri: Some(typed_uri),
            remote_display: Some(remote_uri.to_string()),
            paused_reason,
        });
    }

    if let Some(host) = typed_uri.catalog.as_ref() {
        tracing.add_host(host);
    }
    let remote_display = remote_uri.to_string();
    let upstream_state: quilt::lineage::UpstreamState = lineage.into();
    let has_changes = false; // Refined by refresh_package_status

    Ok(InstalledPackageListItem {
        namespace,
        status: upstream_state.to_string(),
        has_changes,
        has_local_commit,
        uri: Some(typed_uri),
        remote_display: Some(remote_display),
        paused_reason,
    })
}

#[tauri::command]
pub async fn get_installed_packages_list_data(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    watcher: tauri::State<'_, Watcher>,
) -> Result<InstalledPackagesListData, String> {
    // Read the watcher's paused map — the single source of truth — the
    // same way `get_autosync_snapshot` does. Only `Other`-reason pauses
    // carry a `message`; those are the reasons the status string cannot
    // convey, so they are the only ones surfaced on each row.
    let paused_reasons: HashMap<String, String> = watcher
        .snapshot()
        .await
        .paused
        .into_iter()
        .filter_map(|entry| entry.message.map(|message| (entry.namespace, message)))
        .collect();

    get_installed_packages_list_data_from_model(&*m, &tracing, &paused_reasons)
        .await
        .map_err(|e| e.to_frontend_string())
}

// ── Refresh package status (heavy phase) ──

#[derive(Serialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct RefreshedPackageStatus {
    pub status: String,
    pub has_changes: bool,
}

async fn refresh_package_status_from_model(
    m: &impl model::QuiltModel,
    tracing: &crate::telemetry::Telemetry,
    namespace: &quilt_uri::Namespace,
) -> Result<RefreshedPackageStatus, Error> {
    let installed_package = m.get_installed_package(namespace).await?.ok_or_else(|| {
        Error::from(quilt::InstallPackageError::NotInstalled(
            namespace.to_owned(),
        ))
    })?;

    let lineage = m.get_installed_package_lineage(&installed_package).await?;

    let Some(remote_uri) = lineage.remote_uri.as_ref() else {
        let has_changes = match m
            .get_installed_package_status(&installed_package, None)
            .await
        {
            Ok(s) => !s.changes.is_empty(),
            Err(err) => {
                tracing::warn!(
                    "Failed to get status for {}: {err}",
                    installed_package.namespace,
                );
                false
            }
        };
        return Ok(RefreshedPackageStatus {
            status: "local".to_string(),
            has_changes,
        });
    };
    if remote_uri.origin.is_none() {
        return Ok(RefreshedPackageStatus {
            status: "error".to_string(),
            has_changes: false,
        });
    }

    if let Some(host) = remote_uri.origin.as_ref() {
        tracing.add_host(host);
    }

    let (upstream_state, has_changes) = match m
        .get_installed_package_status(&installed_package, None)
        .await
    {
        Ok(s) => (s.upstream_state, !s.changes.is_empty()),
        Err(err) => {
            tracing::warn!(
                "Failed to get status for {}: {err}",
                installed_package.namespace,
            );
            (quilt::lineage::UpstreamState::Error, false)
        }
    };

    Ok(RefreshedPackageStatus {
        status: upstream_state.to_string(),
        has_changes,
    })
}

#[tauri::command]
pub async fn refresh_package_status(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<RefreshedPackageStatus, String> {
    let namespace: quilt_uri::Namespace = namespace
        .try_into()
        .map_err(|e: quilt_uri::UriError| e.to_string())?;

    refresh_package_status_from_model(&*m, &tracing, &namespace)
        .await
        .map_err(|e| e.to_frontend_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::commands::test_support::*;
    use crate::model::mocks;

    #[tokio::test]
    async fn test_get_installed_packages_list_data_empty() -> Result<(), String> {
        let mut model = mocks::create();
        mocks::mock_installed_packages_list(&mut model);
        let tracing = crate::telemetry::Telemetry::default();

        let data = get_installed_packages_list_data_from_model(&model, &tracing, &HashMap::new())
            .await
            .map_err(|e| e.to_string())?;

        assert!(data.packages.is_empty());
        Ok(())
    }

    #[tokio::test]
    async fn test_installed_packages_list_data_statuses() -> Result<(), String> {
        let mut model = mocks::create();

        let pkgs = vec![
            make_installed_package(("test", "ahead")),
            make_installed_package(("test", "behind")),
            make_installed_package(("test", "diverged")),
            make_installed_package(("test", "uptodate")),
        ];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));

        // Set up lineage so From<PackageLineage> produces the expected status.
        // Status is derived from base_hash vs current_hash (ahead) and
        // base_hash vs latest_hash (behind).
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let ns = pkg.namespace.to_string();
                let uri = make_manifest_uri(&ns);
                // base_hash comes from uri.hash ("abcdef")
                let lineage = match ns.as_str() {
                    // Ahead: current_hash != base_hash, base_hash == latest_hash
                    "test/ahead" => {
                        let mut l =
                            quilt::lineage::PackageLineage::from_remote(uri, "abcdef".into());
                        l.commit = Some(quilt::lineage::CommitState {
                            hash: "local1".into(),
                            ..Default::default()
                        });
                        l
                    }
                    // Behind: base_hash != latest_hash, current_hash == base_hash
                    "test/behind" => {
                        quilt::lineage::PackageLineage::from_remote(uri, "remote1".into())
                    }
                    // Diverged: both ahead and behind
                    "test/diverged" => {
                        let mut l =
                            quilt::lineage::PackageLineage::from_remote(uri, "remote2".into());
                        l.commit = Some(quilt::lineage::CommitState {
                            hash: "local2".into(),
                            ..Default::default()
                        });
                        l
                    }
                    // UpToDate: all hashes match
                    _ => quilt::lineage::PackageLineage::from_remote(uri, "abcdef".into()),
                };
                Ok(lineage)
            });

        let tracing = crate::telemetry::Telemetry::default();
        let data = get_installed_packages_list_data_from_model(&model, &tracing, &HashMap::new())
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(data.packages.len(), 4);

        let find = |ns: &str| data.packages.iter().find(|p| p.namespace == ns).unwrap();

        // Spot-check URI propagation on one package; the other packages
        // share the same fixture shape, and `S3PackageUri::from(&ManifestUri)`
        // is exercised by quilt-uri's own tests.
        let ahead = find("test/ahead");
        assert_eq!(ahead.status, "ahead");
        assert!(!ahead.has_changes); // Light phase always returns false
        let ahead_uri = ahead.uri.as_ref().expect("URI present");
        assert_eq!(ahead_uri.bucket, "test");
        assert_eq!(ahead_uri.namespace.to_string(), "test/ahead");
        assert_eq!(
            catalog_host(ahead.uri.as_ref()).as_deref(),
            Some("test.quilt.dev")
        );
        assert!(ahead.remote_display.is_some());

        // For the rest, only the status mapping is the point of this test.
        assert_eq!(find("test/behind").status, "behind");
        assert_eq!(find("test/diverged").status, "diverged");
        assert_eq!(find("test/uptodate").status, "up_to_date");

        Ok(())
    }

    #[tokio::test]
    async fn test_installed_packages_list_data_with_origin_shows_cached_status()
    -> Result<(), String> {
        let mut model = mocks::create();

        let pkgs = vec![make_installed_package(("test", "pkg"))];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));

        // Lineage indicates up_to_date (base == latest == remote hash)
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let data = get_installed_packages_list_data_from_model(&model, &tracing, &HashMap::new())
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(data.packages.len(), 1);
        let pkg = &data.packages[0];
        assert_eq!(pkg.namespace, "test/pkg");
        // Light phase derives status from lineage (up_to_date, not error)
        assert_eq!(pkg.status, "up_to_date");
        assert!(!pkg.has_changes); // Always false in light phase
        assert_eq!(
            catalog_host(pkg.uri.as_ref()).as_deref(),
            Some("test.quilt.dev")
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_installed_packages_list_data_no_origin() -> Result<(), String> {
        let mut model = mocks::create();

        let pkgs = vec![make_installed_package(("test", "noorigin"))];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));

        // Remote URI exists but has no origin → triggers early return with error status
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri_no_origin(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let data = get_installed_packages_list_data_from_model(&model, &tracing, &HashMap::new())
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(data.packages.len(), 1);
        let pkg = &data.packages[0];
        assert_eq!(pkg.namespace, "test/noorigin");
        assert_eq!(pkg.status, "error");
        // URI is exposed (so the "Set remote" popup can pre-fill bucket)
        // but its catalog is unset.
        let pkg_uri = pkg.uri.as_ref().expect("URI present");
        assert_eq!(pkg_uri.bucket, "test");
        assert!(pkg_uri.catalog.is_none());
        // remote_display should still be present
        assert!(pkg.remote_display.is_some());

        Ok(())
    }

    #[tokio::test]
    async fn test_installed_packages_list_data_local_without_remote() -> Result<(), String> {
        let mut model = mocks::create();

        let pkgs = vec![make_installed_package(("test", "local"))];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));

        // No remote_uri at all → local-only package
        model
            .expect_get_installed_package_lineage()
            .returning(|_| Ok(quilt::lineage::PackageLineage::default()));

        let tracing = crate::telemetry::Telemetry::default();
        let data = get_installed_packages_list_data_from_model(&model, &tracing, &HashMap::new())
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(data.packages.len(), 1);
        let pkg = &data.packages[0];
        assert_eq!(pkg.namespace, "test/local");
        assert_eq!(pkg.status, "local");
        assert!(pkg.uri.is_none());
        assert!(pkg.remote_display.is_none());

        Ok(())
    }

    #[tokio::test]
    async fn test_installed_packages_list_data_local_with_origin() -> Result<(), String> {
        let mut model = mocks::create();

        let pkgs = vec![make_installed_package(("test", "localpush"))];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));

        // Has remote URI with origin but never pushed (empty hash → Local)
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

        let tracing = crate::telemetry::Telemetry::default();
        let data = get_installed_packages_list_data_from_model(&model, &tracing, &HashMap::new())
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(data.packages.len(), 1);
        let pkg = &data.packages[0];
        assert_eq!(pkg.namespace, "test/localpush");
        assert_eq!(pkg.status, "local");
        assert!(!pkg.has_changes);
        // Has origin (for Push button and disabled Catalog button in UI).
        assert_eq!(
            catalog_host(pkg.uri.as_ref()).as_deref(),
            Some("test.quilt.dev")
        );

        Ok(())
    }

    // ── refresh_package_status tests (heavy phase) ──

    #[tokio::test]
    async fn test_refresh_package_status_local_only_no_changes() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "local"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
        model
            .expect_get_installed_package_lineage()
            .returning(|_| Ok(quilt::lineage::PackageLineage::default()));
        model
            .expect_get_installed_package_status()
            .returning(|_, _| {
                Ok(quilt::lineage::InstalledPackageStatus::new(
                    quilt::lineage::UpstreamState::Local,
                    quilt::lineage::ChangeSet::new(),
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "local").into();
        let result = refresh_package_status_from_model(&model, &tracing, &ns)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "local");
        assert!(!result.has_changes);
        Ok(())
    }

    #[tokio::test]
    async fn test_refresh_package_status_local_only_with_changes() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "local"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
        model
            .expect_get_installed_package_lineage()
            .returning(|_| Ok(quilt::lineage::PackageLineage::default()));
        model
            .expect_get_installed_package_status()
            .returning(|_, _| {
                let mut changes = quilt::lineage::ChangeSet::new();
                changes.insert(
                    std::path::PathBuf::from("file.txt"),
                    quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
                );
                Ok(quilt::lineage::InstalledPackageStatus::new(
                    quilt::lineage::UpstreamState::Local,
                    changes,
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "local").into();
        let result = refresh_package_status_from_model(&model, &tracing, &ns)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "local");
        assert!(result.has_changes);
        Ok(())
    }

    #[tokio::test]
    async fn test_refresh_package_status_no_origin() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "noorigin"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri_no_origin(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "noorigin").into();
        let result = refresh_package_status_from_model(&model, &tracing, &ns)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "error");
        assert!(!result.has_changes);
        Ok(())
    }

    #[tokio::test]
    async fn test_refresh_package_status_with_changes() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "changed"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
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
            .returning(|_, _| {
                let mut changes = quilt::lineage::ChangeSet::new();
                changes.insert(
                    std::path::PathBuf::from("file.txt"),
                    quilt::lineage::Change::Added(quilt::manifest::ManifestRow::default()),
                );
                Ok(quilt::lineage::InstalledPackageStatus::new(
                    quilt::lineage::UpstreamState::UpToDate,
                    changes,
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "changed").into();
        let result = refresh_package_status_from_model(&model, &tracing, &ns)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "up_to_date");
        assert!(result.has_changes);
        Ok(())
    }

    #[tokio::test]
    async fn test_refresh_package_status_without_changes() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "clean"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "remote1".to_string(),
                ))
            });
        model
            .expect_get_installed_package_status()
            .returning(|_, _| {
                Ok(quilt::lineage::InstalledPackageStatus::new(
                    quilt::lineage::UpstreamState::Behind,
                    quilt::lineage::ChangeSet::default(),
                ))
            });

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "clean").into();
        let result = refresh_package_status_from_model(&model, &tracing, &ns)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "behind");
        assert!(!result.has_changes);
        Ok(())
    }

    #[tokio::test]
    async fn test_refresh_package_status_error_on_status_fetch() -> Result<(), String> {
        let mut model = mocks::create();
        let pkg = make_installed_package(("test", "broken"));
        model
            .expect_get_installed_package()
            .return_once(move |_| Ok(Some(pkg)));
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
            .returning(|_, _| Err(crate::error::Error::General("network error".to_string())));

        let tracing = crate::telemetry::Telemetry::default();
        let ns = ("test", "broken").into();
        let result = refresh_package_status_from_model(&model, &tracing, &ns)
            .await
            .map_err(|e| e.to_string())?;

        assert_eq!(result.status, "error");
        assert!(!result.has_changes);
        Ok(())
    }

    // ── paused_reason population (data-driven red state) ──

    #[tokio::test]
    async fn test_installed_packages_list_data_populates_paused_reason() -> Result<(), String> {
        let mut model = mocks::create();

        let pkgs = vec![
            make_installed_package(("test", "paused")),
            make_installed_package(("test", "clean")),
        ];
        model
            .expect_get_installed_packages_list()
            .return_once(move || Ok(pkgs));
        model
            .expect_get_installed_package_lineage()
            .returning(|pkg| {
                let uri = make_manifest_uri(&pkg.namespace.to_string());
                Ok(quilt::lineage::PackageLineage::from_remote(
                    uri,
                    "abcdef".to_string(),
                ))
            });

        // Stand in for the watcher's paused map: only the paused namespace
        // has an `Other` message.
        let mut paused_reasons = HashMap::new();
        paused_reasons.insert(
            "test/paused".to_string(),
            "workflow rejected metadata".to_string(),
        );

        let tracing = crate::telemetry::Telemetry::default();
        let data = get_installed_packages_list_data_from_model(&model, &tracing, &paused_reasons)
            .await
            .map_err(|e| e.to_string())?;

        let find = |ns: &str| data.packages.iter().find(|p| p.namespace == ns).unwrap();
        assert_eq!(
            find("test/paused").paused_reason.as_deref(),
            Some("workflow rejected metadata"),
        );
        assert!(find("test/clean").paused_reason.is_none());
        Ok(())
    }

    /// The serialized row must be byte-identical to what the UI mirror
    /// (`quilt_sync_ui::commands::PackageItemData`) deserializes in its
    /// `package_item_data_wire_form_is_verbatim`. If the two drift, the
    /// list silently drops the pause reason (or a whole field) at the
    /// Tauri boundary.
    #[test]
    fn package_item_data_wire_form_is_verbatim() {
        let item = InstalledPackageListItem {
            namespace: "acme/data".to_string(),
            status: "paused".to_string(),
            has_changes: false,
            has_local_commit: false,
            uri: None,
            remote_display: None,
            paused_reason: Some("workflow rejected metadata".to_string()),
        };
        assert_eq!(
            serde_json::to_string(&item).unwrap(),
            r#"{"namespace":"acme/data","status":"paused","hasChanges":false,"hasLocalCommit":false,"uri":null,"remoteDisplay":null,"pausedReason":"workflow rejected metadata"}"#
        );
    }
}
