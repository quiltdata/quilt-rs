use crate::cli::model::install_into_temp_dir;
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
    namespace: Namespace,
) -> Result<ManifestUri, Error> {
    match local_domain.get_installed_package(&namespace).await? {
        Some(installed_package) => Ok(installed_package.pull().await?),
        None => Err(Error::NamespaceNotFound(namespace)),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { namespace }: Input,
) -> Result<Output, Error> {
    let manifest_uri = pull_package(local_domain, namespace).await?;
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
    use tempfile::TempDir;


    /// Verifies that pull updates an outdated package to the latest version:
    ///   * installs an outdated package version
    ///   * pulls the latest version
    ///   * verifies the package is up to date
    #[tokio::test]
    async fn test_model() -> Result<(), Error> {
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore@681f1900320a0bb1de2d6aadd5288c727182ecc32b71115b0b29edc25474e43e";
        let (_, _, local_domain) = install_into_temp_dir(uri).await?;

        let output = model(
            &local_domain,
            Input {
                namespace: ("spec", "quiltcore").into(),
            },
        )
        .await?;

        assert_eq!(
            output.hash,
            "44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c"
        );

        Ok(())
    }

    /// Verifies that pull command returns correct output after pulling latest version
    #[tokio::test]
    async fn test_valid_command() -> Result<(), Error> {
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore@681f1900320a0bb1de2d6aadd5288c727182ecc32b71115b0b29edc25474e43e";
        let (m, _, _temp_dir) = install_into_temp_dir(uri).await?;

        if let Std::Out(output_str) = command(
            m,
            Input {
                namespace: ("spec", "quiltcore").into(),
            },
        )
        .await
        {
            assert_eq!(
                output_str,
                r#"Revision "44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c" pulled"#
            );
        } else {
            return Err(Error::Test("Failed to pull".to_string()));
        }

        Ok(())
    }

    /// Verifies that pull command fails when package is not found
    #[tokio::test]
    async fn test_invalid_command() -> Result<(), Error> {
        let temp_dir = TempDir::new().unwrap();
        let test_model = Model::from(&temp_dir);

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
}
