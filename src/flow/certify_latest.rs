use crate::io::manifest::tag_latest;
use crate::io::remote::Remote;
use crate::lineage::PackageLineage;
use crate::uri::ManifestUri;
use crate::Error;

pub async fn certify_latest(
    mut lineage: PackageLineage,
    remote: &impl Remote,
    manifest_uri: ManifestUri,
) -> Result<PackageLineage, Error> {
    tag_latest(remote, &manifest_uri).await?;
    lineage.update_latest(manifest_uri.clone());
    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::mocks;
    use crate::uri::S3Uri;

    #[tokio::test]
    async fn test_certifying_latest() -> Result<(), Error> {
        let remote = mocks::remote::MockRemote::default();
        remote
            .put_object(
                &S3Uri::try_from("s3://b/.quilt/named_packages/f/a/latest")?,
                b"OUTDATED_HASH".to_vec(),
            )
            .await?;
        let source_lineage = mocks::lineage::with_remote("quilt+s3://b#package=f/a@LATEST_HASH")?;
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
        // FIXME: read_to_end
        // assert_eq!(
        //     remote
        //         .get_object(&S3Uri::try_from("s3://b/.quilt/named_packages/a/latest")?)
        //         .await?,
        //     b"LATEST_HASH",
        // );
        Ok(())
    }
}
