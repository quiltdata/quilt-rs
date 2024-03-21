use std::{
    collections::{hash_map::RandomState, BTreeMap, HashSet},
    path::PathBuf,
};

use arrow::error::ArrowError;
use aws_sdk_s3::{
    error::SdkError,
    types::{ChecksumAlgorithm, CompletedMultipartUpload, CompletedPart},
};
use aws_smithy_types::byte_stream::{ByteStream, Length};
use base64::{prelude::BASE64_STANDARD, Engine};
use multihash::Multihash;
use parquet::data_type::AsBytes;
use serde_json::json;
use tokio::{
    fs::{create_dir_all, remove_dir_all, File},
    io::{AsyncReadExt, AsyncWriteExt},
};
use url::Url;

pub mod lineage;
pub mod manifest;
pub mod manifest_handle;
pub mod status;
pub mod storage;
pub mod uri;

use crate::{
    paths,
    quilt4::{
        checksum::{calculate_sha256_checksum, get_checksum_chunksize_and_parts},
        table::HEADER_ROW,
    },
    s3_utils, Error, Row4, Table, UPath,
};

use self::manifest::MULTIHASH_SHA256_CHUNKED;
pub use self::{
    // context::Context,
    lineage::{CommitState, DomainLineage, PackageLineage, PathState},
    manifest::{ContentHash, Manifest, ManifestHeader, ManifestRow},
    manifest_handle::{CachedManifest, InstalledManifest, ReadableManifest, RemoteManifest},
    status::{
        Change, ChangeSet, InstalledPackageStatus, PackageFileFingerprint, UpstreamDiscreteState,
        UpstreamState,
    },
    storage::{fs, s3},
    uri::{RevisionPointer, S3PackageUri},
};

// XXX: is this necessary?
#[derive(Debug, PartialEq, Eq)]
struct S3Domain {
    bucket: String,
}

impl From<&S3PackageUri> for S3Domain {
    fn from(uri: &S3PackageUri) -> Self {
        Self {
            bucket: uri.bucket.clone(),
        }
    }
}

async fn cache_manifest(
    paths: &paths::DomainPaths,
    manifest: &Table,
    bucket: &str,
    hash: &str,
) -> Result<PathBuf, ArrowError> {
    let cache_path = paths.manifest_cache(bucket, hash);
    create_dir_all(&cache_path.parent().unwrap()).await?;
    manifest
        .write_to_upath(&UPath::Local(cache_path.clone()))
        .await
        .map(|_| cache_path)
}

// FIMXE: CachedManifest::browse(&RemoteManifest)
//        or RemoteManifest::browse -> CachedManifest
//        or CachedManifest::try_from(RemoteManifest)
async fn cache_remote_manifest(
    paths: &paths::DomainPaths,
    manifest: &RemoteManifest,
) -> Result<impl ReadableManifest, Error> {
    // check if the manifest is already cached
    // if not, download and cache it
    // return cached manifest

    let cache_path = paths.manifest_cache(&manifest.bucket, &manifest.hash);

    // TODO: who is responsible for this?
    create_dir_all(&cache_path.parent().unwrap()).await?;

    if !fs::exists(&cache_path).await {
        // Does not exist yet
        let client = crate::s3_utils::get_client_for_bucket(&manifest.bucket).await?;

        let result = client
            .get_object()
            .bucket(&manifest.bucket)
            .key(paths::get_manifest_key(&manifest.hash))
            .send()
            .await;

        match result {
            Ok(output) => {
                let mut contents = Vec::new();
                output
                    .body
                    .into_async_read()
                    .read_to_end(&mut contents)
                    .await?;
                fs::write(&cache_path, &contents).await?;
            }
            Err(SdkError::ServiceError(err)) if err.err().is_no_such_key() => {
                // Fallback: Download the JSONL manifest.
                let result = client
                    .get_object()
                    .bucket(&manifest.bucket)
                    .key(paths::get_manifest_key(&manifest.hash))
                    .send()
                    .await
                    .map_err(|err| Error::S3(err.to_string()))?;

                let quilt3_manifest = Manifest::from_file(result.body.into_async_read()).await?;
                let header = Row4 {
                    name: HEADER_ROW.into(),
                    place: HEADER_ROW.into(),
                    path: None,
                    size: 0,
                    hash: Multihash::default(),
                    info: serde_json::json!({
                        "message": quilt3_manifest.header.message,
                        "version": quilt3_manifest.header.version,
                    }),
                    meta: match quilt3_manifest.header.user_meta {
                        Some(meta) => meta.into(),
                        None => serde_json::Value::Null,
                    },
                };
                let mut records = BTreeMap::new();
                for row in quilt3_manifest.rows {
                    let mut info = row.meta.unwrap_or_default();
                    let meta = info.remove("user_meta").unwrap_or_default();
                    records.insert(
                        row.logical_key.clone(),
                        Row4 {
                            name: row.logical_key,
                            place: row.physical_key,
                            path: None,
                            size: row.size,
                            hash: row.hash.try_into()?,
                            info: info.into(),
                            meta,
                        },
                    );
                }
                let table = Table { header, records };
                table.write_to_upath(&UPath::Local(cache_path)).await?
            }
            Err(err) => {
                return Err(Error::S3(err.to_string()));
            }
        }
    }

    Ok(CachedManifest {
        paths: paths.clone(),
        bucket: manifest.bucket.clone(),
        hash: manifest.hash.clone(),
    })
}

