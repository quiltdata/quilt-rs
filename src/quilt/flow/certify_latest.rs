use crate::quilt::lineage::PackageLineage;
use crate::Error;

pub async fn certify_latest(mut lineage: PackageLineage) -> Result<PackageLineage, Error> {
    let new_latest = lineage.remote.hash.clone();
    lineage.remote.update_latest(&new_latest).await?;
    lineage.latest_hash = new_latest.clone();
    lineage.base_hash = new_latest;
    Ok(lineage)
}
