use quilt_rs::RemoteManifest;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Debug)]
pub struct Input {
    pub uri: String,
    pub target: String,
}

#[derive(Debug)]
pub struct Output {
    remote_manifest: RemoteManifest,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Manifest {} created", self.remote_manifest.as_s3_uri())
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    match m.package(args).await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input {
        target: target_string,
        uri: uri_string,
    }: Input,
) -> Result<Output, Error> {
    let uri = quilt_rs::quilt::storage::s3::S3Uri::try_from(uri_string.as_str())?;
    let target_uri = quilt_rs::S3PackageURI::try_from(target_string.as_str())?;
    let remote_manifest = local_domain.package_s3_prefix(&uri, target_uri).await?;
    Ok(Output { remote_manifest })
}
