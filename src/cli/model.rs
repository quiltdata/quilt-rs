use std::path::Path;
use std::path::PathBuf;

use tempfile::TempDir;

use crate::cli::benchmark;
use crate::cli::browse;
use crate::cli::commit;
use crate::cli::install;
use crate::cli::list;
use crate::cli::login;
use crate::cli::package;
use crate::cli::pull;
use crate::cli::push;
use crate::cli::status;
use crate::cli::uninstall;
use crate::cli::Error;

pub struct Model {
    local_domain: quilt_rs::LocalDomain,
}

pub trait Commands {
    fn get_local_domain(&self) -> &quilt_rs::LocalDomain;

    async fn browse(&self, args: browse::Input) -> Result<browse::Output, Error> {
        let local_domain = self.get_local_domain();
        browse::model(local_domain, args).await
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

    async fn package(&self, args: package::Input) -> Result<package::Output, Error> {
        let local_domain = self.get_local_domain();
        package::model(local_domain, args).await
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

    async fn benchmark(&self, args: benchmark::Input) -> Result<benchmark::Output, Error> {
        let local_domain = self.get_local_domain();
        benchmark::model(local_domain, args).await
    }

    async fn uninstall(&self, args: uninstall::Input) -> Result<uninstall::Output, Error> {
        let local_domain = self.get_local_domain();
        uninstall::model(local_domain, args).await
    }
}

impl Commands for Model {
    fn get_local_domain(&self) -> &quilt_rs::LocalDomain {
        &self.local_domain
    }
}

impl Model {
    fn new(local_domain: quilt_rs::LocalDomain) -> Self {
        Model { local_domain }
    }

    pub async fn has_working_directory(&self) -> Result<bool, Error> {
        let working_dir = self.local_domain.working_directory().await?;
        Ok(working_dir.is_some())
    }

    pub async fn set_working_directory(&self, dir: impl AsRef<Path>) -> Result<(), Error> {
        self.local_domain.set_working_directory(dir).await?;
        Ok(())
    }

    #[cfg(test)]
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
    #[must_use]
    fn from(temp_dir: &TempDir) -> Self {
        Model::from(temp_dir.path().to_path_buf())
    }
}

#[cfg(test)]
pub async fn install_package_into_temp_dir(
    uri_str: &str,
) -> Result<(Model, quilt_rs::InstalledPackage, TempDir), Error> {
    let (model, temp_dir) = Model::from_temp_dir()?;

    model.set_working_directory(temp_dir.path()).await?;

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
    model.set_working_directory(temp_dir.path()).await?;
    Ok((model, temp_dir))
}
