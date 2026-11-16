//! What a package's flows write to `data.json` itself.

use test_log::test;

use super::on_data_json::sync_flow_tests::committed_data_txt;
use crate::Res;

/// Builds that require `paths[].hash` cannot load an entry without it.
#[test(tokio::test)]
async fn a_committed_file_is_stored_with_its_hash() -> Res {
    let (package, _temp_dir) = committed_data_txt().await?;
    let record: serde_json::Value =
        serde_json::from_slice(&std::fs::read(package.paths.lineage())?)?;
    let entry = &record["packages"]["demo/tracked"]["paths"]["data.txt"];
    assert!(entry["hash"].is_string(), "{entry}");
    Ok(())
}
