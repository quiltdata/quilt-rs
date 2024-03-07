use tokio::sync;

use crate::cli::browse;
use crate::cli::install;
use crate::cli::list;

pub struct Model {
    local_domain: sync::Mutex<quilt_rs::LocalDomain>,
}

pub trait Commands {
    fn get_local_domain(&self) -> &sync::Mutex<quilt_rs::LocalDomain>;

    async fn browse_remote_manifest(
        &self,
        args: browse::CommandArgs,
    ) -> Result<quilt_rs::Table, String> {
        let local_domain = &self.get_local_domain().lock().await;
        browse::model(&local_domain, args).await
    }

    async fn package_install(
        &self,
        args: install::CommandArgs,
    ) -> Result<
        (
            quilt_rs::InstalledPackage,
            std::path::PathBuf,
            Option<Vec<std::path::PathBuf>>,
        ),
        String,
    > {
        let local_domain = &self.get_local_domain().lock().await;
        install::model(&local_domain, args).await
    }

    async fn get_installed_packages_list(&self) -> Result<Vec<quilt_rs::InstalledPackage>, String> {
        let local_domain = &self.get_local_domain().lock().await;
        list::model(&local_domain).await
    }
}

impl Commands for Model {
    fn get_local_domain(&self) -> &sync::Mutex<quilt_rs::LocalDomain> {
        &self.local_domain
    }
}

impl Model {
    pub fn new(local_domain: quilt_rs::LocalDomain) -> Self {
        Model {
            local_domain: sync::Mutex::new(local_domain),
        }
    }
}
