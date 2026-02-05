use std::path::PathBuf;

use crate::flow;
use crate::io::manifest::resolve_tag;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::paths::copy_cached_to_installed;
use crate::paths::DomainPaths;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
use crate::uri::Tag;
use crate::Res;
use tracing::debug;
use tracing::info;

pub async fn reset_to_latest(
    lineage: PackageLineage,
    manifest: &mut Manifest,
    paths: &DomainPaths,
    storage: &(impl Storage + std::marker::Sync),
    remote: &impl Remote,
    package_home: PathBuf,
    namespace: Namespace,
) -> Res<PackageLineage> {
    info!("⏳ Starting reset to latest for package {}", namespace);

    debug!(
        "⏳ Resolving latest manifest hash for {}",
        lineage.remote.display()
    );
    let latest = resolve_tag(
        remote,
        &lineage.remote.origin,
        &lineage.remote.clone().into(),
        Tag::Latest,
    )
    .await?;
    debug!("✔️ Latest hash resolved: {}", latest.hash);

    if latest.hash == lineage.remote.hash {
        info!("✔️ Package is already at latest version");
        return Ok(lineage);
    }

    let installed_paths: Vec<PathBuf> = lineage.paths.clone().into_keys().collect();
    debug!("⏳ Uninstalling {} paths", installed_paths.len());
    let mut lineage =
        flow::uninstall_paths(lineage, package_home.clone(), storage, &installed_paths).await?;

    debug!("⏳ Updating lineage hashes");
    // TODO: Should be a method of lineage
    lineage.latest_hash.clone_from(&latest.hash);
    lineage.base_hash.clone_from(&latest.hash);
    debug!("✔️ Updated lineage to latest hash: {}", latest.hash);

    debug!("⏳ Caching remote manifest");
    flow::cache_remote_manifest(paths, storage, remote, &latest).await?;

    // TODO: merge the following steps with `pull.rs`

    debug!("⏳ Installing cached manifest");
    copy_cached_to_installed(
        paths,
        storage,
        &ManifestUri {
            namespace: namespace.clone(),
            ..latest.clone()
        },
    )
    .await?;
    lineage.remote = latest;
    debug!("✔️ Manifest installed successfully");

    debug!("⏳ Checking which paths to reinstall");
    let mut paths_to_install = Vec::new();
    for x in &installed_paths {
        if manifest.contains_record(x) {
            debug!("✔️ Will reinstall path: {}", x.display());
            paths_to_install.push(x)
        } else {
            debug!("ℹ️ Path no longer exists in manifest: {}", x.display());
        }
    }

    info!("⏳ Reinstalling {} paths", paths_to_install.len());

    // Convert Manifest to Table for install_paths function
    let result = flow::install_paths(
        lineage,
        manifest,
        paths,
        package_home,
        namespace,
        storage,
        remote,
        &paths_to_install,
    )
    .await?;

    info!("✔️ Successfully reset package to latest version");
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::fixtures;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::PackageLineage;
    use crate::uri::S3Uri;

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
            remote: source_manifest_uri,
            ..PackageLineage::default()
        };

        let remote = MockRemote::default();
        remote
            .put_object(
                &None,
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
            remote: manifest_uri,
            ..PackageLineage::default()
        };

        let jsonl = std::fs::read(fixtures::manifest::jsonl()?)?;
        let test_hash: &str = "deadbeef";
        let remote = MockRemote::default();
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/named_packages/f/a/latest")?,
                test_hash.as_bytes().to_vec(),
            )
            .await?;
        remote
            .put_object(
                &None,
                &S3Uri::try_from(format!("s3://b/.quilt/packages/{}", &test_hash).as_str())?,
                jsonl,
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
                remote: ManifestUri {
                    hash: test_hash.to_string(),
                    ..source_lineage.remote.clone()
                },
                ..source_lineage
            }
        );
        Ok(())
    }
}
