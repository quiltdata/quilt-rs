use std::collections::BTreeMap;
use std::path::PathBuf;

use aws_sdk_s3::error::DisplayErrorContext;
use multihash::Multihash;
use tracing::log;

pub mod flow;
pub mod lineage;
pub mod manifest;
pub mod manifest_handle;
pub mod remote;
pub mod storage;
pub mod uri;

#[cfg(test)]
pub mod mocks;

use crate::paths;
use crate::quilt4::table::HEADER_ROW;
use crate::s3_utils;
use crate::Error;
use crate::Row4;
use crate::Table;

pub use flow::status::UpstreamDiscreteState;
pub use flow::status::UpstreamState;
pub use lineage::CommitState;
pub use lineage::DomainLineage;
pub use lineage::PackageLineage;
pub use lineage::PathState;
pub use manifest::ContentHash;
pub use manifest::Manifest;
pub use manifest::ManifestHeader;
pub use manifest::ManifestRow;
pub use manifest_handle::CachedManifest;
pub use manifest_handle::InstalledManifest;
pub use manifest_handle::ReadableManifest;
pub use manifest_handle::RemoteManifest;
pub use storage::fs;
pub use storage::s3;
pub use storage::Storage;
pub use uri::RevisionPointer;
pub use uri::S3PackageUri;