async fn browse_remote_manifest(
    paths: &paths::DomainPaths,
    remote: &RemoteManifest,
) -> Result<Table, Error> {
    cache_remote_manifest(paths, remote).await?.read().await
}

async fn copy_cached_to_installed(
    paths: &paths::DomainPaths,
    cached_manifest_bucket: &str,
    installed_manifest_namespace: &str,
    hash: &str,
) -> Result<(), Error> {
    tokio::fs::copy(
        paths.manifest_cache(cached_manifest_bucket, hash),
        paths.installed_manifest(installed_manifest_namespace, hash),
    )
    .await?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalDomain {
    pub paths: paths::DomainPaths,
    lineage: lineage::DomainLineageIo,
}

impl LocalDomain {
    pub fn new(root_dir: PathBuf) -> Self {
        let paths = paths::DomainPaths::new(root_dir.clone());
        let lineage = lineage::DomainLineageIo::new(paths.lineage());
        Self { paths, lineage }
    }

    pub async fn browse_remote_manifest(&self, remote: &RemoteManifest) -> Result<Table, Error> {
        browse_remote_manifest(&self.paths, remote).await
    }

    pub async fn install_package(
        &self,
        remote: &RemoteManifest,
    ) -> Result<InstalledPackage, Error> {
        // Read the lineage
        let lineage: DomainLineage = self.lineage.read().await?;

        // bail if already installed
        // TODO: if compatible (same remote), just return the installed package
        if lineage.packages.contains_key(&remote.namespace) {
            return Err(Error::PackageAlreadyInstalled(remote.namespace.clone()));
        }

        cache_remote_manifest(&self.paths, remote).await?;

        // Make an "installed" copy of the remote manifest.
        let installed_manifest_path = self
            .paths
            .installed_manifest(&remote.namespace, &remote.hash);
        create_dir_all(&installed_manifest_path.parent().unwrap()).await?;
        copy_cached_to_installed(&self.paths, &remote.bucket, &remote.namespace, &remote.hash)
            .await?;

        // Create the identity cache dir.
        let objects_dir = self.paths.objects_dir();
        create_dir_all(&objects_dir).await?;

        // Create the working dir.
        let working_dir = self.paths.working_dir(&remote.namespace);
        create_dir_all(&working_dir).await?;

        // Resolve and record latest manifest hash
        let latest_hash = remote.resolve_latest().await?;
        // Update the lineage (with empty paths).
        let mut lineage = lineage;
        lineage.packages.insert(
            remote.namespace.clone(),
            PackageLineage::from_remote(remote.to_owned(), latest_hash),
        );
        self.lineage.write(&lineage).await?;

        // Create the package.
        Ok(InstalledPackage {
            paths: self.paths.clone(),
            lineage: self
                .lineage
                .create_package_lineage(remote.namespace.clone()),
            namespace: remote.namespace.clone(),
        })
    }

    pub async fn uninstall_package(&self, namespace: impl AsRef<str>) -> Result<(), Error> {
        let namespace = namespace.as_ref();
        let mut lineage = self.lineage.read().await?;

        lineage
            .packages
            .remove(namespace)
            .ok_or(Error::PackageNotInstalled(namespace.to_owned()))?;

        self.lineage.write(&lineage).await?;

        if let Err(err) = remove_dir_all(self.paths.installed_manifests(namespace)).await {
            println!("Failed to remove installed manifests: {err}");
        }
        if let Err(err) = remove_dir_all(self.paths.working_dir(namespace)).await {
            println!("Failed to remove working directory: {err}");
        }

        // TODO: Remove object files? But need to make sure no other manifest uses them.

        Ok(())
    }

    pub async fn list_installed_packages(&self) -> Result<Vec<InstalledPackage>, Error> {
        let lineage = self.lineage.read().await?;
        let mut namespaces: Vec<String> = lineage.packages.into_keys().collect();
        namespaces.sort();
        let packages = namespaces
            .into_iter()
            .map(|namespace| InstalledPackage {
                lineage: self.lineage.create_package_lineage(namespace.clone()),
                paths: self.paths.clone(),
                namespace,
            })
            .collect();
        Ok(packages)
    }

    pub async fn get_installed_package(
        &self,
        namespace: &str,
    ) -> Result<Option<InstalledPackage>, Error> {
        let lineage = self.lineage.read().await?;
        if lineage.packages.contains_key(namespace) {
            Ok(Some(InstalledPackage {
                paths: self.paths.clone(),
                lineage: self.lineage.create_package_lineage(namespace.to_string()),
                namespace: namespace.to_string(),
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn package_s3_prefix(
        &self,
        uri: &s3::S3Uri,
        target_uri: S3PackageUri,
    ) -> Result<RemoteManifest, Error> {
        println!("Source URI: {:?}, target URI: {:?}", uri, target_uri);
        // TODO: make get_object_attributes() calls concurrently across list_objects() pages
        // TODO: increase concurrency, to do that we need to figure out how to deal
        //       with fd limits on Mac by default it's 256
        // TODO: s3 uri key ends with / and has no version
        // FIXME: filter or fail on keys with `.` or `..` in path segments as quilt3 do
        let client = crate::s3_utils::get_client_for_bucket(&uri.bucket).await?;

        // XXX: we need real API to build manifests
        let header = Row4 {
            name: HEADER_ROW.into(),
            place: HEADER_ROW.into(),
            path: None,
            size: 0,
            hash: Multihash::default(),
            info: serde_json::json!({
                "message": serde_json::Value::Null, // TODO: commit message?
                "version": "v0", // XXX: is this correct?
            }),
            meta: serde_json::Value::Null, // TODO: accept user meta?
        };
        let mut records: BTreeMap<String, Row4> = BTreeMap::new();

        let prefix_len = uri.key.len();
        let mut p = client
            .list_objects_v2()
            .bucket(&uri.bucket)
            .prefix(&uri.key)
            .into_paginator()
            .page_size(100) // XXX: this is to limit concurrency
            .send();
        while let Some(page) = p.next().await {
            let page = page.map_err(|err| Error::S3(err.to_string()))?;
            let page_contents_iter = page.contents.iter().flatten();

            async fn _get_obj_attrs<'a>(
                client: aws_sdk_s3::Client,
                bucket: &str,
                key: &'a str,
            ) -> Result<
                (
                    &'a str,
                    aws_sdk_s3::operation::get_object_attributes::GetObjectAttributesOutput,
                ),
                Error,
            > {
                let attrs = client
                    .get_object_attributes()
                    .bucket(bucket)
                    .key(key)
                    .object_attributes(aws_sdk_s3::types::ObjectAttributes::Checksum)
                    .object_attributes(aws_sdk_s3::types::ObjectAttributes::ObjectParts)
                    .object_attributes(aws_sdk_s3::types::ObjectAttributes::ObjectSize)
                    .max_parts(storage::s3::MPU_MAX_PARTS as i32)
                    .send()
                    .await
                    .map_err(|err| Error::S3(err.to_string()))?;
                Ok((key, attrs))
            }

            for (key, attrs) in futures::future::try_join_all(page_contents_iter.map(|obj| {
                _get_obj_attrs(
                    client.clone(),
                    &uri.bucket,
                    obj.key.as_ref().expect("object key expected to be present"),
                )
            }))
            .await?
            {
                // Can happen if object is removed after it was listed but before attributes retrieved.
                if attrs.delete_marker.is_some() {
                    assert!(attrs.delete_marker.unwrap());
                    continue;
                }
                let name = &key[prefix_len..];
                // FIXME: we assume that objects have hash and it's compatible with sha-256-chunked
                let s3_checksum = s3_utils::get_compliant_chunked_checksum(&attrs).unwrap();
                let hash =
                    Multihash::wrap(MULTIHASH_SHA256_CHUNKED, s3_checksum.as_bytes()).unwrap();
                records.insert(
                    name.into(),
                    Row4 {
                        name: name.into(),
                        place: s3::make_s3_url(&uri.bucket, key, attrs.version_id.as_deref())
                            .into(),
                        path: None, // WTF is this?
                        // XXX: can we use `as u64` safely here?
                        size: attrs
                            .object_size
                            .expect("object_size is expected because it was requested")
                            as u64,
                        hash,
                        info: serde_json::Value::Null, // XXX: is this right?
                        meta: serde_json::Value::Null, // XXX: is this right?
                    },
                );
            }
        }

        let table = Table { header, records };
        let new_remote = RemoteManifest {
            bucket: target_uri.bucket,
            namespace: target_uri.namespace,
            hash: table.top_hash(),
        };
        let cache_path =
            cache_manifest(&self.paths, &table, &new_remote.bucket, &new_remote.hash).await?;
        new_remote.upload_from(&cache_path).await?;
        new_remote.upload_legacy(&table).await?;
        let top_hash = table.top_hash();
        new_remote
            .put_timestamp_tag(chrono::Utc::now(), &top_hash)
            .await?;
        new_remote.update_latest(&top_hash).await?;

        Ok(new_remote)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InstalledPackage {
    paths: paths::DomainPaths,
    lineage: lineage::PackageLineageIo,
    pub namespace: String,
}

impl InstalledPackage {
    pub async fn entries_paths(&self) -> Result<Vec<String>, Error> {
        self.lineage
            .read()
            .await
            .map(|l| l.paths.into_keys().collect())
    }

    async fn manifest(&self) -> Result<impl ReadableManifest, Error> {
        // read recorded hash
        // get installed manifest
        self.lineage.read().await.map(|l| {
            InstalledManifest::new(
                self.namespace.to_string(),
                l.current_hash().to_string(),
                self.paths.clone(),
            )
        })
    }

    pub fn working_folder(&self) -> PathBuf {
        self.paths.working_dir(&self.namespace)
    }

    // pub async fn uninstall(&self) -> Result<(), Error> {
    //     self.domain.uninstall_package(&self.namespace).await
    // }

    pub async fn status(&self) -> Result<status::InstalledPackageStatus, Error> {
        // compute the status based on the following sources:
        //   - the cached manifest
        //   - paths
        //   - working directory state
        // installed entries marked as "installed" (initially as "downloading")
        // modified entries marked as "modified", etc

        let lineage = self.lineage.read().await?;
        // try updating the latest hash
        if let Ok(latest_hash) = lineage.remote.resolve_latest().await {
            let mut lineage = lineage.clone();
            lineage.latest_hash = latest_hash;
            self.lineage.write(lineage.clone()).await?;
        }
        InstalledPackageStatus::create(&lineage, &self.manifest().await?, self.working_folder())
            .await
    }

    pub async fn install_paths(&self, paths: &Vec<String>) -> Result<(), Error> {
        if paths.is_empty() {
            return Ok(());
        }

        let mut lineage = self.lineage.read().await?;

        // TODO: what happens if paths are already installed? Ignore, or error?
        if !HashSet::<String, RandomState>::from_iter(lineage.paths.keys().cloned())
            .is_disjoint(&HashSet::from_iter(paths.to_owned()))
        {
            return Err(Error::InstallPath(
                "some paths are already installed".to_string(),
            ));
        }

        let objects_dir = self.paths.objects_dir();
        let working_dir = self.working_folder();

        // for each path in paths:
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

        let mut table = self.manifest().await?.read().await?;

        for path in paths {
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

            row.place = Url::from_file_path(&object_dest).unwrap().to_string();

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
        let installed_manifest_path = self
            .paths
            .installed_manifest(&self.namespace, lineage.current_hash());

        table
            .write_to_upath(&UPath::Local(installed_manifest_path))
            .await?;

        self.lineage.write(lineage).await?;

        Ok(())
    }

    pub async fn uninstall_paths(&self, paths: &Vec<String>) -> Result<(), Error> {
        println!("uninstall_paths: {paths:?}");

        let mut lineage = self.lineage.read().await?;

        let working_dir = self.working_folder();
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

        self.lineage.write(lineage).await?;

        // TODO: Remove unused files in OBJECTS_DIR?

        Ok(())
    }

    pub async fn revert_paths(&self, paths: &Vec<String>) -> Result<(), Error> {
        println!("revert_paths: {paths:?}");
        unimplemented!()
    }

    pub async fn commit(
        &self,
        message: String,
        user_meta: Option<manifest::JsonObject>,
    ) -> Result<(), Error> {
        println!("commit: {message:?}, {user_meta:?}");
        // create a new manifest based on the stored version

        // for each modified file:
        //   - compute the new hash
        //   - store in the identity cache at $LOCAL/.quilt/objects/<hash>
        //   - update the modified entries in the manifest with the new physical keys
        //     pointing to the new objects in the identity cache
        //   - ? set entry.meta.pulled_hashes to previous object hash?
        //   - ? set entry.meta.remote_key to the remote's physical key?

        // compute the new top hash
        // store the new manifest under the new top hash at $LOCAL/.quilt/packages/<hash>
        // XXX: prefix with the namespace?
        // XXX: what to do on collisions?
        //      e.g. when a file was changed, committed, and then reverted

        // store revision pointers to the newly created manifest
        //   - in the local registry??
        //   - in the lineage
        //     - commit:
        //       - timestamp
        //       - user ?
        //       - multihash: new_top_hash
        //       - pulled_hashes: [old_top_hash] ?
        //       - paths:
        //         - [modified file's path]:
        //           - multihash
        //           # XXX: do we actually need this? can be inferred from namespace + logical key
        //           - remote_key: "s3://..." # no version id
        //           - local_key: $LOCAL/.quilt/objects/<hash>
        //           - pulled_hashes: [old_hash] ?
        // NOTE: each commit MUST include all paths from prior commits
        //       (since the last pull, until reset by a sync)

        let mut package_lineage = self.lineage.read().await?;

        // TODO: Maybe have the user pass this as an argument?
        let status = self.status().await?;

        let objects_dir = self.paths.objects_dir();
        // TODO: This should really be done when the domain is created.
        create_dir_all(&objects_dir).await?;

        let work_dir = self.working_folder();

        let mut table = self.manifest().await?.read().await?;

        for (logical_key, Change { current, previous }) in status.changes {
            if let Some(previous) = previous {
                let removed = table
                    .records
                    .remove(&logical_key)
                    .ok_or(Error::Commit(format!("cannot remove {}", logical_key)))?;
                if removed.size != previous.size || removed.hash != previous.hash {
                    return Err(Error::Commit(format!(
                        "unexpected size or hash for removed {}",
                        logical_key
                    )));
                }
                package_lineage.paths.remove(&logical_key);
            }
            if let Some(current) = current {
                let object_dest = objects_dir.join(hex::encode(current.hash.digest()));
                let new_physical_key = Url::from_file_path(&object_dest).unwrap().into();

                if table
                    .records
                    .insert(
                        logical_key.to_owned(),
                        Row4 {
                            name: logical_key.to_owned(),
                            place: new_physical_key,
                            path: None,
                            size: current.size,
                            hash: current.hash,
                            info: serde_json::Value::default(),
                            meta: serde_json::Value::default(),
                        },
                    )
                    .is_some()
                {
                    return Err(Error::Commit(format!("cannot overwrite {}", logical_key)));
                }

                let work_dest = work_dir.join(&logical_key);
                if !fs::exists(&object_dest).await {
                    tokio::fs::copy(&work_dest, object_dest).await?;
                }
                package_lineage.paths.insert(
                    logical_key,
                    PathState {
                        timestamp: fs::get_file_modified_ts(&work_dest).await?,
                        hash: current.hash,
                    },
                );
            }
        }

        table.header.info = json!({
            "message": message,
            "version": "v0",
        });
        if let Some(user_meta) = user_meta {
            table.header.meta = user_meta.into();
        }

        let new_top_hash = table.top_hash();

        let new_manifest_path = self
            .paths
            .installed_manifest(&self.namespace, &new_top_hash);

        table
            .write_to_upath(&UPath::Local(new_manifest_path))
            .await?;

        let mut prev_hashes = Vec::new();
        if let Some(commit) = &package_lineage.commit {
            prev_hashes.push(commit.hash.to_owned());
            prev_hashes.extend(commit.prev_hashes.to_owned());
        }
        let commit = CommitState {
            hash: new_top_hash,
            timestamp: chrono::Utc::now(),
            prev_hashes,
        };
        package_lineage.commit = Some(commit);

        self.lineage.write(package_lineage).await?;

        Ok(())
    }

    pub async fn push(&self) -> Result<(), Error> {
        let mut lineage = self.lineage.read().await?;

        let commit = match lineage.commit {
            None => return Ok(()), // nothing to commit
            Some(commit) => commit,
        };

        let remote = &lineage.remote;

        let mut local_manifest = self.manifest().await?.read().await?;
        let remote_manifest = browse_remote_manifest(&self.paths, remote).await?;

        // ## copy data
        // Copy each of the _modified_ paths from their local_key to remote_key,
        // keeping track of the resulting versionIds
        //
        // TODO: FAIL if the remote bucket does NOT support versioning (as it would be destructive)
        let client = crate::s3_utils::get_client_for_bucket(&remote.bucket).await?;

        // ignore removed items, upload changed and new items
        for row in local_manifest.records.values_mut() {
            if let Some(remote_row) = remote_manifest.records.get(&row.name) {
                if remote_row.eq(row) {
                    row.place = remote_row.place.to_owned();
                    continue;
                }
            }

            let local_url = Url::parse(&row.place).unwrap();
            let file_path: PathBuf = local_url.to_file_path().unwrap();

            let s3_key = format!("{}/{}", self.namespace, row.name);
            println!("uploading to s3({}): {}", remote.bucket, s3_key);

            // TODO: upload in parallel. use a stream?
            let (version_id, checksum) = if row.size < storage::s3::MULTIPART_THRESHOLD {
                let body = ByteStream::read_from().path(&file_path).build().await?;

                let response = client
                    .put_object()
                    .bucket(&remote.bucket)
                    .key(&s3_key)
                    .body(body)
                    .checksum_algorithm(ChecksumAlgorithm::Sha256)
                    .send()
                    .await
                    .map_err(|err| Error::S3(err.to_string()))?;

                let s3_checksum_b64 = response
                    .checksum_sha256
                    .ok_or(Error::Checksum("missing checksum".to_string()))?;

                let s3_checksum = BASE64_STANDARD.decode(s3_checksum_b64)?;

                let checksum = if row.size == 0 {
                    // Edge case: a 0-byte upload is treated as an empty list of chunks, rather than
                    // a list of a 0-byte chunk. Its checksum is sha256(''), NOT sha256(sha256('')).
                    s3_checksum
                } else {
                    calculate_sha256_checksum(s3_checksum.as_ref())
                        .await
                        .unwrap()
                        .to_vec()
                };

                (response.version_id, checksum)
            } else {
                let (chunksize, num_chunks) = get_checksum_chunksize_and_parts(row.size);
                let upload_id = client
                    .create_multipart_upload()
                    .bucket(&remote.bucket)
                    .key(&s3_key)
                    .checksum_algorithm(ChecksumAlgorithm::Sha256)
                    .send()
                    .await
                    .map_err(|err| Error::S3(err.to_string()))?
                    .upload_id
                    .ok_or(Error::UploadId("failed to get an UploadId".to_string()))?;

                let mut parts: Vec<CompletedPart> = Vec::new();
                for chunk_idx in 0..num_chunks {
                    let part_number = chunk_idx as i32 + 1;
                    let offset = chunk_idx * chunksize;
                    let length = chunksize.min(row.size - offset);
                    let chunk_body = ByteStream::read_from()
                        .path(&file_path)
                        .offset(offset)
                        .length(Length::Exact(length)) // https://github.com/awslabs/aws-sdk-rust/issues/821
                        .build()
                        .await?;
                    let part_response = client
                        .upload_part()
                        .bucket(&remote.bucket)
                        .key(&s3_key)
                        .upload_id(&upload_id)
                        .part_number(part_number)
                        .checksum_algorithm(ChecksumAlgorithm::Sha256)
                        .body(chunk_body)
                        .send()
                        .await
                        .map_err(|err| {
                            Error::S3(format!("failed to upload part {}: {}", part_number, err))
                        })?;
                    parts.push(
                        CompletedPart::builder()
                            .part_number(part_number)
                            .e_tag(part_response.e_tag.unwrap_or_default())
                            .checksum_sha256(part_response.checksum_sha256.unwrap_or_default())
                            .build(),
                    );
                }

                let response = client
                    .complete_multipart_upload()
                    .bucket(&remote.bucket)
                    .key(&s3_key)
                    .upload_id(&upload_id)
                    .multipart_upload(
                        CompletedMultipartUpload::builder()
                            .set_parts(Some(parts))
                            .build(),
                    )
                    .send()
                    .await
                    .map_err(|err| {
                        Error::S3(format!("failed to complete multipart upload: {}", err))
                    })?;

                let s3_checksum = response
                    .checksum_sha256
                    .ok_or(Error::Checksum("missing checksum".to_string()))?;
                let (checksum_b64, _) = s3_checksum
                    .split_once('-')
                    .ok_or(Error::Checksum("unexpected checksum".to_string()))?;
                let checksum = BASE64_STANDARD.decode(checksum_b64)?;

                (response.version_id, checksum)
            };

            // Update the manifest with the sha2-256-chunked checksum.
            row.hash = Multihash::wrap(MULTIHASH_SHA256_CHUNKED, checksum.as_ref()).unwrap();

            let remote_url = s3::make_s3_url(&remote.bucket, &s3_key, version_id.as_deref());
            println!("got remote url: {}", remote_url);

            // "Relax" the manifest by using those new remote keys
            row.place = remote_url.to_string();
        }

        let top_hash = local_manifest.top_hash();
        let new_remote = RemoteManifest {
            hash: top_hash.clone(),
            ..remote.clone()
        };

        // Cache the relaxed manifest
        let cache_path = cache_manifest(
            &self.paths,
            &local_manifest,
            &new_remote.bucket,
            &new_remote.hash,
        )
        .await?;

        // Push the (cached) relaxed manifest to the remote, don't tag it yet
        new_remote.upload_from(&cache_path).await?;

        // Upload a quilt3 manifest for backward compatibility.
        new_remote.upload_legacy(&local_manifest).await?;

        println!("uploaded remote manifest: {new_remote:?}");

        // Tag the new commit.
        // If {self.commit.tag} does not already exist at
        // {self.remote}/.quilt/named_packages/{self.namespace},
        // create it with the value of {self.commit.hash}
        // TODO: Otherwise try again with the current timestamp as the tag
        // (e.g., try five times with exponential backoff, then Error)
        new_remote
            .put_timestamp_tag(commit.timestamp, &new_remote.hash)
            .await?;

        // Check the hash of remote's latest manifest
        lineage.latest_hash = new_remote.resolve_latest().await?;
        lineage.remote = new_remote;

        // Reset the commit state.
        lineage.commit = None;

        // Try certifying latest if tracking
        if lineage.base_hash == lineage.latest_hash {
            // remote latest has not been updated, certifying the new latest
            lineage.remote.update_latest(&top_hash).await?;
            lineage.latest_hash = top_hash.clone();
            lineage.base_hash = top_hash.clone();
        }

        self.lineage.write(lineage).await?;

        Ok(())
    }

    pub async fn pull(&self) -> Result<(), Error> {
        let status = self.status().await?;
        if !status.changes.is_empty() {
            return Err(Error::Package("package has pending changes".to_string()));
        }

        let lineage = self.lineage.read().await?;
        if lineage.commit.is_some() {
            return Err(Error::Package("package has pending commits".to_string()));
        }
        if lineage.remote.hash != lineage.base_hash {
            return Err(Error::Package("package has diverged".to_string()));
        }
        // TODO: do we need to explicitly update latest_hash?
        // status() tries to update, but may fail.
        if lineage.base_hash == lineage.latest_hash {
            return Err(Error::Package("package is already up-to-date".to_string()));
        }

        // TODO: What should we do about installed paths?
        // They may or may not exist in the updated package.
        let paths: Vec<String> = lineage.paths.keys().cloned().collect();
        self.uninstall_paths(&paths).await?;

        // TODO: uninstall_paths() just modified the lineage, so re-reading it here.
        // There needs to be a better way.
        let mut lineage = self.lineage.read().await?;
        lineage.remote.hash = lineage.latest_hash.clone();
        lineage.base_hash = lineage.latest_hash.clone();

        cache_remote_manifest(&self.paths, &lineage.remote).await?;
        copy_cached_to_installed(
            &self.paths,
            &lineage.remote.bucket,
            &self.namespace,
            &lineage.remote.hash,
        )
        .await?;

        self.lineage.write(lineage).await?;

        let manifest = self.manifest().await?.read().await?;
        let paths_to_install = paths
            .into_iter()
            .filter(|x| manifest.records.contains_key(x))
            .collect();
        self.install_paths(&paths_to_install).await?;

        Ok(())
    }

    pub async fn certify_latest(&self) -> Result<(), Error> {
        let mut lineage = self.lineage.read().await?;
        let new_latest = lineage.remote.hash.clone();
        lineage.remote.update_latest(&new_latest).await?;
        lineage.latest_hash = new_latest.clone();
        lineage.base_hash = new_latest;
        self.lineage.write(lineage).await
    }

    pub async fn reset_to_latest(&self) -> Result<(), Error> {
        let lineage = self.lineage.read().await?;

        let new_latest = lineage.remote.resolve_latest().await?;
        if new_latest == lineage.remote.hash {
            // already at latest
            return Ok(());
        }

        let paths: Vec<String> = lineage.paths.into_keys().collect();
        self.uninstall_paths(&paths).await?;
        let mut lineage = self.lineage.read().await?;

        lineage.latest_hash = new_latest.clone();
        lineage.remote.hash = new_latest.clone();
        lineage.base_hash = new_latest;

        cache_remote_manifest(&self.paths, &lineage.remote).await?;
        copy_cached_to_installed(
            &self.paths,
            &lineage.remote.bucket,
            &self.namespace,
            &lineage.remote.hash,
        )
        .await?;

        self.lineage.write(lineage).await?;

        let manifest = self.manifest().await?.read().await?;
        let paths_to_install = paths
            .into_iter()
            .filter(|x| manifest.records.contains_key(x))
            .collect();
        self.install_paths(&paths_to_install).await
    }
}

// a conflict is identified by the identifiers of the two conflicting manifests
#[derive(Debug, PartialEq)]
pub struct Conflict {
    package: InstalledPackage,
    changes: ChangeSet<String, PackageFileFingerprint>,
    folder: PathBuf,
}

#[cfg(test)]
mod tests {
    use super::*;
    use temp_testdir::TempDir;
    use tokio_test::{assert_err, block_on};

    use crate::quilt::manifest::MULTIHASH_SHA256;

    fn get_timestamp() -> String {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string()
    }

    #[test]
    #[ignore]
    fn flow() {
        // ## Setup
        let test_uri_string = "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md";

        let test_uri: S3PackageUri = test_uri_string.parse().expect("Failed to parse URI");
        assert_eq!(
            test_uri,
            S3PackageUri {
                bucket: "udp-spec".into(),
                namespace: "spec/quiltcore".into(),
                path: Some("READ ME.md".into()),
                revision: RevisionPointer::default(),
            }
        );

        // TODO: abstract s3 and fs, inject mocked versions
        // let root_fs = fs::MemoryFS::from_strs([("/", "")]);
        //
        // let stage_bucket = fs::MemoryFS::from_strs([("key", "contents")]);

        // let buckets = s3::MemoryBuckets::from([(String::from("quilt-example"), stage_bucket)]);

        // let context = TestContext::new(&root_fs, &buckets);

        // let local_path = UPath(scheme: "file", path: "$HOME/Documents/QuiltSync")
        // let local_path = "/home/quilt-sync";
        let temp_dir = TempDir::default();
        let local_path = PathBuf::from(temp_dir.as_ref());
        let local_domain = LocalDomain::new(local_path);

        // ## Pull the manifest

        let remote_manifest =
            block_on(RemoteManifest::resolve(&test_uri)).expect("Failed to resolve manifest");

        let cached_manifest =
            block_on(cache_remote_manifest(&local_domain.paths, &remote_manifest))
                .expect("Failed to cache the manifest");

        let manifest = block_on(cached_manifest.read()).expect("Failed to parse the manifest");

        println!("manifest: {manifest:?}");
        // TODO: assert manifest has the expected contents

        // ## Install the files
        //
        // installed_package knows its domain and install folder
        // downloads manifest for latest revision into an editable working copy
        // creates install folder and Lineages it to remote_package_uri

        let paths = vec![test_uri.path.unwrap()];
        let installed_package = block_on(local_domain.install_package(&remote_manifest))
            .expect("Failed to install package");
        block_on(installed_package.install_paths(&paths)).expect("Failed to install paths");

        // Can only install it once.
        assert_err!(block_on(installed_package.install_paths(&paths)));

        // TODO: assert files are installed

        // List filenames in Manifest
        // XXX: does this list the files in the working directory or in the manifest or a
        // combination of those?
        // for entry in installed_package.entries() {
        //     println!("{:?}", entry.logical_key);
        // }

        let status = block_on(installed_package.status()).expect("Failed to get status");
        assert_eq!(status, InstalledPackageStatus::default());

        // ## Modify installed files

        let readme_path = installed_package.working_folder().join("READ ME.md");
        println!("readme_path: {readme_path:?}");

        let old_readme =
            block_on(fs::read_to_string(&readme_path)).expect("Failed to read 'READ ME.md'");

        let timestamp = get_timestamp();
        println!("timestamp: {timestamp:?}");
        block_on(fs::write(readme_path, timestamp.as_bytes()))
            .expect("Failed to overwrite 'READ ME.md'");
        let status = block_on(installed_package.status()).expect("Failed to get status");
        let expected_status = InstalledPackageStatus::new(
            UpstreamState::default(),
            ChangeSet::from([(
                "READ ME.md".into(),
                Change {
                    current: Some(PackageFileFingerprint {
                        size: timestamp.len() as u64,
                        hash: Multihash::wrap(
                            MULTIHASH_SHA256,
                            block_on(calculate_sha256_checksum(timestamp.as_bytes()))
                                .unwrap()
                                .as_ref(),
                        )
                        .unwrap(),
                    }),
                    previous: Some(PackageFileFingerprint {
                        size: old_readme.len() as u64,
                        hash: Multihash::wrap(
                            MULTIHASH_SHA256,
                            block_on(calculate_sha256_checksum(old_readme.as_bytes()))
                                .unwrap()
                                .as_ref(),
                        )
                        .unwrap(),
                    }),
                },
            )]),
        );
        assert_eq!(status, expected_status);

        // ## Commit

        let commit_message = format!("Commit made at {}", timestamp);
        let user_meta = serde_json::json!({
            "test": "value",
            "timestamp": timestamp,
        })
        .as_object()
        .unwrap()
        .to_owned();

        // commit local state to the local manifest,
        // accumulate commit metadata in lineage
        block_on(installed_package.commit(commit_message, Some(user_meta)))
            .expect("Failed to commit");

        // // ## Sync
        // let remote_latest = block_on(installed_package.push())
        //     .expect("Failed to push")
        //     .expect("Expected to return diverged remote latest");
        //
        // // TODO: inject new remote latest
        // let expected_remote_latest = RemoteManifest {
        //     bucket: "quilt-example".into(),
        //     namespace: "akarve/test_dest".into(),
        //     hash: "abc".into(),
        // };
        //
        // assert_eq!(remote_latest, expected_remote_latest);
        //
        // // ## Merge
        // // latest not certified -- merging
        //
        // let merge = block_on(installed_package.merge(remote_latest)).expect("Failed to start merge");
        //
        // // open folder
        // // Tell user to compare with working versions
        // // resolve conflicts
        // // Once the user confirms they have resolved merge conflicts:
        //
        // block_on(merge.commence()).expect("Failed to commence merge");
        //
        // block_on(installed_package.commit("merge", user_meta)).expect("Failed to commit merge");
        //
        // let remote_latest = block_on(installed_package.push()).expect("Failed to push");
        //
        // assert_eq!(remote_latest, None, "Expected to certify remote latest");
    }
}
