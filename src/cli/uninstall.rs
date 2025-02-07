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

    use crate::cli::model::install_package_into_temp_dir;
    use crate::cli::model::Model;

    /// Verifies that uninstall removes an installed package:
    ///   * installs a package
    ///   * uninstalls it
    ///   * verifies it's no longer present
    #[tokio::test]
    async fn test_model() -> Result<(), Error> {
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore@44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c";
        let (m, _, _temp_dir) = install_package_into_temp_dir(uri).await?;

        {
            let local_domain = m.get_local_domain();
            let output = model(
                local_domain,
                Input {
                    namespace: ("spec", "quiltcore").into(),
                },
            )
            .await?;

            assert_eq!(output.namespace, ("spec", "quiltcore").into());
        }

        {
            let local_domain = m.get_local_domain();
            // Try to uninstall again - should fail
            if let Err(error_str) = model(
                local_domain,
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
        }

        Ok(())
    }

    /// Verifies that uninstall command returns correct output after uninstalling a package
    #[tokio::test]
    async fn test_valid_command() -> Result<(), Error> {
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore@44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c";
        let (m, _, _temp_dir) = install_package_into_temp_dir(uri).await?;

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
        let (m, _temp_dir) = Model::from_temp_dir()?;

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
