use std::{collections::BTreeMap, path::PathBuf};

use multihash::Multihash;
use parquet::data_type::AsBytes;
use tokio::fs::remove_dir_all;

pub mod flow;
pub mod lineage;
pub mod manifest;
pub mod manifest_handle;
pub mod storage;
pub mod uri;

use crate::{paths, quilt4::table::HEADER_ROW, s3_utils, Error, Row4, Table, UPath};

use self::manifest::MULTIHASH_SHA256_CHUNKED;
pub use self::{
    flow::status::{
        create_status, refresh_latest_hash, Change, ChangeSet, InstalledPackageStatus,
        PackageFileFingerprint, UpstreamDiscreteState, UpstreamState,
    },
    lineage::{CommitState, DomainLineage, PackageLineage, PathState},
    manifest::{ContentHash, Manifest, ManifestHeader, ManifestRow},
    manifest_handle::{CachedManifest, InstalledManifest, ReadableManifest, RemoteManifest},
    storage::{fs, s3},
    uri::{RevisionPointer, S3PackageUri},
};
use flow::browse::{browse_remote_manifest, cache_manifest};
use flow::commit::commit_package;
use flow::install_package::install_package;
use flow::install_paths::install_paths;
use flow::pull::pull_package;
use flow::push::push_package;
use flow::reset_to_latest::reset_to_latest;
use flow::uninstall_paths::uninstall_paths;

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
        let lineage = install_package(lineage, &self.paths, remote).await?;
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
                let hash = Multihash::wrap(MULTIHASH_SHA256_CHUNKED, s3_checksum.as_bytes())?;
                records.insert(
                    name.into(),
                    Row4 {
                        name: name.into(),
                        place: s3::make_s3_url(&uri.bucket, key, attrs.version_id.as_deref())
                            .into(),
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

    pub async fn status(&self) -> Result<InstalledPackageStatus, Error> {
        let lineage = self.lineage.read().await?;
        let lineage = refresh_latest_hash(lineage).await?;
        let (lineage, status) =
            create_status(lineage, &self.manifest().await?, self.working_folder()).await?;
        self.lineage.write(lineage).await?;
        Ok(status)
    }

    pub async fn install_paths(&self, paths: &Vec<String>) -> Result<(), Error> {
        if paths.is_empty() {
            return Ok(());
        }
        let file_ops = fs::RelativeFileOps::new(self.working_folder());
        let lineage = self.lineage.read().await?;
        let lineage = install_paths(
            lineage,
            &self.manifest().await?,
            &self.paths,
            self.working_folder(),
            self.namespace.to_string(),
            file_ops,
            paths,
        )
        .await?;
        self.lineage.write(lineage).await
    }

    pub async fn uninstall_paths(&self, paths: &Vec<String>) -> Result<(), Error> {
        let file_ops = fs::RelativeFileOps::new(self.working_folder());
        let lineage = self.lineage.read().await?;
        let lineage = uninstall_paths(lineage, self.working_folder(), file_ops, paths).await?;
        self.lineage.write(lineage).await
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
        let lineage = self.lineage.read().await?;
        let lineage = commit_package(
            lineage,
            &self.manifest().await?,
            &self.paths,
            self.working_folder(),
            self.namespace.to_string(),
            message,
            user_meta,
        )
        .await?;
        self.lineage.write(lineage).await
    }

    pub async fn push(&self) -> Result<(), Error> {
        let lineage = self.lineage.read().await?;
        let lineage = push_package(
            lineage,
            &self.manifest().await?,
            &self.paths,
            self.namespace.to_string(),
        )
        .await?;
        self.lineage.write(lineage).await
    }

    pub async fn pull(&self) -> Result<(), Error> {
        let lineage = self.lineage.read().await?;
        let lineage = pull_package(
            lineage,
            &self.manifest().await?,
            &self.paths,
            self.working_folder(),
            self.namespace.to_string(),
        )
        .await?;
        self.lineage.write(lineage).await
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
        let lineage = reset_to_latest(
            lineage,
            &self.manifest().await?,
            &self.paths,
            self.working_folder(),
            self.namespace.to_string(),
        )
        .await?;
        self.lineage.write(lineage).await
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

    use crate::quilt::flow::browse::cache_remote_manifest;
    use crate::quilt::manifest::MULTIHASH_SHA256;
    use crate::quilt4::checksum::calculate_sha256_checksum;

    fn get_timestamp() -> String {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs()
            .to_string()
    }

    #[test]
    #[ignore]
    fn flow() -> Result<(), Error> {
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
                            block_on(calculate_sha256_checksum(timestamp.as_bytes()))?.as_ref(),
                        )?,
                    }),
                    previous: Some(PackageFileFingerprint {
                        size: old_readme.len() as u64,
                        hash: Multihash::wrap(
                            MULTIHASH_SHA256,
                            block_on(calculate_sha256_checksum(old_readme.as_bytes()))?.as_ref(),
                        )?,
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
        Ok(())
    }
}
