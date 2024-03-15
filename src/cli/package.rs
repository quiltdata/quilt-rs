use quilt_rs::{quilt::storage::s3::S3URI, S3PackageURI};

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
    manifest: quilt_rs::Table,
    paths: Vec<String>,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut output = vec![format!("Manifest {:?} created", self.manifest,)];
        for path in &self.paths {
            output.push(format!("Path: {:?}", path));
        }
        write!(f, "{}", output.join("\n"))
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
    let uri: S3URI = uri.parse()?;
    let target_uri: S3PackageURI = target.parse()?;
    let (manifest, paths) = local_domain.package_s3_prefix(&uri, target_uri).await?;
    Ok(Output { manifest, paths })
}
