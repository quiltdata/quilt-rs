use std::path::Path;
use std::path::PathBuf;

#[cfg(test)]
use tempfile::TempDir;

use crate::lineage::Home;

use crate::cli::browse;
use crate::cli::commit;
use crate::cli::create;
use crate::cli::install;
use crate::cli::list;
use crate::cli::login;
use crate::cli::pull;
use crate::cli::push;
use crate::cli::status;
use crate::cli::uninstall;
use crate::cli::Error;

pub struct Model {
    local_domain: crate::LocalDomain,
}

pub trait Commands {
    fn get_local_domain(&self) -> &crate::LocalDomain;

    async fn browse(&self, args: browse::Input) -> Result<browse::Output, Error> {
        let local_domain = self.get_local_domain();
        browse::model(local_domain, args).await
    }

    async fn create(&self, args: create::Input) -> Result<create::Output, Error> {
        let local_domain = self.get_local_domain();
        create::model(local_domain, args).await
    }

    async fn commit(&self, args: commit::Input) -> Result<commit::Output, Error> {
        let local_domain = self.get_local_domain();
        commit::model(local_domain, args).await
    }

    async fn install(&self, args: install::Input) -> Result<install::Output, Error> {
        let local_domain = self.get_local_domain();
        install::model(local_domain, args).await
    }

    async fn list(&self) -> Result<list::Output, Error> {
        let local_domain = self.get_local_domain();
        list::model(local_domain).await
    }

    async fn login(&self, args: login::Input) -> Result<login::Output, Error> {
        let local_domain = self.get_local_domain();
        login::model(local_domain, args).await
    }

    async fn pull(&self, args: pull::Input) -> Result<pull::Output, Error> {
        let local_domain = self.get_local_domain();
        pull::model(local_domain, args).await
    }

    async fn push(&self, args: push::Input) -> Result<push::Output, Error> {
        let local_domain = self.get_local_domain();
        push::model(local_domain, args).await
    }

    async fn status(&self, args: status::Input) -> Result<status::Output, Error> {
        let local_domain = self.get_local_domain();
        status::model(local_domain, args).await
    }

    async fn uninstall(&self, args: uninstall::Input) -> Result<uninstall::Output, Error> {
        let local_domain = self.get_local_domain();
        uninstall::model(local_domain, args).await
    }
}

impl Commands for Model {
    fn get_local_domain(&self) -> &crate::LocalDomain {
        &self.local_domain
    }
}

impl Model {
    fn new(local_domain: crate::LocalDomain) -> Self {
        Model { local_domain }
    }

    pub async fn get_home(&self) -> Result<Home, Error> {
        Ok(self.local_domain.get_home().await?)
    }

    pub async fn set_home(&self, dir: impl AsRef<Path>) -> Result<Home, Error> {
        Ok(self.local_domain.set_home(dir).await?)
    }

    #[cfg(test)]
    pub fn from_temp_dir() -> Result<(Self, TempDir), Error> {
        let temp_dir = TempDir::new()?;
        Ok((Model::from(&temp_dir), temp_dir))
    }
}

impl From<PathBuf> for Model {
    fn from(root: PathBuf) -> Self {
        let local_domain = crate::LocalDomain::new(root);
        Model::new(local_domain)
    }
}

#[cfg(test)]
impl From<&TempDir> for Model {
    fn from(temp_dir: &TempDir) -> Self {
        Model::from(temp_dir.path().to_path_buf())
    }
}

#[cfg(test)]
pub async fn install_package_into_temp_dir(
    uri_str: &str,
) -> Result<(Model, crate::InstalledPackage, TempDir), Error> {
    let (model, temp_dir) = Model::from_temp_dir()?;

    model.set_home(temp_dir.path()).await?;

    let output = model
        .install(install::Input {
            namespace: None,
            paths: None,
            uri: uri_str.to_string(),
        })
        .await?;

    let installed_package = output.get_installed_package();

    tracing::log::debug!(
        "Installed package manifest: {:?}",
        installed_package.manifest().await?
    );

    // We must return `temp_dir` because otherwise it will be dropped and removed
    Ok((model, installed_package, temp_dir))
}

#[cfg(test)]
pub async fn create_model_in_temp_dir() -> Result<(Model, TempDir), Error> {
    let (model, temp_dir) = Model::from_temp_dir()?;
    model.set_home(temp_dir.path()).await?;
    Ok((model, temp_dir))
}
