use std::collections::BTreeMap;
use std::path::PathBuf;

use aws_sdk_s3::error::DisplayErrorContext;
use tracing::log;

use crate::flow::browse::browse_remote_manifest;
use crate::flow::certify_latest::certify_latest;
use crate::flow::commit::commit_package;
use crate::flow::install_package::install_package;
use crate::flow::install_paths::install_paths;
use crate::flow::pull::pull_package;
use crate::flow::push::push_package;
use crate::flow::reset_to_latest::reset_to_latest;
use crate::flow::status::create_status;
use crate::flow::status::refresh_latest_hash;
use crate::flow::uninstall_package::uninstall_package;
use crate::flow::uninstall_paths::uninstall_paths;
use crate::io::manifest::tag_latest;
use crate::io::manifest::tag_timestamp;
use crate::io::manifest::upload_manifest;
use crate::io::remote::s3::RemoteS3;
use crate::io::remote::utils::get_attrs_for_key;
use crate::io::remote::utils::get_client_for_bucket;
use crate::io::remote::Remote;
use crate::io::storage::fs;
use crate::io::storage::Storage;
use crate::lineage;
use crate::lineage::CommitState;
use crate::lineage::DomainLineage;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::LineagePaths;
use crate::manifest::JsonObject;
use crate::manifest::Row;
use crate::manifest::Table;
use crate::paths;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
use crate::uri::S3PackageUri;
use crate::uri::S3Uri;
use crate::Error;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalDomain<S: Storage = fs::LocalStorage, R: Remote = RemoteS3> {
    paths: paths::DomainPaths,
    lineage: lineage::DomainLineageIo,
    storage: S,
    remote: R,
}

impl LocalDomain {
    pub fn new(root_dir: PathBuf) -> Self {
        let paths = paths::DomainPaths::new(root_dir.clone());
        let lineage = lineage::DomainLineageIo::new(paths.lineage());
        let storage = fs::LocalStorage::new();
        let remote = RemoteS3::new();
        Self {
            lineage,
            paths,
            remote,
            storage,
        }
    }

    pub async fn browse_remote_manifest(&self, uri: &S3PackageUri) -> Result<Table, Error> {
        browse_remote_manifest(&self.paths, &self.storage, &self.remote, uri).await
    }

    pub fn create_installed_package(&self, namespace: Namespace) -> InstalledPackage {
        InstalledPackage {
            lineage: self.lineage.create_package_lineage(namespace.clone()),
            namespace: namespace.clone(),
            paths: self.paths.clone(),
            remote: self.remote.clone(),
            storage: self.storage.clone(),
        }
    }

    pub async fn install_package(
        &self,
        manifest_uri: &ManifestUri,
    ) -> Result<InstalledPackage, Error> {
        // Read the lineage
        let lineage: DomainLineage = self.lineage.read(&self.storage).await?;
        let lineage = install_package(
            lineage,
            &self.paths,
            &self.storage,
            &self.remote,
            manifest_uri,
        )
        .await?;
        let _fixme = self.lineage.write(&self.storage, lineage).await?;

        Ok(self.create_installed_package(manifest_uri.namespace.clone()))
    }

