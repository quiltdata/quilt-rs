use std::collections::hash_map::RandomState;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::PathBuf;

use tokio_stream::Stream;
use tokio_stream::StreamExt;
use url::Url;

use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::lineage::PathState;
use crate::manifest::Row;
use crate::manifest::Table;
use crate::paths::scaffold_paths;
use crate::paths::DomainPaths;
use crate::uri::Namespace;
use crate::uri::S3Uri;
use crate::Error;

async fn cache_immutable_object(
    storage: &impl Storage,
    remote: &impl Remote,
    object_dest: &PathBuf,
    uri: &S3Uri,
) -> Result<(), Error> {
    let body = remote.get_object_stream(uri).await?;
    storage.write_byte_stream(object_dest, body).await
}

async fn create_mutable_copy(
    storage: &impl Storage,
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

async fn stream_remote_with_installed_rows(
    remote_manifest: &Table,
    local_entries: BTreeMap<PathBuf, Row>,
) -> impl Stream<Item = Result<Row, Error>> {
    remote_manifest
        .records_stream()
        .await
        .map(move |row| match local_entries.get(&row.name) {
            Some(row) => Ok(row.clone()),
            None => Ok(row),
        })
}

/// Installs paths to already existing manifest (provided as an argument to this function).
/// It also modifies manifest, because installed paths have `place` pointing to `file://location`
// TODO: move `working_dir` to `paths`, and `paths` to `storage`
#[allow(clippy::too_many_arguments)]
pub async fn install_paths(
    mut lineage: PackageLineage,
    table: &mut Table,
    paths: &DomainPaths,
    working_dir: PathBuf,
    namespace: Namespace,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    entries_paths: &Vec<PathBuf>,
) -> Result<PackageLineage, Error> {
    if entries_paths.is_empty() {
        return Ok(lineage);
    }

    scaffold_paths(storage, paths.required_installed_package_paths(&namespace)).await?;

    // TODO: what happens if paths are already installed? Ignore, or error?
    // Fail early if path is already installed
    if !HashSet::<PathBuf, RandomState>::from_iter(lineage.paths.keys().cloned())
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
    let mut entries = BTreeMap::new();

    for path in entries_paths {
        // TODO: Consider using a hashmap or treemap for manifest.rows
        let row = table
            .get_record(path)
            .await?
            .ok_or(Error::Table(format!("path {:?} not found", path)))?;

        let object_dest = paths.object(row.hash.digest());

        if !storage.exists(&object_dest).await {
            cache_immutable_object(storage, remote, &object_dest, &row.place.parse()?).await?;
        }

        let place = Url::from_file_path(&object_dest)
            .map_err(|_| {
                Error::InstallPath(format!("Failed to create URL from {:?}", &object_dest))
            })?
            .to_string();
        entries.insert(
            row.name.clone(),
            Row {
                place,
                ..row.clone()
            },
        );

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

    let header = table.get_header().await?;
    let stream = stream_remote_with_installed_rows(table, entries).await;
    let manifest_path = |t: &str| paths.installed_manifest(&namespace, t);
    build_manifest_from_rows_stream(storage, manifest_path, header, stream).await?;

    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use tempfile;

    use crate::manifest::Row;
    use crate::mocks;

    #[tokio::test]
    async fn test_installing_one_cached_path() -> Result<(), Error> {
        let working_dir = tempfile::tempdir()?;

        let domain_paths = &DomainPaths::new(working_dir.path().to_path_buf());

        let remote = mocks::remote::MockRemote::default();
        let storage = mocks::storage::MockStorage::default();
        let object_path = domain_paths.object(mocks::row_hash_sample1().digest());
        storage
            .write_file(working_dir.path().join(object_path), &Vec::new())
            .await?;

        let lineage = mocks::lineage::with_commit_hash("fghijk");
        let entries_paths = vec![PathBuf::from("a/a")];
        let mut manifest = mocks::manifest::with_record_keys(entries_paths.clone());

        assert!(lineage.paths.is_empty());
        let lineage = install_paths(
            lineage,
            &mut manifest,
            domain_paths,
            working_dir.path().to_path_buf(),
            ("foo", "bar").into(),
            &storage,
            &remote,
            &entries_paths,
        )
        .await?;
        assert!(lineage.paths.contains_key(&PathBuf::from("a/a")));
        assert!(
            storage
                .exists(&working_dir.path().join(PathBuf::from("a/a")))
                .await
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_installing_one_uncached_path() -> Result<(), Error> {
        let working_dir = tempfile::tempdir()?;

        let domain_paths = &DomainPaths::new(working_dir.path().to_path_buf());

        let remote = mocks::remote::MockRemote::default();
        let storage = mocks::storage::MockStorage::default();
        let lineage = mocks::lineage::with_commit_hash("fghijk");
        let entries_paths = vec![PathBuf::from("a/a")];
        let mut manifest = mocks::manifest::with_rows(vec![Row {
            name: PathBuf::from("a/a"),
            hash: mocks::row_hash_sample1(),
            place: "s3://any/any/any/any/any.md".to_string(),
            ..Row::default()
        }]);
        remote
            .put_object(&S3Uri::try_from("s3://any/any/any/any/any.md")?, Vec::new())
            .await?;

        assert!(lineage.paths.is_empty());
        let lineage = install_paths(
            lineage,
            &mut manifest,
            domain_paths,
            working_dir.path().to_path_buf(),
            ("foo", "bar").into(),
            &storage,
            &remote,
            &entries_paths,
        )
        .await?;
        assert!(lineage.paths.contains_key(&PathBuf::from("a/a")));
        assert!(
            storage
                .exists(&working_dir.path().join(PathBuf::from("a/a")))
                .await
        );
        let object_path = domain_paths.object(mocks::row_hash_sample1().digest());
        assert!(storage.exists(object_path).await);

        Ok(())
    }

    #[tokio::test]
    async fn test_installing_path_that_doesnt_exists_in_manifest() -> Result<(), Error> {
        let lineage = mocks::lineage::with_commit_hash("fghijk");
        let remote = mocks::remote::MockRemote::default();
        let storage = mocks::storage::MockStorage::default();
        let entries_paths = vec![PathBuf::from("z/z")];
        let mut manifest = mocks::manifest::with_record_keys(vec![PathBuf::from("a/a")]);

        assert!(lineage.paths.is_empty());
        let lineage = install_paths(
            lineage,
            &mut manifest,
            &DomainPaths::default(),
            PathBuf::new(),
            Namespace::default(),
            &storage,
            &remote,
            &entries_paths,
        )
        .await;
        assert_eq!(
            lineage.unwrap_err().to_string(),
            r#"Table error: path "z/z" not found"#
        );
        Ok(())
    }
}
