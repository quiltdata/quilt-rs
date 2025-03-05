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
    let ManifestUri { hash, .. } = pull_package(local_domain, namespace).await?;
    Ok(Output { hash })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::cli::fixtures::packages::outdated as pkg;
    use crate::cli::model::install_package_into_temp_dir;
    use crate::cli::model::Model;

    /// Verifies that pull updates an outdated package to the latest version:
    ///   * installs an outdated package version
    ///   * pulls the latest version
    ///   * verifies the package is up to date
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

            assert_eq!(output.hash, pkg::LATEST_TOP_HASH);
        }

        Ok(())
    }

    /// Verifies that pull command fails when package is not found
    #[test(tokio::test)]
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
            assert_eq!(error_str.to_string(), "Package in/valid not found");
        } else {
            return Err(Error::Test("Expected package not found error".to_string()));
        }

        Ok(())
    }
}
