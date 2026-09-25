//! `get_installed_package_data` — per-package entry listing for the Leptos UI.

use serde::Serialize;

use crate::Error;
use crate::commands::RoleCache;
use crate::experimental_settings::ExperimentalSettings;
use crate::experimental_settings::SharedExperimentalSettings;
use crate::model;
use crate::quilt;
use crate::routes;

use super::package_entries::{EntryCounts, EntryList, InstalledPackageEntryData, entry_list};
use super::package_list::denied_mark;

// ── Installed Package data for Leptos UI ──

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
#[allow(clippy::struct_excessive_bools)]
pub struct InstalledPackageData {
    pub namespace: quilt_uri::Namespace,
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
    /// Why the active role cannot reach this package's bucket, worded exactly
    /// as the roster words it. `Some` only on a denial.
    ///
    /// It is not an auth failure — the credentials vended fine — so the page
    /// must state the fact and must **not** offer Login: signing in again
    /// re-vends the same denied role, which is the loop this replaces.
    pub no_access_reason: Option<String>,
    /// Sorted by path, then capped at [`super::package_entries::ENTRIES_CAP`].
    pub entries: Vec<InstalledPackageEntryData>,
    /// Whole-package facet counts; `counts.all + counts.ignored == total`.
    pub counts: EntryCounts,
    /// Every entry the package has, ignored ones included, before the cap.
    pub total: usize,
    /// Whether the cap dropped entries: `total > entries.len()`. Set here so
    /// the UI never infers truncation from a length.
    pub truncated: bool,
    pub has_remote_entries: bool,
    pub ignored_count: usize,
    pub unmodified_count: usize,
    pub filter_unmodified: bool,
    pub filter_ignored: bool,
    /// This package's standing sync scope: `true` once the user has asked for
    /// the whole package. Reported raw — the screen renders the choice, and
    /// `entire_package_sync_enabled` decides whether it may be shown at all.
    pub syncs_entire_package: bool,
    /// Whether the experiment that offers the scope control is on. `false`
    /// means the screen renders exactly as it did before the feature existed,
    /// whatever `syncs_entire_package` says.
    pub entire_package_sync_enabled: bool,
}

/// The installed revision's display message: the manifest header's commit
/// message, or `None` when absent. The short-hash fallback is UI-owned.
fn manifest_message(manifest: &quilt::manifest::Manifest) -> Option<String> {
    manifest.header.message.clone()
}

#[allow(clippy::too_many_lines, reason = "cohesive package-data assembly")]
async fn get_installed_package_data_from_model(
    m: &impl model::QuiltModel,
    experimental: &ExperimentalSettings,
    roles: &RoleCache,
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

    let mut no_access_reason = None;
    let pkg_status = if lineage.remote_uri.is_none() || origin_host.is_some() {
        match m
            .get_installed_package_status(&installed_package, None)
            .await
        {
            Ok(s) => s,
            // A denial is not a broken session: the credentials vended, and
            // the active role simply cannot read this bucket. Collapsing it
            // into the synthetic error status makes the page say "sign in
            // again", and the re-vend hands back the same denied role — the
            // unrecoverable loop this page must not reproduce. State the
            // fact instead, and keep the entries: they come from the cached
            // manifest and the working tree, neither of which was refused.
            Err(err) if err.is_access_denied() => {
                no_access_reason = denied_mark(m, roles, origin_host).await.reason;
                match m.recompute_local_status(&installed_package, None).await {
                    Ok(status) => status,
                    Err(err) => {
                        tracing::warn!("Failed to recompute local status: {err}");
                        quilt::lineage::InstalledPackageStatus::error()
                    }
                }
            }
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

    let EntryList {
        entries: entries_list,
        counts,
        total,
        truncated,
    } = entry_list(namespace, &pkg_status, installed_paths, &manifest_entries);

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
        namespace: namespace.clone(),
        uri: typed_uri,
        status: status_str.to_string(),
        installed_hash,
        installed_message,
        remote_locked,
        has_local_commit,
        no_access_reason,
        entries: entries_list,
        counts,
        total,
        truncated,
        has_remote_entries,
        ignored_count,
        unmodified_count,
        filter_unmodified: filter.unmodified,
        filter_ignored: filter.ignored,
        syncs_entire_package: lineage.sync_scope.covers_untracked(),
        entire_package_sync_enabled: experimental.entire_package_sync,
    })
}

