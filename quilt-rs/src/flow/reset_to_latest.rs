use std::path::PathBuf;

use crate::Res;
use crate::flow::apply_latest_update;
use crate::io::manifest::resolve_tag;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::paths::DomainPaths;
use quilt_uri::Namespace;
use quilt_uri::Tag;
use tracing::debug;
use tracing::info;

pub async fn reset_to_latest(
    mut lineage: PackageLineage,
    manifest: &mut Manifest,
    paths: &DomainPaths,
    storage: &(impl Storage + std::marker::Sync),
    remote: &impl Remote,
    package_home: PathBuf,
    namespace: Namespace,
) -> Res<PackageLineage> {
    info!("⏳ Starting reset to latest for package {}", namespace);

    let remote_uri = lineage.remote()?.clone();

    debug!(
        "⏳ Resolving latest manifest hash for {}",
        remote_uri.display()
    );
    let origin = remote_uri.origin.clone();
    let latest = resolve_tag(remote, origin.as_ref(), remote_uri, Tag::Latest).await?;
    debug!("✔️ Latest hash resolved: {}", latest.hash);

    if latest.hash == lineage.remote()?.hash {
        info!("✔️ Package is already at latest version");
        return Ok(lineage);
    }

    // Discard any pending local commit. The prior behavior preserved
    // `commit` across reset to support an offline-commit / independent-push
    // workflow: a user could reset their view of remote `latest` without
    // throwing away work they intended to push later. With autosync the
    // merge page is reachable asynchronously and `certify_latest` now
    // pushes when `commit.is_some()`, so a stale commit after reset would
    // let a subsequent certify resurrect the discarded revision (its
    // installed manifest is still on disk). The trade-off shifted; reset
    // now matches the UX promise of erasing local commits. This must run on
    // the pre-uninstall lineage, before it is moved into the primitive.
    lineage.commit = None;

    // Reset's touch-set is every installed path: overwrite local with remote.
    let touched: Vec<PathBuf> = lineage.paths.keys().cloned().collect();

    let result = apply_latest_update(
        lineage,
        manifest,
        paths,
        storage,
        remote,
        package_home,
        namespace,
        latest,
        &touched,
    )
    .await?;

    info!("✔️ Successfully reset package to latest version");
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::PackageLineage;
    use quilt_uri::ManifestUri;
    use quilt_uri::S3Uri;

    use test_log::test;

    #[test(tokio::test)]
    async fn test_if_already_latest() -> Res {
        let source_manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "foo".to_string(),
            origin: None,
        };
        let source_lineage = PackageLineage {
            remote_uri: Some(source_manifest_uri),
            ..PackageLineage::default()
        };

        let remote = MockRemote::default();
        remote
            .put_object(
                None,
                &S3Uri::try_from("s3://b/.quilt/named_packages/f/a/latest")?,
                b"foo".to_vec(),
            )
            .await?;

        let resolved_lineage = reset_to_latest(
            source_lineage.clone(),
            &mut Manifest::default(),
            &DomainPaths::default(),
            &MockStorage::default(),
            &remote,
            PathBuf::default(),
            Namespace::default(),
        )
        .await?;
        assert_eq!(resolved_lineage, source_lineage);
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_reseting_to_latest() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "OUTDATED_HASH".to_string(),
            origin: None,
        };

        let paths = DomainPaths::default();
        let storage = MockStorage::default();
        paths
            .scaffold_for_caching(&storage, &manifest_uri.bucket)
            .await?;

        let source_lineage = PackageLineage {
            remote_uri: Some(manifest_uri),
            ..PackageLineage::default()
        };

        let test_hash: &str = "deadbeef";
        let dummy_manifest = r#"{"version": "v0"}"#;
        let remote = MockRemote::default();
        remote
            .put_object(
                None,
                &S3Uri::try_from("s3://b/.quilt/named_packages/f/a/latest")?,
                test_hash.as_bytes().to_vec(),
            )
            .await?;
        remote
            .put_object(
                None,
                &S3Uri::try_from(format!("s3://b/.quilt/packages/{test_hash}").as_str())?,
                dummy_manifest.as_bytes().to_vec(),
            )
            .await?;

        let resolved_lineage = reset_to_latest(
            source_lineage.clone(),
            &mut Manifest::default(),
            &paths,
            &storage,
            &remote,
            PathBuf::default(),
            Namespace::default(),
        )
        .await?;
        assert_eq!(
            resolved_lineage,
            PackageLineage {
                base_hash: test_hash.to_string(),
                latest_hash: test_hash.to_string(),
                remote_uri: Some(ManifestUri {
                    hash: test_hash.to_string(),
                    ..source_lineage.remote().unwrap().clone()
                }),
                ..source_lineage
            }
        );
        Ok(())
    }

    /// Regression: `reset_to_latest` must clear `lineage.commit`. Otherwise
    /// the lineage stays self-inconsistent (`UpToDate` on hashes, Ahead via
    /// `current_hash()`), and a later Diverged → merge → "Promote my
    /// revision" would push and tag the very revision the user just
    /// discarded.
    #[test(tokio::test)]
    async fn test_reset_clears_pending_commit() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "OUTDATED_HASH".to_string(),
            origin: None,
        };

        let paths = DomainPaths::default();
        let storage = MockStorage::default();
        paths
            .scaffold_for_caching(&storage, &manifest_uri.bucket)
            .await?;

        let source_lineage = PackageLineage {
            commit: Some(crate::lineage::CommitState {
                timestamp: chrono::Utc::now(),
                hash: "PENDING_LOCAL_COMMIT".to_string(),
                prev_hashes: Vec::new(),
            }),
            remote_uri: Some(manifest_uri),
            ..PackageLineage::default()
        };

        let test_hash: &str = "deadbeef";
        let dummy_manifest = r#"{"version": "v0"}"#;
        let remote = MockRemote::default();
        remote
            .put_object(
                None,
                &S3Uri::try_from("s3://b/.quilt/named_packages/f/a/latest")?,
                test_hash.as_bytes().to_vec(),
            )
            .await?;
        remote
            .put_object(
                None,
                &S3Uri::try_from(format!("s3://b/.quilt/packages/{test_hash}").as_str())?,
                dummy_manifest.as_bytes().to_vec(),
            )
            .await?;

        let resolved_lineage = reset_to_latest(
            source_lineage,
            &mut Manifest::default(),
            &paths,
            &storage,
            &remote,
            PathBuf::default(),
            Namespace::default(),
        )
        .await?;
        assert!(
            resolved_lineage.commit.is_none(),
            "reset must discard the pending local commit, got {:?}",
            resolved_lineage.commit,
        );
        Ok(())
    }
}
