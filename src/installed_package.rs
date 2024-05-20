use std::collections::BTreeMap;
use std::path::PathBuf;

use tracing::log;

use crate::flow;
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
use crate::paths;
use crate::uri::ManifestUri;
use crate::uri::Namespace;
use crate::Res;

/// Similar to `LocalDomain` because it has access to the same lineage file and remote/storage
/// traits.
/// But it only manages one particular installed package.
/// It can be instantiated from `LocalDomain` by installing new or listing existing packages.
#[derive(Clone, Debug, PartialEq)]
pub struct InstalledPackage<S: Storage + Clone = LocalStorage, R: Remote + Clone = RemoteS3> {
    pub lineage: lineage::PackageLineageIo,
    pub paths: paths::DomainPaths,
    pub remote: R,
    pub storage: S,
    pub namespace: Namespace,
}

impl InstalledPackage {
    pub async fn manifest(&self) -> Res<Table> {
        let lineage = self.lineage.read(&self.storage).await?;
        let pathbuf = self
            .paths
            .installed_manifest(&self.namespace, lineage.current_hash());
        Table::read_from_path(&self.storage, &pathbuf).await
    }

    pub async fn lineage(&self) -> Res<lineage::PackageLineage> {
        self.lineage.read(&self.storage).await
    }

    pub fn working_folder(&self) -> PathBuf {
        self.paths.working_dir(&self.namespace)
    }

    pub async fn status(&self) -> Res<InstalledPackageStatus> {
        let lineage = self.lineage.read(&self.storage).await?;
        let lineage = flow::refresh_latest_hash(lineage, &self.remote).await?;
        let manifest = self.manifest().await?;
        let (lineage, status) =
            flow::status(lineage, &self.storage, &manifest, self.working_folder()).await?;
        self.lineage.write(&self.storage, lineage).await?;
        Ok(status)
    }

    pub async fn install_paths(&self, paths: &Vec<PathBuf>) -> Res<LineagePaths> {
        if paths.is_empty() {
            return Ok(BTreeMap::new());
        }
        let lineage = self.lineage.read(&self.storage).await?;
        let mut manifest = self.manifest().await?;
        let lineage = flow::install_paths(
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

    pub async fn uninstall_paths(&self, paths: &Vec<PathBuf>) -> Res<LineagePaths> {
        let lineage = self.lineage.read(&self.storage).await?;
        let lineage =
            flow::uninstall_paths(lineage, self.working_folder(), &self.storage, paths).await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.paths)
    }

    pub async fn revert_paths(&self, paths: &Vec<String>) -> Res {
        log::debug!("revert_paths: {paths:?}");
        unimplemented!()
    }

    pub async fn commit(
        &self,
        message: String,
        user_meta: Option<JsonObject>,
    ) -> Res<Option<CommitState>> {
        let lineage = self.lineage.read(&self.storage).await?;
        let mut manifest = self.manifest().await?;

        let (lineage, status) =
            flow::status(lineage, &self.storage, &manifest, self.working_folder()).await?;

        let lineage = flow::commit(
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

    pub async fn push(&self) -> Res<ManifestUri> {
        let lineage = self.lineage.read(&self.storage).await?;
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
        let lineage = self.lineage.read(&self.storage).await?;
        let mut manifest = self.manifest().await?;
        let (lineage, status) =
            flow::status(lineage, &self.storage, &manifest, self.working_folder()).await?;
        let lineage = flow::pull(
            lineage,
            &mut manifest,
            &self.paths,
            &self.storage,
            &self.remote,
            self.working_folder(),
            status,
            self.namespace.clone(),
        )
        .await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote)
    }

    pub async fn certify_latest(&self) -> Res<ManifestUri> {
        let lineage = self.lineage.read(&self.storage).await?;
        let latest_manifest_uri = lineage.remote.clone();
        let lineage = flow::certify_latest(lineage, &self.remote, latest_manifest_uri).await?;
        let lineage = self.lineage.write(&self.storage, lineage).await?;
        Ok(lineage.remote)
    }

    pub async fn reset_to_latest(&self) -> Res<ManifestUri> {
        let lineage = self.lineage.read(&self.storage).await?;
        let mut manifest = self.manifest().await?;
        let lineage = flow::reset_to_latest(
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
