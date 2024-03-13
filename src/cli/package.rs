use crate::cli::model::Commands;
use crate::cli::output::Std;

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
    Input {
        target: target_string,
        uri: uri_string,
    }: Input,
) -> Result<Output, String> {
    let uri = quilt_rs::quilt::storage::s3::S3Uri::try_from(uri_string.as_str())?;
    let target_uri = quilt_rs::S3PackageURI::try_from(target_string.as_str())?;
    let (manifest, paths) = local_domain.package_s3_prefix(&uri, target_uri).await?;
    Ok(Output { manifest, paths })
}
