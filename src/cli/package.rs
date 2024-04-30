use quilt_rs::ManifestUri;

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
    manifest_uri: ManifestUri,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Manifest {} created",
            quilt_rs::uri::S3Uri::from(&self.manifest_uri)
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
    local_domain: &quilt_rs::LocalDomain,
    Input { target, uri }: Input,
) -> Result<Output, Error> {
    let uri = uri.parse()?;
    let target_uri = target.parse()?;
    let manifest_uri = local_domain.package_s3_prefix(&uri, target_uri).await?;
    Ok(Output { manifest_uri })
}
