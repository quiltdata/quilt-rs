use std::path::PathBuf;

use crate::quilt::lineage::PackageLineageIo;
use crate::Error;

pub async fn uninstall_paths(
    lineage_io: &PackageLineageIo,
    working_dir: PathBuf,
    paths: &Vec<String>,
) -> Result<(), Error> {
    println!("uninstall_paths: {paths:?}");

    let mut lineage = lineage_io.read().await?;

    for path in paths {
        lineage.paths.remove(path).ok_or(Error::Uninstall(format!(
            "path {} not found. Cannot uninstall.",
            path
        )))?;

        let working_path = working_dir.join(path);
        match tokio::fs::remove_file(working_path).await {
            Ok(()) => (),
            Err(err) => {
                if err.kind() != std::io::ErrorKind::NotFound {
                    return Err(Error::Io(err));
                }
            }
        };
    }

    lineage_io.write(lineage).await?;

    // TODO: Remove unused files in OBJECTS_DIR?

    Ok(())
}
