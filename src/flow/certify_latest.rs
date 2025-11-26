use crate::io::manifest::tag_latest;
use crate::io::remote::Remote;
use crate::lineage::PackageLineage;
use crate::uri::ManifestUri;
use crate::Res;
use tracing::info;

/// Tags the `manifest_uri` as "latest" remotely.
/// And update localy in .quilt/lineage.json `base_hash` and `latest_hash` to that hash as well.
pub async fn certify_latest(
    mut lineage: PackageLineage,
    remote: &impl Remote,
    manifest_uri: ManifestUri,
) -> Res<PackageLineage> {
    info!(
        "⏳ Certifying manifest {} as latest",
        manifest_uri.display()
    );
    tag_latest(remote, &manifest_uri).await?;
    lineage.update_latest(manifest_uri.clone());
    info!(
        "✔️ Successfully certified manifest {} as latest",
        manifest_uri.display()
    );
    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::io::remote::mocks::MockRemote;
    use crate::uri::S3Uri;

    #[tokio::test]
    async fn test_certifying_latest() -> Res {
        let remote = MockRemote::default();
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/named_packages/f/a/latest")?,
                b"OUTDATED_HASH".to_vec(),
            )
            .await?;

        let source_manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "LATEST_HASH".to_string(),
            catalog: None,
        };
        let source_lineage = PackageLineage {
            remote: source_manifest_uri,
            ..PackageLineage::default()
        };
        let resolved_lineage = certify_latest(
            source_lineage.clone(),
            &remote,
            source_lineage.remote.clone(),
        )
        .await?;
        assert_eq!(
            resolved_lineage,
            PackageLineage {
                base_hash: "LATEST_HASH".to_string(),
                latest_hash: "LATEST_HASH".to_string(),
                ..source_lineage
            }
        );

        let latest_uri = S3Uri::try_from("s3://b/.quilt/named_packages/f/a/latest")?;
        let latest_file = remote.get_object_stream(&None, &latest_uri).await?;
        assert_eq!(latest_file.body.collect().await?.to_vec(), b"LATEST_HASH",);
        Ok(())
    }
}
