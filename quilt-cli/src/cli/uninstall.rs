use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

use quilt_uri::Namespace;

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
    Std::from_result(m.uninstall(args).await)
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

    use test_log::test;

    use crate::cli::fixtures::packages::default as pkg;
    use crate::cli::model::install_package_into_temp_dir;

    /// Verifies that uninstall removes an installed package:
    ///   * installs a package
    ///   * uninstalls it
    ///   * verifies it's no longer present
    #[test(tokio::test)]
    async fn test_model() -> Result<(), Error> {
        let uri = pkg::URI;
        let (m, _, _temp_dir) = install_package_into_temp_dir(uri).await?;

        {
            let local_domain = m.get_local_domain();
            let output = model(
                local_domain,
                Input {
                    namespace: pkg::NAMESPACE.into(),
                },
            )
            .await?;

            assert_eq!(output.namespace, ("reference", "quilt-rs").into());
        }

        {
            let local_domain = m.get_local_domain();
            // Try to uninstall again - should fail
            if let Err(error_str) = model(
                local_domain,
                Input {
                    namespace: pkg::NAMESPACE.into(),
                },
            )
            .await
            {
                assert_eq!(
                    error_str.to_string(),
                    "quilt_rs error: The given package is not installed: reference/quilt-rs"
                );
            } else {
                return Err(Error::Test("Expected package not found error".to_string()));
            }
        }

        Ok(())
    }
}
