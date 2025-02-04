use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

use quilt_rs::uri::Namespace;

#[derive(Debug)]
pub struct Input {
    pub namespace: Namespace,
}

pub struct Output {
    namespace: Namespace,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Package {} successfully uninstalled", self.namespace)
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    match m.uninstall(args).await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,

    Input { namespace }: Input,
) -> Result<Output, Error> {
    local_domain.uninstall_package(namespace.clone()).await?;
    Ok(Output { namespace })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::model::Model;
    use quilt_rs::uri::{ManifestUri, S3PackageUri};
    use quilt_rs::{InstalledPackage, LocalDomain};
    use std::path::PathBuf;
    use tempfile::TempDir;

    async fn install_package(
        uri_str: &str,
    ) -> Result<(TempDir, InstalledPackage, LocalDomain), Error> {
        let uri = S3PackageUri::try_from(uri_str)?;

        let temp_dir = TempDir::new()?;
        let local_path = PathBuf::from(temp_dir.as_ref());
        let local_domain = LocalDomain::new(local_path);

        let manifest_uri = ManifestUri::try_from(uri)?;
        let installed_package = local_domain.install_package(&manifest_uri).await?;

        Ok((temp_dir, installed_package, local_domain))
    }

    /// Verifies that uninstall removes an installed package:
    ///   * installs a package
    ///   * uninstalls it
    ///   * verifies it's no longer present
    #[tokio::test]
    async fn test_model() -> Result<(), Error> {
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore@44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c";
        // Don't drop temp_dir, because it contains lineage
        let (_temp_dir, _, local_domain) = install_package(uri).await?;

        let output = model(
            &local_domain,
            Input {
                namespace: ("spec", "quiltcore").into(),
            },
        )
        .await?;

        assert_eq!(output.namespace, ("spec", "quiltcore").into());

        // Try to uninstall again - should fail
        if let Err(error_str) = model(
            &local_domain,
            Input {
                namespace: ("spec", "quiltcore").into(),
            },
        )
        .await
        {
            assert_eq!(
                error_str.to_string(),
                "quilt_rs error: The given package is not installed: spec/quiltcore"
            );
        } else {
            return Err(Error::Test("Expected package not found error".to_string()));
        }

        Ok(())
    }

    /// Verifies that uninstall command returns correct output after uninstalling a package
    #[tokio::test]
    async fn test_valid_command() -> Result<(), Error> {
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore@44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c";
        let (temp_dir, _installed_package, _) = install_package(uri).await?;
        let test_model = Model::from(temp_dir.as_ref().to_path_buf());

        if let Std::Out(output_str) = command(
            test_model,
            Input {
                namespace: ("spec", "quiltcore").into(),
            },
        )
        .await
        {
            assert_eq!(
                output_str,
                "Package spec/quiltcore successfully uninstalled"
            );
        } else {
            return Err(Error::Test("Failed to uninstall".to_string()));
        }

        Ok(())
    }

    /// Verifies that uninstall command fails when package is not found
    #[tokio::test]
    async fn test_invalid_command() -> Result<(), Error> {
        let temp_dir = TempDir::new().unwrap();
        let m = Model::from(temp_dir.path().to_path_buf());

        if let Std::Err(error_str) = command(
            m,
            Input {
                namespace: ("in", "valid").into(),
            },
        )
        .await
        {
            assert!(error_str
                .to_string()
                .ends_with("The given package is not installed: in/valid"),);
        } else {
            return Err(Error::Test("Expected package not found error".to_string()));
        }

        Ok(())
    }
}
