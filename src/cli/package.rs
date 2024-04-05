use crate::quilt::s3::S3Uri;
use crate::{RemoteManifest, S3PackageUri};

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
        write!(
            f,
            "Manifest {} created",
            crate::quilt::storage::s3::S3Uri::from(&self.remote_manifest)
        )
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    match m.package(args).await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

pub async fn model(
    local_domain: &crate::LocalDomain,
    Input { target, uri }: Input,
) -> Result<Output, Error> {
    let uri: S3Uri = uri.parse()?;
    let target_uri: S3PackageUri = target.parse()?;
    let remote_manifest = local_domain.package_s3_prefix(&uri, target_uri).await?;
    Ok(Output { remote_manifest })
}
