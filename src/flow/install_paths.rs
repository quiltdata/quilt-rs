use std::collections::hash_map::RandomState;
use std::collections::BTreeMap;
use std::collections::HashSet;
use std::path::PathBuf;

use tokio_stream::StreamExt;
use url::Url;

use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::manifest::RowsStream;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::lineage::PathState;
use crate::manifest::Row;
use crate::manifest::Table;
use crate::paths::scaffold_paths;
use crate::paths::DomainPaths;
use crate::uri::Host;
use crate::uri::Namespace;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

async fn cache_immutable_object(
    storage: &impl Storage,
    remote: &impl Remote,
    host: Option<Host>,
    object_dest: &PathBuf,
    uri: &S3Uri,
) -> Res {
    let stream = remote.get_object_stream(host, uri).await?;
    storage.write_byte_stream(object_dest, stream.body).await
}

async fn create_mutable_copy(
    storage: &impl Storage,
    immutable_source: &PathBuf,
    mutable_target: &PathBuf,
) -> Res<chrono::DateTime<chrono::Utc>> {
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
) -> impl RowsStream {
    remote_manifest
        .records_stream()
        .await
        .map(move |rows_result| {
            rows_result.map(|rows| {
                rows.into_iter()
                    .map(|row_result| {
                        row_result.map(|row| match local_entries.get(&row.name) {
                            Some(row) => row.clone(),
                            None => row,
                        })
                    })
                    .collect()
            })
        })
}