    pub async fn uninstall_package(&self, namespace: Namespace) -> Result<(), Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let lineage = uninstall_package(lineage, &self.paths, &self.storage, namespace).await?;
        let _fixme = self.lineage.write(&self.storage, lineage).await?;
        Ok(())
    }

    pub async fn list_installed_packages(&self) -> Result<Vec<InstalledPackage>, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let mut namespaces: Vec<Namespace> = lineage.packages.into_keys().collect();
        namespaces.sort();
        let packages = namespaces
            .into_iter()
            .map(|namespace| InstalledPackage {
                lineage: self.lineage.create_package_lineage(namespace.clone()),
                namespace,
                paths: self.paths.clone(),
                remote: self.remote.clone(),
                storage: self.storage.clone(),
            })
            .collect();
        Ok(packages)
    }

    pub async fn get_installed_package(
        &self,
        namespace: &Namespace,
    ) -> Result<Option<InstalledPackage>, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        if lineage.packages.contains_key(namespace) {
            Ok(Some(InstalledPackage {
                lineage: self.lineage.create_package_lineage(namespace.clone()),
                namespace: namespace.clone(),
                paths: self.paths.clone(),
                remote: self.remote.clone(),
                storage: self.storage.clone(),
            }))
        } else {
            Ok(None)
        }
    }

    pub async fn package_s3_prefix(
        &self,
        uri: &S3Uri,
        target_uri: S3PackageUri,
    ) -> Result<ManifestUri, Error> {
        log::debug!("Source URI: {:?}, target URI: {:?}", uri, target_uri);
        // TODO: make get_object_attributes() calls concurrently across list_objects() pages
        // TODO: increase concurrency, to do that we need to figure out how to deal
        //       with fd limits on Mac by default it's 256
        // TODO: s3 uri key ends with / and has no version
        // FIXME: filter or fail on keys with `.` or `..` in path segments as quilt3 do
        let client = get_client_for_bucket(&uri.bucket).await?;

        // XXX: we need real API to build manifests
        let header = Row::default();
        let mut records: BTreeMap<PathBuf, Row> = BTreeMap::new();

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
                get_attrs_for_key(
                    client.clone(),
                    &uri.bucket,
                    obj.key.as_ref().expect("object key expected to be present"),
                )
            }))
            .await?
            {
                let name = PathBuf::from(attrs.key[prefix_len..].to_string());
                let record_url = S3Uri {
                    bucket: uri.bucket.clone(),
                    key: attrs.key,
                    version: attrs.version_id,
                };
                records.insert(
                    name.clone(),
                    Row {
                        name,
                        place: record_url.to_string(),
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
        let manifest_uri = upload_manifest(
            &self.storage,
            &self.remote,
            &self.paths,
            target_uri.bucket,
            target_uri.namespace,
            table,
        )
        .await?;
        tag_timestamp(&self.remote, &manifest_uri, chrono::Utc::now()).await?;
        tag_latest(&self.remote, &manifest_uri).await?;

        Ok(manifest_uri)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct InstalledPackage<S: Storage + Clone = fs::LocalStorage, R: Remote + Clone = RemoteS3> {
    lineage: lineage::PackageLineageIo,
    paths: paths::DomainPaths,
    remote: R,
    storage: S,
    pub namespace: Namespace,
}

impl InstalledPackage {
    pub async fn manifest(&self) -> Result<Table, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let pathbuf = self
            .paths
            .installed_manifest(&self.namespace, lineage.current_hash());
        Table::read_from_path(&self.storage, &pathbuf).await
    }

    pub async fn lineage(&self) -> Result<lineage::PackageLineage, Error> {
        self.lineage.read(&self.storage).await
    }

    pub fn working_folder(&self) -> PathBuf {
        self.paths.working_dir(&self.namespace)
    }

    pub async fn status(&self) -> Result<InstalledPackageStatus, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        // TODO: lineage.refresh_latest?
        let lineage = refresh_latest_hash(lineage, &self.remote).await?;
        let manifest = self.manifest().await?;
        let (lineage, status) =
            create_status(lineage, &self.storage, &manifest, self.working_folder()).await?;
        self.lineage.write(&self.storage, lineage).await?;
        Ok(status)
    }

    pub async fn install_paths(&self, paths: &Vec<PathBuf>) -> Result<LineagePaths, Error> {
        if paths.is_empty() {
            return Ok(BTreeMap::new());
        }
        let lineage = self.lineage.read(&self.storage).await?;
        let mut manifest = self.manifest().await?;
        let lineage = install_paths(
            lineage,
            &mut manifest,
            &self.paths,
            self.working_folder(),
            self.namespace.clone(),
            &self.storage,
            &self.remote,
            paths,
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.paths)
    }

    pub async fn uninstall_paths(&self, paths: &Vec<PathBuf>) -> Result<LineagePaths, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let lineage = uninstall_paths(lineage, self.working_folder(), &self.storage, paths).await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.paths)
    }

    pub async fn revert_paths(&self, paths: &Vec<String>) -> Result<(), Error> {
        log::debug!("revert_paths: {paths:?}");
        unimplemented!()
    }

    pub async fn commit(
        &self,
        message: String,
        user_meta: Option<JsonObject>,
    ) -> Result<Option<CommitState>, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let mut manifest = self.manifest().await?;

        let (lineage, status) =
            create_status(lineage, &self.storage, &manifest, self.working_folder()).await?;

        let lineage = commit_package(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            self.working_folder(),
            status,
            self.namespace.clone(),
            message,
            user_meta,
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.commit)
    }

    pub async fn push(&self) -> Result<ManifestUri, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let mut manifest = self.manifest().await?;
        let lineage = push_package(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            &self.remote,
            self.namespace.clone(),
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote)
    }

    pub async fn pull(&self) -> Result<ManifestUri, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let mut manifest = self.manifest().await?;
        let (lineage, status) =
            create_status(lineage, &self.storage, &manifest, self.working_folder()).await?;
        let lineage = pull_package(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            self.working_folder(),
            status,
            self.namespace.clone(),
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote)
    }

    pub async fn certify_latest(&self) -> Result<ManifestUri, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let latest_manifest_uri = lineage.remote.clone();
        let lineage = certify_latest(lineage, &self.remote, latest_manifest_uri).await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote)
    }

    pub async fn reset_to_latest(&self) -> Result<ManifestUri, Error> {
        let lineage = self.lineage.read(&self.storage).await?;
        let mut manifest = self.manifest().await?;
        let lineage = reset_to_latest(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            &self.remote,
            self.working_folder(),
            self.namespace.clone(),
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use multihash::Multihash;
    use temp_testdir::TempDir;
    use tokio::io::AsyncWriteExt;
    use tokio_test::assert_err;
    use tokio_test::block_on;

    use crate::checksum::calculate_sha256_checksum;
    use crate::checksum::MULTIHASH_SHA256;
    use crate::flow::browse::cache_remote_manifest;
    use crate::io::manifest::resolve_manifest_uri;
    use crate::lineage::Change;
    use crate::lineage::ChangeSet;
    use crate::lineage::DiscreteChange;
    use crate::lineage::PackageFileFingerprint;
    use crate::lineage::UpstreamState;
    use crate::mocks;
    use crate::uri::RevisionPointer;

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
                namespace: ("spec", "quiltcore").into(),
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
        let storage = mocks::storage::MockStorage::default();

        // ## Pull the manifest

        let remote = RemoteS3::new();
        let manifest_uri =
            block_on(resolve_manifest_uri(&remote, &test_uri)).expect("Failed to resolve manifest");

        let manifest = block_on(cache_remote_manifest(
            &local_domain.paths,
            &storage,
            &remote,
            &manifest_uri.clone().into(),
        ))
        .expect("Failed to cache the manifest");

        log::debug!("manifest: {manifest:?}");
        // TODO: assert manifest has the expected contents

        // ## Install the files
        //
        // installed_package knows its domain and install folder
        // downloads manifest for latest revision into an editable working copy
        // creates install folder and Lineages it to remote_package_uri

        let paths = vec![test_uri.path.unwrap()];
        let installed_package = block_on(local_domain.install_package(&manifest_uri))
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
            block_on(tokio::fs::read_to_string(&readme_path)).expect("Failed to read 'READ ME.md'");

        let timestamp = get_timestamp();
        log::debug!("timestamp: {timestamp:?}");
        let mut readme_file =
            block_on(tokio::fs::File::create(readme_path)).expect("Failed to create file");
        block_on(readme_file.write_all(timestamp.as_bytes()))
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
                    state: DiscreteChange::Modified,
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
