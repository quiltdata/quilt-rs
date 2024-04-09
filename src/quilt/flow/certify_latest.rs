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

    use std::collections::HashMap;

    use crate::quilt::mocks;
    use crate::quilt::remote::mock_remote::MockRemote;

    #[tokio::test]
    async fn test_certifying_latest() -> Result<(), Error> {
        let mut remote = MockRemote {
            registry: HashMap::from([(
                "s3://b/.quilt/named_packages/a/latest".to_string(),
                b"OUTDATED_HASH".into(),
            )]),
        };
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
        assert_eq!(
            remote
                .registry
                .get("s3://b/.quilt/named_packages/a/latest")
                .unwrap(),
            b"LATEST_HASH",
        );
        Ok(())
    }
}
