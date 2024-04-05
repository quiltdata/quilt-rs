use std::path::PathBuf;
use temp_dir::TempDir;
use tokio::sync;

use crate::cli::browse;
use crate::cli::install;
use crate::cli::list;
use crate::cli::package;
use crate::cli::uninstall;
use crate::cli::Error;

pub struct Model {
    local_domain: sync::Mutex<crate::LocalDomain>,
}

pub trait Commands {
    fn get_local_domain(&self) -> &sync::Mutex<crate::LocalDomain>;

    #[allow(async_fn_in_trait)]
    async fn browse(&self, args: browse::Input) -> Result<browse::Output, Error> {
        let local_domain = &self.get_local_domain().lock().await;
        browse::model(local_domain, args).await
    }

    #[allow(async_fn_in_trait)]
    async fn install(&self, args: install::Input) -> Result<install::Output, Error> {
        let local_domain = &self.get_local_domain().lock().await;
        install::model(local_domain, args).await
    }

    #[allow(async_fn_in_trait)]
    async fn list(&self) -> Result<list::Output, Error> {
        let local_domain = &self.get_local_domain().lock().await;
        list::model(local_domain).await
    }

    #[allow(async_fn_in_trait)]
    async fn package(&self, args: package::Input) -> Result<package::Output, Error> {
        let local_domain = &self.get_local_domain().lock().await;
        package::model(local_domain, args).await
    }

    #[allow(async_fn_in_trait)]
    async fn uninstall(&self, args: uninstall::Input) -> Result<uninstall::Output, Error> {
        let local_domain = &self.get_local_domain().lock().await;
        uninstall::model(local_domain, args).await
    }
}

impl Commands for Model {
    fn get_local_domain(&self) -> &sync::Mutex<crate::LocalDomain> {
        &self.local_domain
    }
}

impl Model {
    fn new(local_domain: crate::LocalDomain) -> Self {
        Model {
            local_domain: sync::Mutex::new(local_domain),
        }
    }
    pub fn from_temp_dir() -> Result<(Self, TempDir), Error> {
        let temp_dir =
            TempDir::with_prefix("quilt-rs").map_err(|err| Error::TempDir(err.to_string()))?;
        Ok((Model::from(temp_dir.path().to_path_buf()), temp_dir))
    }
}

impl From<PathBuf> for Model {
    fn from(root: PathBuf) -> Self {
        let local_domain = crate::LocalDomain::new(root);
        Model::new(local_domain)
    }
}
