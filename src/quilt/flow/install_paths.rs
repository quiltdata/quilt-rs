use std::collections::hash_map::RandomState;
use std::collections::HashSet;
use std::path::PathBuf;

use aws_sdk_s3::error::DisplayErrorContext;
use tokio::fs::File;
use tokio::io::AsyncWriteExt;
use url::Url;

use crate::paths;
use crate::quilt::lineage::PackageLineage;
use crate::quilt::lineage::PathState;
use crate::quilt::manifest_handle::ReadableManifest;
use crate::quilt::storage::s3;
use crate::quilt::Storage;
use crate::s3_utils;
use crate::Error;

async fn cache_immutable_object(object_dest: &PathBuf, uri: &s3::S3Uri) -> Result<(), Error> {
    let version = uri
        .version
        .clone()
        .ok_or(Error::S3Uri("missing versionId in s3 URL".to_string()))?;

    let mut file = File::create(&object_dest).await?;

    let client = s3_utils::get_client_for_bucket(&uri.bucket).await?;

    let mut object = client
        .get_object()
        .bucket(uri.bucket.clone())
        .key(uri.key.clone())
        .version_id(version)
        .send()
        .await
        .map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?;

    while let Some(bytes) = object
        .body
        .try_next()
        .await
        .map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?
    {
        file.write_all(&bytes).await?;
    }
    file.flush().await?;
    Ok(())
}

async fn create_mutable_copy(
    storage: &mut impl Storage,
    immutable_source: &PathBuf,
    mutable_target: &PathBuf,
) -> Result<chrono::DateTime<chrono::Utc>, Error> {
    let parent_dir = mutable_target.parent();
    if let Some(parent) = parent_dir {
        storage.create_dir_all(parent).await?;
    }
    storage.copy(&immutable_source, &mutable_target).await?;
    storage.modified_timestamp(&mutable_target).await
}

pub async fn install_paths(
    mut lineage: PackageLineage,
    manifest: &(impl ReadableManifest + Sync),
    paths: &paths::DomainPaths,
    working_dir: PathBuf,
    namespace: String,
    storage: &mut impl Storage,
    entries_paths: &Vec<String>,
) -> Result<PackageLineage, Error> {
    if entries_paths.is_empty() {
        return Ok(lineage);
    }

    // TODO: what happens if paths are already installed? Ignore, or error?
    // Fail early if path is already installed
    if !HashSet::<String, RandomState>::from_iter(lineage.paths.keys().cloned())
        .is_disjoint(&HashSet::from_iter(entries_paths.to_owned()))
    {
        return Err(Error::InstallPath(
            "some paths are already installed".to_string(),
        ));
    }

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

        let object_dest = paths.object(&row.hash);

        if !storage.exists(&object_dest).await {
            cache_immutable_object(&object_dest, &row.place.parse()?).await?;
        }

        row.place = Url::from_file_path(&object_dest)
            .map_err(|_| {
                Error::InstallPath(format!("Failed to create URL from {:?}", &object_dest))
            })?
            .to_string();

        let working_dest = working_dir.join(&row.name);
        let last_modified = create_mutable_copy(storage, &object_dest, &working_dest).await?;

        lineage.paths.insert(
            row.name.clone(),
            PathState {
                timestamp: last_modified,
                hash: row.hash,
            },
        );
    }

    // save the manifest
    // TODO: Write to a temporary file first.
    let installed_manifest_path = paths.installed_manifest(&namespace, lineage.current_hash());

    table.write_to_path(&installed_manifest_path).await?;

    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;
    use std::path::PathBuf;
    use temp_dir::TempDir;

    use crate::quilt::lineage::CommitState;
    use crate::quilt::storage::fs::LocalStorage;
    use crate::quilt::storage::mock_storage::MockStorage;
    use crate::Row4;
    use crate::Table;

    struct InMemoryManifest {}
    impl ReadableManifest for InMemoryManifest {
        async fn read(&self) -> Result<Table, Error> {
            Ok(Table {
                records: BTreeMap::from([(
                    "a/a".to_string(),
                    Row4 {
                        name: "a/a".to_string(),
                        place: "s3://data-yaml-spec-tests/scale/10u/e0-0.txt?versionId=jHb6DGN43Ex7EhbxZc2G9JnAkWSeTfEY".to_string(),
                        hash: multihash::Multihash::wrap(345, b"Hello world")?,
                        ..Row4::default()
                    },
                )]),
                ..Table::default()
            })
        }
    }

    #[tokio::test]
    async fn test_installing_one_path() -> Result<(), Error> {
        let working_dir = TempDir::new()?;

        let namespace = "foo/bar".to_string();

        let domain_paths = &paths::DomainPaths::new(working_dir.path().to_path_buf());
        // TODO: Can't use MockStorage because of Table::write_to_upath
        let mut storage = LocalStorage::new();
        storage
            .create_dir_all(domain_paths.installed_manifests(&namespace))
            .await?;
        storage.create_dir_all(domain_paths.objects_dir()).await?;

        let lineage = PackageLineage {
            commit: Some(CommitState {
                hash: "fghijk".to_string(),
                ..CommitState::default()
            }),
            ..PackageLineage::default()
        };
        let entries_paths = vec!["a/a".to_string()];
        let manifest = InMemoryManifest {};

        assert!(lineage.paths.is_empty());
        let lineage = install_paths(
            lineage,
            &manifest,
            domain_paths,
            working_dir.path().to_path_buf(),
            namespace,
            &mut storage,
            &entries_paths,
        )
        .await?;
        assert!(lineage.paths.contains_key("a/a"));

        Ok(())
    }

    #[tokio::test]
    async fn test_installing_path_that_doesnt_exists_in_manifest() -> Result<(), Error> {
        let lineage = PackageLineage {
            commit: Some(CommitState {
                hash: "fghijk".to_string(),
                ..CommitState::default()
            }),
            ..PackageLineage::default()
        };
        let mut storage = MockStorage::default();
        let entries_paths = vec!["z/z".to_string()];
        let manifest = InMemoryManifest {};

        assert!(lineage.paths.is_empty());
        let lineage = install_paths(
            lineage,
            &manifest,
            &paths::DomainPaths::default(),
            PathBuf::new(),
            String::default(),
            &mut storage,
            &entries_paths,
        )
        .await;
        assert_eq!(
            lineage.unwrap_err().to_string(),
            "Table error: path z/z not found".to_string()
        );
        Ok(())
    }
}
