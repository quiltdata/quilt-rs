use crate::quilt::lineage::PackageLineage;
use crate::quilt::remote::Remote;
use crate::Error;

pub async fn certify_latest(
    mut lineage: PackageLineage,
    remote: &mut impl Remote,
) -> Result<PackageLineage, Error> {
    let new_latest = lineage.remote.hash.clone();
    lineage.remote.update_latest(remote, &new_latest).await?;
    lineage.latest_hash = new_latest.clone();
    lineage.base_hash = new_latest;
    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::quilt::storage::s3::S3Uri;

    use crate::quilt::mocks;
    use crate::quilt::remote::mock_remote::MockRemote;

    #[tokio::test]
    async fn test_certifying_latest() -> Result<(), Error> {
        let mut remote = MockRemote::default();
        remote
            .put_object(
                &S3Uri::try_from("s3://b/.quilt/named_packages/a/latest")?,
                b"OUTDATED_HASH".to_vec(),
            )
            .await?;
        let source_lineage = mocks::lineage::with_remote("quilt+s3://b#package=a@LATEST_HASH")?;
        let resolved_lineage = certify_latest(source_lineage.clone(), &mut remote).await?;
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
