use std::collections::BTreeMap;
use std::path::PathBuf;

use tracing::log;

use crate::flow;
use crate::io::remote::resolve_workflow;
use crate::io::remote::Remote;
use crate::io::remote::RemoteS3;
use crate::io::storage::LocalStorage;
use crate::io::storage::Storage;
use crate::lineage;
use crate::lineage::CommitState;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::LineagePaths;
use crate::manifest::JsonObject;
use crate::manifest::Table;
use crate::manifest::Workflow;
use crate::paths;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

/// Similar to `LocalDomain` because it has access to the same lineage file and remote/storage
/// traits.
/// But it only manages one particular installed package.
/// It can be instantiated from `LocalDomain` by installing new or listing existing packages.
#[derive(Debug)]
pub struct InstalledPackage<S: Storage = LocalStorage, R: Remote = RemoteS3> {
    pub lineage: lineage::PackageLineageIo,
    pub paths: paths::DomainPaths,
    pub remote: R,
    pub storage: S,
    pub namespace: Namespace,
}

impl std::fmt::Display for InstalledPackage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, r##"Installed package "{}""##, self.namespace,)
    }
}

impl InstalledPackage {
    pub async fn scaffold_paths(&self) -> Res {
        self.paths
            .scaffold_for_installing(&self.storage, &self.namespace)
            .await
    }

    pub async fn scaffold_paths_for_caching(&self, bucket: &str) -> Res {
        self.paths.scaffold_for_caching(&self.storage, bucket).await
    }

    pub async fn manifest(&self) -> Res<Table> {
        let (_, lineage) = self.lineage.read(&self.storage).await?;
        let pathbuf = self
            .paths
            .installed_manifest(&self.namespace, lineage.current_hash());
        Table::read_from_path(&self.storage, &pathbuf).await
    }

    pub async fn lineage(&self) -> Res<lineage::PackageLineage> {
        let (_, lineage) = self.lineage.read(&self.storage).await?;
        Ok(lineage)
    }

    pub async fn working_folder(&self) -> Res<PathBuf> {
        self.lineage.working_directory(&self.storage).await
    }

    pub async fn status(&self) -> Res<InstalledPackageStatus> {
        let (working_folder, lineage) = self.lineage.read(&self.storage).await?;
        let lineage = flow::refresh_latest_hash(lineage, &self.remote).await?;
        let manifest = self.manifest().await?;
        let (lineage, status) =
            flow::status(lineage, &self.storage, &manifest, working_folder).await?;
        self.lineage.write(&self.storage, lineage).await?;
        Ok(status)
    }

    pub async fn install_paths(&self, paths: &Vec<PathBuf>) -> Res<LineagePaths> {
        if paths.is_empty() {
            return Ok(BTreeMap::new());
        }

        self.scaffold_paths().await?;

        let (working_folder, lineage) = self.lineage.read(&self.storage).await?;

        self.scaffold_paths_for_caching(&lineage.remote.bucket)
            .await?;

        let mut manifest = self.manifest().await?;
        let lineage = flow::install_paths(
            lineage,
            &mut manifest,
            &self.paths,
            working_folder,
            self.namespace.clone(),
            &self.storage,
            &self.remote,
            paths,
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.paths)
    }