/// Installs paths to already existing manifest (provided as an argument to this function).
/// It also modifies manifest, because installed paths have `place` pointing to `file://location`
// TODO: `working_dir` is in `paths` already, and we pass namespace anyway
//       so we can remove working_dir from the arguments
#[allow(clippy::too_many_arguments)]
pub async fn install_paths(
    mut lineage: PackageLineage,
    table: &mut Table,
    paths: &DomainPaths,
    working_dir: PathBuf, // This working dir is working dir of the package
    namespace: Namespace,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    entries_paths: &Vec<PathBuf>,
) -> Res<PackageLineage> {
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
            cache_immutable_object(
                storage,
                remote,
                lineage.remote.catalog.clone(),
                &object_dest,
                &row.place.parse()?,
            )
            .await?;
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
    use std::str::FromStr;
    use tempfile;

    use crate::manifest::Row;
    use crate::mocks;

    // Verify installing the path that is already fetched to the `.quilt/objects`
    // Practically it is useful when we try to install identical files. Then we can re-use cache (because files are located by hash).
    // In other cases, it tests implementation details.
    #[tokio::test]
    async fn test_installing_one_cached_path() -> Res {
        let domain_working_dir = tempfile::tempdir()?;
        let domain_paths = &DomainPaths::new(domain_working_dir.path().to_path_buf());

        let namespace = ("foo", "bar");
        let working_dir = domain_paths.working_dir(&namespace.into());

        // Simulate the file already exists in `.quilt/objects/HASH`
        // We trust that the hash is correct, so we can skip the actual file content
        let storage = mocks::storage::MockStorage::default();
        // The same hash is used in `mocks::manifest::with_record_keys`
        // So, it's not completely random.
        let hash = mocks::row_hash_sample1();
        let object_path = domain_paths.object(hash.digest());
        let absolute_path = domain_working_dir.path().join(object_path);
        // Path is `.quilt/objects/HASH`
        storage.write_file(absolute_path, &Vec::new()).await?;

        // Simulate the manifest with rows containing objects
        let lineage = PackageLineage::default();
        let single_object_path = PathBuf::from("a/a");
        let entries_paths = vec![single_object_path.clone()];
        let mut manifest = mocks::manifest::with_record_keys(entries_paths.clone());

        // Lineage does not track anything before the installation
        assert!(lineage.paths.is_empty());

        // We deal with cached file, so remote is "empty" and doesn't make any HTTP calls,
        // since it doesn't throw "key not found"
        let remote = mocks::remote::MockRemote::default();
        let lineage = install_paths(
            lineage,
            &mut manifest,
            domain_paths,
            working_dir.clone(),
            namespace.into(),
            &storage,
            &remote,
            &entries_paths,
        )
        .await?;

        // Now lineage tracks the file in the working directory
        assert!(lineage.paths.contains_key(&single_object_path));
        // And working directory of the package contains the file
        assert!(storage.exists(&working_dir.join(single_object_path)).await);

        Ok(())
    }

    /// Verify installing a path that is not cached locally in `.quilt/objects`.
    /// The path should be downloaded from the remote storage, cached locally, and then installed into the working directory.
    #[tokio::test]
    async fn test_installing_one_uncached_path() -> Res {
        let domain_working_dir = tempfile::tempdir()?;
        let domain_paths = &DomainPaths::new(domain_working_dir.path().to_path_buf());

        let namespace = ("foo", "bar");
        let working_dir = domain_paths.working_dir(&namespace.into());

        // Simulate the manifest with rows containing an object path
        let remote = mocks::remote::MockRemote::default();
        let storage = mocks::storage::MockStorage::default();
        let single_object_path = PathBuf::from("a/a");
        let entries_paths = vec![single_object_path.clone()];

        let remote_file_url = "s3://any/valid-url.md".to_string();

        // Before installation, lineage does not track any paths
        let lineage = PackageLineage::default();

        // Simulate the remote object
        let remote_object_uri = S3Uri::from_str(&remote_file_url)?;
        remote
            .put_object(
                lineage.remote.catalog.clone(),
                &remote_object_uri,
                Vec::new(),
            )
            .await?;

        // Create the manifest with a single remote row with a random hash
        let hash: multihash::Multihash<256> = multihash::Multihash::wrap(0x16, b"anything")?;
        let mut manifest = mocks::manifest::with_rows(vec![Row {
            name: single_object_path.clone(),
            hash,
            place: remote_file_url,
            ..Row::default()
        }]);

        assert!(lineage.paths.is_empty());

        // Perform the installation
        let lineage = install_paths(
            lineage,
            &mut manifest,
            domain_paths,
            working_dir.clone(),
            namespace.into(),
            &storage,
            &remote,
            &entries_paths,
        )
        .await?;

        // Verify the path is now tracked in lineage
        assert!(lineage.paths.contains_key(&single_object_path));
        // Verify the working directory contains the installed file
        assert!(storage.exists(&working_dir.join(single_object_path)).await);
        // Verify the object is cached locally in `.quilt/objects`
        // Note, that we don't verify the hash and trust the manifest
        let object_path = domain_paths.object(hash.digest());
        assert!(storage.exists(object_path).await);

        Ok(())
    }

    // Nothing special, just a combination of two previous tests,
    // so we're sure that single file is not a special case.
    #[tokio::test]
    async fn test_installing_multiple_paths() -> Res {
        let domain_working_dir = tempfile::tempdir()?;
        let domain_paths = &DomainPaths::new(domain_working_dir.path().to_path_buf());

        let namespace = ("foo", "bar");
        let working_dir = domain_paths.working_dir(&namespace.into());

        // Simulate the manifest with rows containing objects
        let lineage = PackageLineage::default();
        let row_1 = Row {
            name: PathBuf::from("a"),
            place: "file:///ignored".to_string(),
            hash: multihash::Multihash::wrap(0x16, b"one")?,
            ..Row::default()
        };
        let row_2 = Row {
            name: PathBuf::from("b/b"),
            place: "s3://bucket/foo/bar".to_string(),
            hash: multihash::Multihash::wrap(0x16, b"two")?,
            ..Row::default()
        };
        let row_3 = Row {
            name: PathBuf::from("c/c/c"),
            place: "file:///ignored".to_string(),
            hash: multihash::Multihash::wrap(0x16, b"three")?,
            ..Row::default()
        };
        let row_4 = Row {
            name: PathBuf::from("d/d/d/d"),
            place: "s3://bucket/foo/baz".to_string(),
            hash: multihash::Multihash::wrap(0x16, b"four")?,
            ..Row::default()
        };
        let rows = vec![row_1.clone(), row_2.clone(), row_3.clone(), row_4.clone()];
        let mut manifest = mocks::manifest::with_rows(rows);

        // Simulate two of three files (1 and 3) are already exist in `.quilt/objects/HASH`
        // We trust that the hash is correct, so we can skip the actual file content
        let storage = mocks::storage::MockStorage::default();
        let parent = domain_working_dir.path();
        let object_path_1 = parent.join(domain_paths.object(row_1.hash.digest()));
        storage.write_file(object_path_1, &Vec::new()).await?;
        let object_path_3 = parent.join(domain_paths.object(row_3.hash.digest()));
        storage.write_file(object_path_3, &Vec::new()).await?;

        // Simulate the remote object
        let remote = mocks::remote::MockRemote::default();
        let remote_object_uri_2 = S3Uri::from_str(&row_2.place)?;
        remote
            .put_object(
                lineage.remote.catalog.clone(),
                &remote_object_uri_2,
                Vec::new(),
            )
            .await?;
        let remote_object_uri_4 = S3Uri::from_str(&row_4.place)?;
        remote
            .put_object(
                lineage.remote.catalog.clone(),
                &remote_object_uri_4,
                Vec::new(),
            )
            .await?;

        let entries_paths = vec![
            row_1.name.clone(),
            row_2.name.clone(),
            row_3.name.clone(),
            row_4.name.clone(),
        ];

        // Lineage does not track anything before the installation
        assert!(lineage.paths.is_empty());

        let lineage = install_paths(
            lineage,
            &mut manifest,
            domain_paths,
            working_dir.clone(),
            namespace.into(),
            &storage,
            &remote,
            &entries_paths,
        )
        .await?;

        // Now lineage tracks the files in the working directory
        assert!(lineage.paths.contains_key(&row_1.name));
        assert!(lineage.paths.contains_key(&row_2.name));
        assert!(lineage.paths.contains_key(&row_3.name));
        assert!(lineage.paths.contains_key(&row_4.name));
        // And working directory of the package contains the files
        assert!(storage.exists(&working_dir.join(row_1.name)).await);
        assert!(storage.exists(&working_dir.join(row_2.name)).await);
        assert!(storage.exists(&working_dir.join(row_3.name)).await);
        assert!(storage.exists(&working_dir.join(row_4.name)).await);

        Ok(())
    }

    // Verify that the installation fails when we try to install a path that doesn't exist in the
    // manifest.
    #[tokio::test]
    async fn test_installing_path_that_doesnt_exists_in_manifest() -> Res {
        let lineage = PackageLineage::default();
        let remote = mocks::remote::MockRemote::default();
        let storage = mocks::storage::MockStorage::default();

        // We want to install z/z
        let entries_paths = vec![PathBuf::from("z/z")];
        // But manifest clearly doens't contain it. It contain different path
        let mut manifest = mocks::manifest::with_record_keys(vec![PathBuf::from("a/a")]);

        // Assert we don't track anything
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

    // TODO: fail if path is already installed
    // TODO: fail if manifest entry has invalid URL
}
