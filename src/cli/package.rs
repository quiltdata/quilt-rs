use quilt_rs::uri::ManifestUri;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Debug)]
pub struct Input {
    pub message: Option<String>,
    pub target: String,
    pub uri: String,
    pub user_meta: Option<quilt_rs::manifest::JsonObject>,
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
    Input {
        message,
        target,
        uri,
        user_meta,
    }: Input,
) -> Result<Output, Error> {
    let uri = uri.parse()?;
    let target_uri = target.parse()?;
    let manifest_uri = local_domain
        .package_s3_prefix(&uri, target_uri, message, user_meta)
        .await?;
    Ok(Output { manifest_uri })
}
