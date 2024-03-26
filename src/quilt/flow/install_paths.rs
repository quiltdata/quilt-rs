use std::{
    collections::{hash_map::RandomState, HashSet},
    path::PathBuf,
};

use tokio::{fs::File, io::AsyncWriteExt};
use url::Url;

use crate::quilt::{
    lineage::{PackageLineageIo, PathState},
    manifest_handle::ReadableManifest,
    storage::{fs, s3},
};
use crate::{paths, s3_utils, Error, UPath};

pub async fn install_paths(
    lineage_io: &PackageLineageIo,
    manifest: &(impl ReadableManifest + Sync),
    paths: &paths::DomainPaths,
    working_dir: PathBuf,
    namespace: String,
    entries_paths: &Vec<String>,
) -> Result<(), Error> {
    if entries_paths.is_empty() {
        return Ok(());
    }

    let mut lineage = lineage_io.read().await?;

    // TODO: what happens if paths are already installed? Ignore, or error?
    if !HashSet::<String, RandomState>::from_iter(lineage.paths.keys().cloned())
        .is_disjoint(&HashSet::from_iter(entries_paths.to_owned()))
    {
        return Err(Error::InstallPath(
            "some paths are already installed".to_string(),
        ));
    }

    let objects_dir = paths.objects_dir();

    // for each path in entries_paths:
    //   get entry from installed manifest
    //   cache the entry into identity cache (if not there)
    //   replace entry's physical key in the manifest with the cached physical key
    //
    // write the adjusted manifest into the installed manifest path
    // copy the selected paths into the working folder
    //
    // record installation into the lineage:
    //   add installed package entry:
    //     remote: RemoteManifest

    let mut table = manifest.read().await?;

    for path in entries_paths {
        // TODO: Consider using a hashmap or treemap for manifest.rows
        let row = table
            .records
            .get_mut(path)
            .ok_or(Error::Table(format!("path {} not found", path)))?;

        let s3::S3Uri {
            bucket,
            key,
            version,
        } = row.place.parse()?;
        let version = version.ok_or(Error::S3Uri("missing versionId in s3 URL".to_string()))?;

        let object_dest = objects_dir.join(hex::encode(row.hash.digest()));

        if !fs::exists(&object_dest).await {
            let mut file = File::create(&object_dest).await?;

            let client = s3_utils::get_client_for_bucket(&bucket).await?;

            let mut object = client
                .get_object()
                .bucket(bucket)
                .key(key)
                .version_id(version)
                .send()
                .await
                .map_err(|err| Error::S3(format!("failed to get S3 object: {}", err)))?;

            while let Some(bytes) = object
                .body
                .try_next()
                .await
                .map_err(|err| Error::S3(format!("failed to read S3 object: {}", err)))?
            {
                file.write_all(&bytes).await?;
            }
            file.flush().await?;
        }

        row.place = Url::from_file_path(&object_dest)
            .map_err(|_| {
                Error::InstallPath(format!("Failed to create URL from {:?}", &object_dest))
            })?
            .to_string();

        let working_dest = working_dir.join(&row.name);
        let parent_dir = working_dest.parent();
        if parent_dir.is_some() {
            tokio::fs::create_dir_all(parent_dir.unwrap()).await?;
        }
        tokio::fs::copy(&object_dest, &working_dest).await?;
        let timestamp = fs::get_file_modified_ts(&working_dest).await?;
        lineage.paths.insert(
            row.name.to_owned(),
            PathState {
                timestamp,
                hash: row.hash.to_owned(),
            },
        );
    }

    // save the manifest
    // TODO: Write to a temporary file first.
    let installed_manifest_path = paths.installed_manifest(&namespace, lineage.current_hash());

    table
        .write_to_upath(&UPath::Local(installed_manifest_path))
        .await?;

    lineage_io.write(lineage).await?;

    Ok(())
}
