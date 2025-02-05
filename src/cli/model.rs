use std::path::PathBuf;
use tempfile::TempDir;
use tokio::sync;

use crate::cli::benchmark;
use crate::cli::browse;
use crate::cli::commit;
use crate::cli::install;
use crate::cli::list;
use crate::cli::package;
use crate::cli::pull;
use crate::cli::push;
use crate::cli::status;
use crate::cli::uninstall;
use crate::cli::Error;

pub struct Model {
    local_domain: sync::Mutex<quilt_rs::LocalDomain>,
}

pub trait Commands {
    fn get_local_domain(&self) -> &sync::Mutex<quilt_rs::LocalDomain>;

    async fn browse(&self, args: browse::Input) -> Result<browse::Output, Error> {
        let local_domain = &self.get_local_domain().lock().await;
        browse::model(local_domain, args).await
    }

    async fn commit(&self, args: commit::Input) -> Result<commit::Output, Error> {
        let local_domain = &self.get_local_domain().lock().await;
        commit::model(local_domain, args).await
    }

    async fn install(&self, args: install::Input) -> Result<install::Output, Error> {
        let local_domain = &self.get_local_domain().lock().await;
        install::model(local_domain, args).await
    }

    async fn list(&self) -> Result<list::Output, Error> {
        let local_domain = &self.get_local_domain().lock().await;
        list::model(local_domain).await
    }

    async fn package(&self, args: package::Input) -> Result<package::Output, Error> {
        let local_domain = &self.get_local_domain().lock().await;
        package::model(local_domain, args).await
    }

    async fn pull(&self, args: pull::Input) -> Result<pull::Output, Error> {
        let local_domain = &self.get_local_domain().lock().await;
        pull::model(local_domain, args).await
    }

    async fn push(&self, args: push::Input) -> Result<push::Output, Error> {
        let local_domain = &self.get_local_domain().lock().await;
        push::model(local_domain, args).await
    }

    async fn status(&self, args: status::Input) -> Result<status::Output, Error> {
        let local_domain = &self.get_local_domain().lock().await;
        status::model(local_domain, args).await
    }

    async fn benchmark(&self, args: benchmark::Input) -> Result<benchmark::Output, Error> {
        let local_domain = &self.get_local_domain().lock().await;
        benchmark::model(local_domain, args).await
    }

    async fn uninstall(&self, args: uninstall::Input) -> Result<uninstall::Output, Error> {
        let local_domain = &self.get_local_domain().lock().await;
        uninstall::model(local_domain, args).await
    }
}

impl Commands for Model {
    fn get_local_domain(&self) -> &sync::Mutex<quilt_rs::LocalDomain> {
        &self.local_domain
    }
}

impl Model {
    fn new(local_domain: quilt_rs::LocalDomain) -> Self {
        Model {
            local_domain: sync::Mutex::new(local_domain),
        }
    }
    pub fn from_temp_dir() -> Result<(Self, TempDir), Error> {
        let temp_dir = TempDir::new()?;
        Ok((Model::from(&temp_dir), temp_dir))
    }
}

impl From<PathBuf> for Model {
    fn from(root: PathBuf) -> Self {
        let local_domain = quilt_rs::LocalDomain::new(root);
        Model::new(local_domain)
    }
}

impl From<&TempDir> for Model {
    fn from(temp_dir: &TempDir) -> Self {
        Model::from(temp_dir.path().to_path_buf())
    }
}