use flow::browse::browse_remote_manifest;
use flow::browse::cache_manifest;
use flow::certify_latest::certify_latest;
use flow::commit::commit_package;
use flow::install_package::install_package;
use flow::install_paths::install_paths;
use flow::pull::pull_package;
use flow::push::push_package;
use flow::reset_to_latest::reset_to_latest;
use flow::status::create_status;
use flow::status::refresh_latest_hash;
use flow::status::Change;
use flow::status::ChangeSet;
use flow::status::InstalledPackageStatus;
use flow::status::PackageFileFingerprint;
use flow::uninstall_package::uninstall_package;
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

    pub async fn browse_remote_manifest(
        &self,
        remote_manifest: &RemoteManifest,
    ) -> Result<Table, Error> {
        let mut storage = fs::LocalStorage::new();
        let remote = s3_utils::RemoteS3::new();
        browse_remote_manifest(&self.paths, &mut storage, &remote, remote_manifest).await
    }

    fn create_installed_package(&self, namespace: String) -> InstalledPackage {
        InstalledPackage {
            lineage: self.lineage.create_package_lineage(namespace.clone()),
            namespace: namespace.clone(),
            paths: self.paths.clone(),
        }
    }

    pub async fn install_package(
        &self,
        remote_manifest: &RemoteManifest,
    ) -> Result<InstalledPackage, Error> {
        // Read the lineage
        let lineage: DomainLineage = self.lineage.read().await?;
        let mut storage = fs::LocalStorage::new();
        let remote = s3_utils::RemoteS3::new();
        let lineage =
            install_package(lineage, &self.paths, &mut storage, &remote, remote_manifest).await?;
        self.lineage.write(&lineage).await?;

        Ok(self.create_installed_package(remote_manifest.namespace.clone()))
    }

    pub async fn uninstall_package(&self, namespace: impl AsRef<str>) -> Result<(), Error> {
        let storage = fs::LocalStorage::new();
        let lineage = self.lineage.read().await?;
        // FIXME: write lineage in the end?
        uninstall_package(lineage, &self.paths, &storage, namespace).await?;
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
        let mut storage = fs::LocalStorage::new();
        log::debug!("Source URI: {:?}, target URI: {:?}", uri, target_uri);
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
            let page = page.map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?;
            let page_contents_iter = page.contents.iter().flatten();

            for attrs in futures::future::try_join_all(page_contents_iter.map(|obj| {
                s3_utils::get_attrs_for_key(
                    client.clone(),
                    &uri.bucket,
                    obj.key.as_ref().expect("object key expected to be present"),
                )
            }))
            .await?
            {
                let name = attrs.key[prefix_len..].to_string();
                records.insert(
                    name.clone(),
                    Row4 {
                        name,
                        place: s3::make_s3_url(
                            &uri.bucket,
                            &attrs.key,
                            attrs.version_id.as_deref(),
                        )
                        .into(),
                        // XXX: can we use `as u64` safely here?
                        size: attrs.size,
                        hash: attrs.hash,
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
        let cache_path = cache_manifest(
            &self.paths,
            &mut storage,
            &table,
            &new_remote.bucket,
            &new_remote.hash,
        )
        .await?;
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

    pub async fn status(&self) -> Result<InstalledPackageStatus, Error> {
        let remote = s3_utils::RemoteS3::new();
        let mut storage = fs::LocalStorage::new();
        let lineage = self.lineage.read().await?;
        let lineage = refresh_latest_hash(lineage, &remote).await?;
        let (lineage, status) = create_status(
            lineage,
            &mut storage,
            &self.manifest().await?,
            self.working_folder(),
        )
        .await?;
        self.lineage.write(lineage).await?;
        Ok(status)
    }

    pub async fn install_paths(&self, paths: &Vec<String>) -> Result<(), Error> {
        if paths.is_empty() {
            return Ok(());
        }
        let mut storage = fs::LocalStorage::new();
        let lineage = self.lineage.read().await?;
        let lineage = install_paths(
            lineage,
            &self.manifest().await?,
            &self.paths,
            self.working_folder(),
            self.namespace.to_string(),
            &mut storage,
            paths,
        )
        .await?;
        self.lineage.write(lineage).await
    }

    pub async fn uninstall_paths(&self, paths: &Vec<String>) -> Result<(), Error> {
        let mut storage = fs::LocalStorage::new();
        let lineage = self.lineage.read().await?;
        let lineage = uninstall_paths(lineage, self.working_folder(), &mut storage, paths).await?;
        self.lineage.write(lineage).await
    }

    pub async fn revert_paths(&self, paths: &Vec<String>) -> Result<(), Error> {
        log::debug!("revert_paths: {paths:?}");
        unimplemented!()
    }

    pub async fn commit(
        &self,
        message: String,
        user_meta: Option<manifest::JsonObject>,
    ) -> Result<(), Error> {
        let mut storage = fs::LocalStorage::new();
        let lineage = self.lineage.read().await?;
        let lineage = commit_package(
            lineage,
            &self.manifest().await?,
            &self.paths,
            &mut storage,
            self.working_folder(),
            self.namespace.to_string(),
            message,
            user_meta,
        )
        .await?;
        self.lineage.write(lineage).await
    }

    pub async fn push(&self) -> Result<(), Error> {
        let mut storage = fs::LocalStorage::new();
        let lineage = self.lineage.read().await?;
        let lineage = push_package(
            lineage,
            &self.manifest().await?,
            &self.paths,
            &mut storage,
            self.namespace.to_string(),
        )
        .await?;
        self.lineage.write(lineage).await
    }

    pub async fn pull(&self) -> Result<(), Error> {
        let mut storage = fs::LocalStorage::new();
        let lineage = self.lineage.read().await?;
        let lineage = pull_package(
            lineage,
            &self.manifest().await?,
            &self.paths,
            &mut storage,
            self.working_folder(),
            self.namespace.to_string(),
        )
        .await?;
        self.lineage.write(lineage).await
    }

    pub async fn certify_latest(&self) -> Result<(), Error> {
        let lineage = self.lineage.read().await?;
        let lineage = certify_latest(lineage).await?;
        self.lineage.write(lineage).await
    }

    pub async fn reset_to_latest(&self) -> Result<(), Error> {
        let mut storage = fs::LocalStorage::new();
        let lineage = self.lineage.read().await?;
        let lineage = reset_to_latest(
            lineage,
            &self.manifest().await?,
            &self.paths,
            &mut storage,
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
    use tokio_test::assert_err;
    use tokio_test::block_on;

    use crate::quilt::flow::browse::cache_remote_manifest;
    use crate::quilt::manifest::MULTIHASH_SHA256;
    use crate::quilt::storage::mock_storage::MockStorage;
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
        let mut storage = MockStorage::default();

        // ## Pull the manifest

        let remote = s3_utils::RemoteS3::new();
        let remote_manifest = block_on(RemoteManifest::resolve(&remote, &test_uri))
            .expect("Failed to resolve manifest");

        let cached_manifest = block_on(cache_remote_manifest(
            &local_domain.paths,
            &mut storage,
            &remote,
            &remote_manifest,
        ))
        .expect("Failed to cache the manifest");

        let manifest =
            block_on(cached_manifest.read(&mut storage)).expect("Failed to parse the manifest");

        log::debug!("manifest: {manifest:?}");
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
        log::debug!("readme_path: {readme_path:?}");

        let old_readme =
            block_on(fs::read_to_string(&readme_path)).expect("Failed to read 'READ ME.md'");

        let timestamp = get_timestamp();
        log::debug!("timestamp: {timestamp:?}");
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
