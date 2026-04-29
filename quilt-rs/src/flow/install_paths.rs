use std::collections::BTreeMap;
use std::collections::HashSet;
use std::collections::hash_map::RandomState;
use std::path::PathBuf;

use tokio_stream::StreamExt;
use tracing::debug;
use tracing::info;
use url::Url;

use crate::Error;
use crate::InstallPathError;
use crate::Res;
use crate::error::ManifestError;
use crate::io::manifest::RowsStream;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::lineage::PathState;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::paths::DomainPaths;
use quilt_uri::Host;
use quilt_uri::Namespace;
use quilt_uri::S3Uri;

async fn cache_immutable_object(
    storage: &impl Storage,
    remote: &impl Remote,
    host: &Option<Host>,
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
    remote_manifest: &Manifest,
    local_entries: BTreeMap<PathBuf, ManifestRow>,
) -> impl RowsStream {
    remote_manifest
        .records_stream()
        .await
        .map(move |rows_result| {
            rows_result.map(|rows| {
                rows.into_iter()
                    .map(|row_result| {
                        row_result.map(|row| match local_entries.get(&row.logical_key) {
                            Some(local_row) => local_row.clone(),
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
    manifest: &mut Manifest,
    paths: &DomainPaths,
    working_dir: PathBuf, // This working dir is working dir of the package
    namespace: Namespace,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    entries_paths: &[&PathBuf],
) -> Res<PackageLineage> {
    if entries_paths.is_empty() {
        info!("No paths to install");
        return Ok(lineage);
    }

    let remote_uri = lineage.remote()?.clone();

    info!(
        "⏳ Installing {} paths for package {}",
        entries_paths.len(),
        namespace
    );

    debug!("🔍 Checking for already installed paths");
    // TODO: what happens if paths are already installed? Ignore, or error?
    // Fail early if path is already installed
    if !HashSet::<&PathBuf, RandomState>::from_iter(lineage.paths.keys())
        .is_disjoint(&HashSet::from_iter(entries_paths.to_owned()))
    {
        debug!("❌ Found paths that are already installed");
        return Err(Error::InstallPath(InstallPathError::AlreadyInstalled));
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
        let row = manifest
            .get_record(path)
            .ok_or(ManifestError::Table(format!("path {path:?} not found")))?;

        let object_dest = paths.object(row.hash.digest());

        if !storage.exists(&object_dest).await {
            cache_immutable_object(
                storage,
                remote,
                &remote_uri.origin,
                &object_dest,
                &row.physical_key.parse()?,
            )
            .await?;
            debug!("✔️ Cached object: {}", object_dest.display());
        } else {
            debug!("✔️ Object already in cache: {}", object_dest.display());
        }

        let place = Url::from_file_path(&object_dest)
            .map_err(|_| Error::InstallPath(InstallPathError::Install(object_dest.clone())))?
            .to_string();
        debug!(
            "✔️ Path {} converted to a `place` {}",
            object_dest.display(),
            place
        );
        entries.insert(row.logical_key.clone(), row.clone());

        let working_dest = working_dir.join(&row.logical_key);
        let last_modified = create_mutable_copy(storage, &object_dest, &working_dest).await?;
        debug!(
            "✔️ Created mutable copy at {} for {}",
            last_modified,
            working_dest.display()
        );

        lineage.paths.insert(
            row.logical_key.clone(),
            PathState {
                timestamp: last_modified,
                hash: row.hash.clone().into(),
            },
        );
        debug!("✔️ Added {}  to lineage paths ", row.logical_key.display());
    }

    debug!("⏳ Building manifest with installed rows");
    let stream = stream_remote_with_installed_rows(manifest, entries).await;
    let dest_dir = paths.installed_manifests_dir(&namespace);
    build_manifest_from_rows_stream(storage, dest_dir, manifest.header.clone(), stream).await?;

    info!("✔️ Successfully installed {} paths", entries_paths.len());
    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use aws_sdk_s3::primitives::ByteStream;
    use std::path::PathBuf;
    use std::str::FromStr;

    use crate::fixtures;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::Home;
    use crate::paths;
    use quilt_uri::ManifestUri;

    // Verify installing the path that is already fetched to the `.quilt/objects`
    // Practically it is useful when we try to install identical files. Then we can re-use cache (because files are located by hash).
    // In other cases, it tests implementation details.
    #[test(tokio::test)]
    async fn test_installing_one_cached_path() -> Res {
        let (home, _temp_dir1) = Home::from_temp_dir()?;
        let (domain_paths, _temp_dir2) = &DomainPaths::from_temp_dir()?;

        let namespace = Namespace::from(("foo", "bar"));
        let package_home = paths::package_home(&home, &namespace);

        // Simulate the file already exists in `.quilt/objects/HASH`
        // We trust that the hash is correct, so we can skip the actual file content
        let storage = MockStorage::default();

        // Simulate the manifest with rows containing objects
        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri::default()),
            ..PackageLineage::default()
        };
        let single_object_path = PathBuf::from("less-then-8mb.txt");
        let entries_paths = vec![&single_object_path];
        let mut manifest = fixtures::manifest_with_objects_all_sizes::manifest().await?;

        let hash: multihash::Multihash<256> = manifest
            .get_record(&single_object_path)
            .unwrap()
            .hash
            .clone()
            .into();
        // let hash = fixtures::create_multihash(fixtures::objects::LESS_THAN_8MB_HASH_B64)?;
        let object_path = domain_paths.object(hash.digest());
        let absolute_path = home.join(object_path);
        // Path is `.quilt/objects/HASH`
        storage
            .write_byte_stream(absolute_path, ByteStream::default())
            .await?;

        // Lineage does not track anything before the installation
        assert!(lineage.paths.is_empty());

        // We deal with cached file, so remote is "empty" and doesn't make any HTTP calls,
        // since it doesn't throw "key not found"
        let remote = MockRemote::default();
        let lineage = install_paths(
            lineage,
            &mut manifest,
            domain_paths,
            package_home.clone(),
            namespace,
            &storage,
            &remote,
            &entries_paths,
        )
        .await?;

        // Now lineage tracks the file in the working directory
        assert!(lineage.paths.contains_key(&single_object_path));
        // And working directory of the package contains the file
        assert!(storage.exists(&package_home.join(single_object_path)).await);

        Ok(())
    }

    /// Verify installing a path that is not cached locally in `.quilt/objects`.
    /// The path should be downloaded from the remote storage, cached locally, and then installed into the working directory.
    #[test(tokio::test)]
    async fn test_installing_one_uncached_path() -> Res {
        let (home, _temp_dir1) = Home::from_temp_dir()?;
        let (domain_paths, _temp_dir2) = &DomainPaths::from_temp_dir()?;

        let namespace = Namespace::from(("foo", "bar"));
        let package_home = paths::package_home(&home, &namespace);

        // Simulate the manifest with rows containing an object path
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        let single_object_path = PathBuf::from("a/a");
        let entries_paths = vec![&single_object_path];

        domain_paths
            .scaffold_for_installing(&storage, &home, &namespace)
            .await?;

        let remote_file_url = "s3://any/valid-url.md".to_string();

        // Before installation, lineage does not track any paths
        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri::default()),
            ..PackageLineage::default()
        };

        // Simulate the remote object
        let remote_object_uri = S3Uri::from_str(&remote_file_url)?;
        remote
            .put_object(&None, &remote_object_uri, Vec::new())
            .await?;

        // Create the manifest with a single remote row with a random hash
        let hash: multihash::Multihash<256> = multihash::Multihash::wrap(0x12, b"anything")?;
        let mut manifest = Manifest::default();
        manifest
            .insert_record(ManifestRow {
                logical_key: single_object_path.clone(),
                hash: hash.try_into()?,
                physical_key: remote_file_url,
                ..ManifestRow::default()
            })
            .await?;

        assert!(lineage.paths.is_empty());

        // Perform the installation
        let lineage = install_paths(
            lineage,
            &mut manifest,
            domain_paths,
            package_home.clone(),
            namespace,
            &storage,
            &remote,
            &entries_paths,
        )
        .await?;

        // Verify the path is now tracked in lineage
        assert!(lineage.paths.contains_key(&single_object_path));
        // Verify the working directory contains the installed file
        assert!(storage.exists(&package_home.join(single_object_path)).await);
        // Verify the object is cached locally in `.quilt/objects`
        // Note, that we don't verify the hash and trust the manifest
        let object_path = domain_paths.object(hash.digest());
        assert!(storage.exists(object_path).await);

        Ok(())
    }

    // Nothing special, just a combination of two previous tests,
    // so we're sure that single file is not a special case.
    #[test(tokio::test)]
    async fn test_installing_multiple_paths() -> Res {
        let (home, _temp_dir1) = Home::from_temp_dir()?;
        let (domain_paths, _temp_dir2) = &DomainPaths::from_temp_dir()?;

        let namespace = Namespace::from(("foo", "bar"));
        let package_home = paths::package_home(&home, &namespace);

        // Simulate the manifest with rows containing objects
        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri::default()),
            ..PackageLineage::default()
        };
        let row_1 = ManifestRow {
            logical_key: PathBuf::from("a"),
            physical_key: "file:///ignored".to_string(),
            hash: multihash::Multihash::wrap(0x12, b"one")?.try_into()?,
            ..ManifestRow::default()
        };
        let row_2 = ManifestRow {
            logical_key: PathBuf::from("b/b"),
            physical_key: "s3://bucket/foo/bar".to_string(),
            hash: multihash::Multihash::wrap(0x12, b"two")?.try_into()?,
            ..ManifestRow::default()
        };
        let row_3 = ManifestRow {
            logical_key: PathBuf::from("c/c/c"),
            physical_key: "file:///ignored".to_string(),
            hash: multihash::Multihash::wrap(0x12, b"three")?.try_into()?,
            ..ManifestRow::default()
        };
        let row_4 = ManifestRow {
            logical_key: PathBuf::from("d/d/d/d"),
            physical_key: "s3://bucket/foo/baz".to_string(),
            hash: multihash::Multihash::wrap(0x12, b"four")?.try_into()?,
            ..ManifestRow::default()
        };
        let mut manifest = Manifest::default();
        manifest.insert_record(row_1.clone()).await?;
        manifest.insert_record(row_2.clone()).await?;
        manifest.insert_record(row_3.clone()).await?;
        manifest.insert_record(row_4.clone()).await?;

        // Simulate two of three files (1 and 3) are already exist in `.quilt/objects/HASH`
        // We trust that the hash is correct, so we can skip the actual file content
        let storage = MockStorage::default();
        let object_path_1 = home.join(domain_paths.object(row_1.hash.digest()));
        storage
            .write_byte_stream(object_path_1, ByteStream::default())
            .await?;
        let object_path_3 = home.join(domain_paths.object(row_3.hash.digest()));
        storage
            .write_byte_stream(object_path_3, ByteStream::default())
            .await?;

        // Simulate the remote object
        let remote = MockRemote::default();
        let remote_object_uri_2 = S3Uri::from_str(&row_2.physical_key)?;
        remote
            .put_object(&None, &remote_object_uri_2, Vec::new())
            .await?;
        let remote_object_uri_4 = S3Uri::from_str(&row_4.physical_key)?;
        remote
            .put_object(&None, &remote_object_uri_4, Vec::new())
            .await?;

        let entries_paths = vec![
            &row_1.logical_key,
            &row_2.logical_key,
            &row_3.logical_key,
            &row_4.logical_key,
        ];

        // Lineage does not track anything before the installation
        assert!(lineage.paths.is_empty());

        let lineage = install_paths(
            lineage,
            &mut manifest,
            domain_paths,
            package_home.clone(),
            namespace,
            &storage,
            &remote,
            &entries_paths,
        )
        .await?;

        // Now lineage tracks the files in the working directory
        assert!(lineage.paths.contains_key(&row_1.logical_key));
        assert!(lineage.paths.contains_key(&row_2.logical_key));
        assert!(lineage.paths.contains_key(&row_3.logical_key));
        assert!(lineage.paths.contains_key(&row_4.logical_key));
        // And working directory of the package contains the files
        assert!(storage.exists(&package_home.join(&row_1.logical_key)).await);
        assert!(storage.exists(&package_home.join(&row_2.logical_key)).await);
        assert!(storage.exists(&package_home.join(&row_3.logical_key)).await);
        assert!(storage.exists(&package_home.join(&row_4.logical_key)).await);

        Ok(())
    }

    // Verify that the installation fails when we try to install a path that doesn't exist in the
    // manifest.
    #[test(tokio::test)]
    async fn test_installing_path_that_doesnt_exists_in_manifest() -> Res {
        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri::default()),
            ..PackageLineage::default()
        };
        let remote = MockRemote::default();
        let storage = MockStorage::default();

        let not_existed = PathBuf::from("z/z");
        // We want to install z/z
        let entries_paths = vec![&not_existed];
        // But manifest clearly doens't contain it
        let mut manifest = fixtures::manifest_with_objects_all_sizes::manifest().await?;

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

    #[test(tokio::test)]
    async fn test_installing_more_than_1024_paths() -> Res {
        let (home, _temp_dir1) = Home::from_temp_dir()?;
        let (domain_paths, _temp_dir2) = &DomainPaths::from_temp_dir()?;

        let namespace = Namespace::from(("foo", "bar"));
        let package_home = paths::package_home(&home, &namespace);

        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri::default()),
            ..PackageLineage::default()
        };
        let storage = MockStorage::default();
        let remote = MockRemote::default();

        let mut manifest = Manifest::default();
        let mut entries_paths = Vec::new();
        let mut path_refs = Vec::new();

        // Create 1024 * 2 test paths and rows
        for i in 0..2048 {
            let path = PathBuf::from(format!("path_{}.txt", i));
            let place = format!("s3://bucket/path_{}.txt", i);
            let hash = multihash::Multihash::wrap(0x12, format!("hash_{}", i).as_bytes())?;

            let row = ManifestRow {
                logical_key: path.clone(),
                physical_key: place.clone(),
                hash: hash.try_into()?,
                ..ManifestRow::default()
            };

            manifest.insert_record(row).await?;
            entries_paths.push(path);

            // Simulate remote objects
            let remote_uri = S3Uri::from_str(&place)?;
            remote.put_object(&None, &remote_uri, Vec::new()).await?;
        }

        // Create references for the function call
        for path in &entries_paths {
            path_refs.push(path);
        }

        domain_paths
            .scaffold_for_installing(&storage, &home, &namespace)
            .await?;

        assert!(lineage.paths.is_empty());

        let lineage = install_paths(
            lineage,
            &mut manifest,
            domain_paths,
            package_home.clone(),
            namespace,
            &storage,
            &remote,
            &path_refs,
        )
        .await?;

        // Verify all 2048 paths are tracked in lineage
        assert_eq!(lineage.paths.len(), 2048);
        for path in &entries_paths {
            assert!(lineage.paths.contains_key(path));
        }

        // Verify all files exist in working directory
        for path in &entries_paths {
            assert!(storage.exists(&package_home.join(path)).await);
        }

        Ok(())
    }

    // TODO: fail if path is already installed
    // TODO: fail if manifest entry has invalid URL
}