    pub async fn uninstall_paths(&self, paths: &Vec<PathBuf>) -> Res<LineagePaths> {
        let (working_folder, lineage) = self.lineage.read(&self.storage).await?;
        let lineage = flow::uninstall_paths(lineage, working_folder, &self.storage, paths).await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.paths)
    }

    pub async fn revert_paths(&self, paths: &Vec<String>) -> Res {
        log::debug!("revert_paths: {:?}", paths);
        unimplemented!()
    }

    pub async fn commit(
        &self,
        message: String,
        user_meta: Option<JsonObject>,
        workflow: Option<Workflow>,
    ) -> Res<CommitState> {
        self.scaffold_paths().await?;

        let (working_folder, lineage) = self.lineage.read(&self.storage).await?;
        let mut manifest = self.manifest().await?;

        let (lineage, status) =
            flow::status(lineage, &self.storage, &manifest, working_folder.clone()).await?;

        let lineage = flow::commit(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            working_folder,
            status,
            self.namespace.clone(),
            message,
            user_meta,
            workflow,
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        match lineage.commit {
            Some(commit) => Ok(commit),
            None => Err(Error::Commit("Nothing committed".to_string())),
        }
    }

    pub async fn push(&self) -> Res<ManifestUri> {
        self.scaffold_paths().await?;

        let (_, lineage) = self.lineage.read(&self.storage).await?;

        if lineage.commit.is_none() {
            return Err(Error::Push("No commits to push".to_string()));
        }

        self.scaffold_paths_for_caching(&lineage.remote.bucket)
            .await?;

        let manifest = self.manifest().await?;
        let lineage = flow::push(
            lineage,
            manifest,
            &self.paths,
            &self.storage,
            &self.remote,
            Some(self.namespace.clone()),
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote)
    }

    pub async fn pull(&self) -> Res<ManifestUri> {
        self.scaffold_paths().await?;

        let (working_folder, lineage) = self.lineage.read(&self.storage).await?;

        self.scaffold_paths_for_caching(&lineage.remote.bucket)
            .await?;

        let mut manifest = self.manifest().await?;
        let (lineage, status) =
            flow::status(lineage, &self.storage, &manifest, working_folder.clone()).await?;
        let lineage = flow::pull(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            &self.remote,
            working_folder,
            status,
            self.namespace.clone(),
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote)
    }

    pub async fn certify_latest(&self) -> Res<ManifestUri> {
        let (_, lineage) = self.lineage.read(&self.storage).await?;
        let latest_manifest_uri = lineage.remote.clone();
        let lineage = flow::certify_latest(lineage, &self.remote, latest_manifest_uri).await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote)
    }

    pub async fn reset_to_latest(&self) -> Res<ManifestUri> {
        self.scaffold_paths().await?;

        let (working_folder, lineage) = self.lineage.read(&self.storage).await?;

        self.scaffold_paths_for_caching(&lineage.remote.bucket)
            .await?;

        let mut manifest = self.manifest().await?;
        let lineage = flow::reset_to_latest(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            &self.remote,
            working_folder,
            self.namespace.clone(),
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote)
    }

    pub async fn resolve_workflow(&self, workflow_id: Option<String>) -> Res<Option<Workflow>> {
        let (_, lineage) = self.lineage.read(&self.storage).await?;
        let remote_uri = lineage.remote;
        let workflows_config_uri = S3Uri {
            key: ".quilt/workflows/config.yml".to_string(),
            ..S3Uri::from(&remote_uri)
        };
        resolve_workflow(
            &self.remote,
            &remote_uri.catalog,
            workflow_id,
            &workflows_config_uri,
        )
        .await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    use crate::lineage::DomainLineageIo;
    use crate::lineage::PackageLineageIo;

    #[tokio::test]
    async fn test_spamming_commit_writes() -> Res {
        let temp_dir = TempDir::new()?;
        let paths = paths::DomainPaths::new(temp_dir.path().to_path_buf());

        let storage = LocalStorage::new();
        let remote = RemoteS3::new(paths.clone(), storage.clone());
        let namespace: Namespace = ("test", "history").into();

        paths.scaffold_for_installing(&storage, &namespace).await?;
        // Initialize domain lineage file
        storage
            .write_file(
                &paths.lineage(),
                br#"{
                "packages": {
                    "test/history": {
                        "commit": null,
                        "remote": {
                            "bucket": "bucket",
                            "namespace": "test/history",
                            "hash": "abc123",
                            "catalog": "test.quilt.dev"
                        },
                        "base_hash": "abc123",
                        "latest_hash": "abc123",
                        "paths": {}
                    }},
                "working_directory": "/tmp/working_dir"
                }"#,
            )
            .await?;

        // Copy manifest to the expected path
        let reference_manifest = crate::fixtures::manifest::parquet_checksummed();
        let test_manifest = temp_dir
            .path()
            .to_path_buf()
            .join(".quilt/installed/test/history/abc123");
        storage.copy(reference_manifest?, test_manifest).await?;

        let domain_lineage_io = DomainLineageIo::new(paths.lineage());

        let package = InstalledPackage {
            lineage: PackageLineageIo::new(domain_lineage_io, namespace.clone()),
            paths,
            remote,
            storage,
            namespace,
        };

        // Make 10 commits with different content
        let mut expected_hashes = Vec::new();
        for i in 0..10 {
            let commit = package
                .commit(
                    format!("Commit new1 {}", i),
                    Some(
                        serde_json::json!({ "count": i })
                            .as_object()
                            .unwrap()
                            .clone(),
                    ),
                    None,
                )
                .await?;
            expected_hashes.insert(i, commit.hash);
        }

        // Remove last, cause it's the "current" hash, not a part of `prev_hashes`
        expected_hashes.pop();

        let commit_state = package.lineage().await?.commit.unwrap();

        assert_eq!(commit_state.prev_hashes.len(), 9);
        // let hashes_to_assert: Vec<String> = expected_hashes.into_iter().rev().collect();
        assert_eq!(
            commit_state.prev_hashes,
            expected_hashes.into_iter().rev().collect::<Vec<String>>()
        );

        Ok(())
    }
}
