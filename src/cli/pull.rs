use quilt_rs::quilt::RemoteManifest;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Debug)]
pub struct Input {
    pub namespace: String,
}

#[derive(Debug)]
pub struct Output {
    pub hash: String,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, r##"Revision "{}" pulled"##, self.hash)
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    match m.pull(args).await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

async fn pull_package(
    local_domain: &quilt_rs::LocalDomain,
    namespace: String,
) -> Result<RemoteManifest, Error> {
    let installed_package = local_domain.get_installed_package(&namespace).await?;

    match installed_package {
        Some(installed_package) => Ok(installed_package.pull().await?),
        None => Err(Error::NamespaceNotFound(namespace.to_string())),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { namespace }: Input,
) -> Result<Output, Error> {
    let remote_manifest = pull_package(local_domain, namespace).await?;
    Ok(Output {
        hash: remote_manifest.hash,
    })
}