#[tauri::command]
pub async fn get_installed_package_data(
    m: tauri::State<'_, model::Model>,
    roles: tauri::State<'_, RoleCache>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    experimental: tauri::State<'_, SharedExperimentalSettings>,
    namespace: String,
    filter: Option<String>,
) -> Result<InstalledPackageData, String> {
    let namespace: quilt_uri::Namespace = namespace
        .try_into()
        .map_err(|e: quilt_uri::UriError| e.to_string())?;
    let filter = filter
        .map(|f| routes::EntriesFilter::from_filter_str(&f))
        .unwrap_or_default();

    let experimental = experimental.read().await.clone();
    get_installed_package_data_from_model(&*m, &experimental, &roles, &tracing, &namespace, filter)
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
            &ExperimentalSettings::default(),
            &RoleCache::default(),
            &tracing,
            &namespace,
            routes::EntriesFilter::default(),
        )
        .await
        .map_err(|e| e.to_string())?;

        assert_eq!(data.namespace.to_string(), "foo/bar");
        let uri = data.uri.as_ref().expect("URI present");
        assert_eq!(uri.bucket, "quilt-example");
        assert_eq!(
            catalog_host(data.uri.as_ref()).as_deref(),
            Some("test.quilt.dev")
        );
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
            &ExperimentalSettings::default(),
            &RoleCache::default(),
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
            &ExperimentalSettings::default(),
            &RoleCache::default(),
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
            &ExperimentalSettings::default(),
            &RoleCache::default(),
            &tracing,
            &namespace,
            routes::EntriesFilter::default(),
        )
        .await
        .map_err(|e| e.to_string())?;

        assert_eq!(data.status, "error");
        assert_eq!(
            catalog_host(data.uri.as_ref()).as_deref(),
            Some("test.quilt.dev")
        );
        Ok(())
    }

    /// The detail page used to map *every* status failure onto the synthetic
    /// error status, which the banner renders as "Unable to check remote
    /// status" plus a Login button. On a denial that is the re-login loop:
    /// the re-vend hands back the same role that was just refused. The page
    /// must state the fact and name the role instead.
    #[tokio::test]
    async fn a_denial_states_the_role_instead_of_asking_for_a_new_login() -> Result<(), String> {
        let mut model = mocks::create();

        model
            .expect_get_installed_package()
            .returning(|_| Ok(Some(make_installed_package(("foo", "bar")))));
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
            .returning(|_, _| Err(access_denied_error()));
        model.expect_recompute_local_status().returning(|_, _| {
            Ok(quilt::lineage::InstalledPackageStatus::new(
                quilt::lineage::UpstreamState::UpToDate,
                one_local_change(),
            ))
        });
        model
            .expect_get_installed_package_records()
            .returning(|_| Ok(std::collections::BTreeMap::new()));
        model.expect_refresh_roles().returning(|_| {
            Ok(quilt_rs::RoleInfo {
                current: "ReadOnly".to_string(),
                available: vec!["ReadWrite".to_string(), "ReadOnly".to_string()],
            })
        });
        model.expect_clear_remote_client_cache().returning(|_| ());

        let tracing = crate::telemetry::Telemetry::default();
        let namespace = ("foo", "bar").into();

        let data = get_installed_package_data_from_model(
            &model,
            &ExperimentalSettings::default(),
            &RoleCache::default(),
            &tracing,
            &namespace,
            routes::EntriesFilter::default(),
        )
        .await
        .map_err(|e| e.to_string())?;

        assert_ne!(
            data.status, "error",
            "the error status is what puts a Login button on the page"
        );
        assert_eq!(
            data.no_access_reason.as_deref(),
            Some("Current role ReadOnly has no access to this bucket"),
            "the page must say the same thing the roster says"
        );
        assert!(
            data.entries.iter().any(|e| e.filename == "file.txt"),
            "the working tree was not refused, so the entries survive"
        );
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
            &ExperimentalSettings::default(),
            &RoleCache::default(),
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
            &ExperimentalSettings::default(),
            &RoleCache::default(),
            &tracing,
            &namespace,
            routes::EntriesFilter::default(),
        )
        .await
        .map_err(|e| e.to_string())?;

        assert_eq!(data.status, "local");
        // Has origin for Push button and disabled Catalog button.
        assert_eq!(
            catalog_host(data.uri.as_ref()).as_deref(),
            Some("test.quilt.dev")
        );
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
            &ExperimentalSettings::default(),
            &RoleCache::default(),
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
            &ExperimentalSettings::default(),
            &RoleCache::default(),
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

    /// A package's files by class, each a list of logical paths. The fixture
    /// places each class where the page read finds it: changes in the status,
    /// downloaded files in lineage paths and the manifest, not-downloaded ones
    /// in the manifest only, ignored ones in the local walk.
    #[derive(Default)]
    struct Files {
        added: Vec<String>,
        modified: Vec<String>,
        deleted: Vec<String>,
        pristine: Vec<String>,
        remote: Vec<String>,
        ignored: Vec<String>,
    }

    fn numbered(prefix: &str, n: usize) -> Vec<String> {
        (0..n).map(|i| format!("{prefix}{i:04}")).collect()
    }

    async fn read_package(files: Files) -> Result<InstalledPackageData, String> {
        use std::path::PathBuf;

        let row = |path: &String| quilt::manifest::ManifestRow {
            logical_key: PathBuf::from(path),
            size: 1,
            ..Default::default()
        };
        let mut changes = quilt::lineage::ChangeSet::new();
        for path in &files.added {
            changes.insert(path.into(), quilt::lineage::Change::Added(row(path)));
        }
        for path in &files.modified {
            changes.insert(path.into(), quilt::lineage::Change::Modified(row(path)));
        }
        for path in &files.deleted {
            changes.insert(path.into(), quilt::lineage::Change::Removed(row(path)));
        }
        let mut status = quilt::lineage::InstalledPackageStatus::new(
            quilt::lineage::UpstreamState::UpToDate,
            changes,
        );
        status.ignored_files = files
            .ignored
            .iter()
            .map(|path| (path.into(), "*.tmp".to_string(), 1))
            .collect();

        let downloaded: Vec<&String> = files
            .modified
            .iter()
            .chain(&files.deleted)
            .chain(&files.pristine)
            .collect();
        let paths: quilt::lineage::LineagePaths = downloaded
            .iter()
            .map(|path| (PathBuf::from(path), quilt::lineage::PathState::default()))
            .collect();
        let records: std::collections::BTreeMap<PathBuf, quilt::manifest::ManifestRow> = downloaded
            .into_iter()
            .chain(&files.remote)
            .map(|path| (PathBuf::from(path), row(path)))
            .collect();

        let mut model = mocks::create();
        model
            .expect_get_installed_package()
            .returning(|_| Ok(Some(make_installed_package(("foo", "bar")))));
        model
            .expect_get_installed_package_lineage()
            .returning(move |pkg| {
                let mut lineage = quilt::lineage::PackageLineage::from_remote(
                    make_manifest_uri(&pkg.namespace.to_string()),
                    "abcdef".to_string(),
                );
                lineage.paths = paths.clone();
                Ok(lineage)
            });
        model
            .expect_get_installed_package_status()
            .return_once(move |_, _| Ok(status));
        model
            .expect_get_installed_package_records()
            .return_once(move |_| Ok(records));

        get_installed_package_data_from_model(
            &model,
            &ExperimentalSettings::default(),
            &RoleCache::default(),
            &crate::telemetry::Telemetry::default(),
            &("foo", "bar").into(),
            routes::EntriesFilter::default(),
        )
        .await
        .map_err(|e| e.to_string())
    }

    async fn read_remote_only(n: usize) -> Result<InstalledPackageData, String> {
        read_package(Files {
            remote: numbered("f", n),
            ..Files::default()
        })
        .await
    }

    #[tokio::test]
    async fn under_the_cap_every_file_is_listed() -> Result<(), String> {
        let data = read_remote_only(999).await?;
        assert_eq!(data.entries.len(), 999);
        assert_eq!(data.total, 999);
        assert!(!data.truncated);
        Ok(())
    }

    #[tokio::test]
    async fn exactly_the_cap_is_not_truncated() -> Result<(), String> {
        let data = read_remote_only(1000).await?;
        assert_eq!(data.entries.len(), 1000);
        assert_eq!(data.total, 1000);
        assert!(!data.truncated, "nothing was dropped at exactly 1000");
        Ok(())
    }

    #[tokio::test]
    async fn one_over_the_cap_drops_one_file() -> Result<(), String> {
        let data = read_remote_only(1001).await?;
        assert_eq!(data.entries.len(), 1000);
        assert_eq!(data.total, 1001);
        assert!(data.truncated);
        Ok(())
    }

    #[tokio::test]
    async fn two_over_the_cap_drops_two_files() -> Result<(), String> {
        let data = read_remote_only(1002).await?;
        assert_eq!(data.entries.len(), 1000);
        assert_eq!(data.total, 1002);
        assert!(data.truncated);
        assert_eq!(
            data.entries.last().map(|e| e.filename.as_str()),
            Some("f0999"),
            "the kept rows are the first 1000 paths"
        );
        Ok(())
    }

    /// v1 filled the list class by class, changes first, so a package over
    /// the cap listed its changed files and dropped paths that sort before
    /// them. The cap now keeps the first 1000 paths, whatever their class.
    #[tokio::test]
    async fn the_cap_keeps_the_first_paths_whatever_their_class() -> Result<(), String> {
        let data = read_package(Files {
            added: numbered("z/added-", 300),
            modified: numbered("y/modified-", 200),
            deleted: numbered("x/deleted-", 200),
            remote: numbered("a/remote-", 700),
            ignored: numbered("b/ignored-", 10),
            pristine: numbered("c/pristine-", 10),
        })
        .await?;

        let kept: Vec<&str> = data.entries.iter().map(|e| e.filename.as_str()).collect();
        assert_eq!(kept.len(), 1000);
        assert_eq!(kept.first(), Some(&"a/remote-0000"));
        assert_eq!(kept[699], "a/remote-0699");
        assert_eq!(kept[700], "b/ignored-0000");
        assert_eq!(kept[710], "c/pristine-0000");
        assert_eq!(kept[720], "x/deleted-0000");
        assert_eq!(kept[920], "y/modified-0000");
        assert_eq!(kept.last(), Some(&"y/modified-0079"));
        assert!(
            !kept.iter().any(|f| f.starts_with("z/")),
            "every added file sorts past the cap"
        );
        assert_eq!(data.total, 1420);
        assert!(data.truncated);
        Ok(())
    }

    #[tokio::test]
    async fn counts_cover_the_whole_package_when_the_list_is_cut() -> Result<(), String> {
        let data = read_package(Files {
            added: numbered("z/added-", 300),
            modified: numbered("y/modified-", 200),
            deleted: numbered("x/deleted-", 200),
            remote: numbered("a/remote-", 700),
            ignored: numbered("b/ignored-", 10),
            pristine: numbered("c/pristine-", 10),
        })
        .await?;

        assert!(data.truncated);
        assert_eq!(
            data.counts,
            EntryCounts {
                all: 1410,
                changed: 700,
                not_downloaded: 700,
                ignored: 10,
            }
        );
        assert_eq!(data.total, data.counts.all + data.counts.ignored);
        Ok(())
    }

    #[tokio::test]
    async fn counts_split_the_package_into_disjoint_facets() -> Result<(), String> {
        let data = read_package(Files {
            added: numbered("added-", 1),
            modified: numbered("modified-", 2),
            deleted: numbered("deleted-", 3),
            pristine: numbered("pristine-", 4),
            remote: numbered("remote-", 5),
            ignored: numbered("ignored-", 6),
        })
        .await?;

        assert_eq!(
            data.counts,
            EntryCounts {
                all: 15,
                changed: 6,
                not_downloaded: 5,
                ignored: 6,
            }
        );
        assert_eq!(data.total, 21);
        assert!(!data.truncated);
        Ok(())
    }

    /// v1 reads the same payload. Its own counts keep their meaning:
    /// unmodified is pristine plus not downloaded.
    #[tokio::test]
    async fn v1_counts_survive_the_cap() -> Result<(), String> {
        let data = read_package(Files {
            added: numbered("z/added-", 300),
            modified: numbered("y/modified-", 200),
            deleted: numbered("x/deleted-", 200),
            remote: numbered("a/remote-", 700),
            ignored: numbered("b/ignored-", 10),
            pristine: numbered("c/pristine-", 10),
        })
        .await?;

        assert_eq!(data.ignored_count, 10);
        assert_eq!(data.unmodified_count, 710);
        assert!(data.has_remote_entries);
        Ok(())
    }

    /// Anchored identically in the UI's `entry_counts_wire_form_is_verbatim`.
    #[test]
    fn entry_counts_wire_form_is_verbatim() {
        let counts = EntryCounts {
            all: 1410,
            changed: 700,
            not_downloaded: 700,
            ignored: 10,
        };
        assert_eq!(
            serde_json::to_string(&counts).unwrap(),
            r#"{"all":1410,"changed":700,"notDownloaded":700,"ignored":10}"#,
        );
    }

    #[tokio::test]
    async fn the_cap_crosses_the_wire_as_total_and_truncated() -> Result<(), String> {
        let data = read_remote_only(1001).await?;
        let wire = serde_json::to_value(&data).map_err(|e| e.to_string())?;
        assert_eq!(wire["total"], 1001);
        assert_eq!(wire["truncated"], true);
        assert_eq!(wire["counts"]["notDownloaded"], 1001);
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
