use quilt_rs::uri::ManifestUri;
use quilt_rs::uri::Namespace;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Debug)]
pub struct Input {
    pub namespace: Namespace,
}

#[derive(Debug)]
pub struct Output {
    pub hash: String,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, r##"New revision "{}" pushed"##, self.hash)
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    match m.push(args).await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

async fn push_package(
    local_domain: &quilt_rs::LocalDomain,
    namespace: Namespace,
) -> Result<ManifestUri, Error> {
    match local_domain.get_installed_package(&namespace).await? {
        Some(installed_package) => Ok(installed_package.push().await?),
        None => Err(Error::NamespaceNotFound(namespace)),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { namespace }: Input,
) -> Result<Output, Error> {
    let manifest_uri = push_package(local_domain, namespace).await?;
    Ok(Output {
        hash: manifest_uri.hash,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::model::Model;
    use quilt_rs::uri::{ManifestUri, S3PackageUri};
    use quilt_rs::{InstalledPackage, LocalDomain};
    use std::path::PathBuf;
    use temp_testdir::TempDir;

    async fn install_package(
        uri_str: &str,
        root_dir: Option<PathBuf>,
    ) -> Result<(TempDir, InstalledPackage, LocalDomain), Error> {
        let uri = S3PackageUri::try_from(uri_str)?;

        let temp_dir = TempDir::default();
        let local_path = root_dir.unwrap_or_else(|| PathBuf::from(temp_dir.as_ref()));
        let local_domain = LocalDomain::new(local_path);

        let manifest_uri = ManifestUri::try_from(uri)?;
        let installed_package = local_domain.install_package(&manifest_uri).await?;

        Ok((temp_dir, installed_package, local_domain))
    }

    #[tokio::test]
    async fn test_namespace_not_found() -> Result<(), Error> {
        let (test_model, _temp_dir) = Model::from_temp_dir()?;

        if let Std::Err(error_str) = command(
            test_model,
            Input {
                namespace: ("in", "valid").into(),
            },
        )
        .await
        {
            assert_eq!(error_str.to_string(), "Package in/valid not found");
        } else {
            return Err(Error::Test("Expected package not found error".to_string()));
        }

        Ok(())
    }

    #[tokio::test]
    async fn test_no_commit() -> Result<(), Error> {
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore@44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c";
        let (_temp_dir, _installed_package, local_domain) = install_package(uri, None).await?;

        if let Std::Err(error_str) = command(
            Model::from(tempdir.as_ref().to_path_buf()),
            Input {
                namespace: ("spec", "quiltcore").into(),
            },
        )
        .await
        {
            assert_eq!(error_str.to_string(), "No changes to push");
        } else {
            return Err(Error::Test("Expected no changes error".to_string()));
        }

        Ok(())
    }
}
