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

    use test_log::test;

    use crate::cli::model::install_package_into_temp_dir;
    use crate::cli::model::Model;

    /// Verifies that pull updates an outdated package to the latest version:
    ///   * installs an outdated package version
    ///   * pulls the latest version
    ///   * verifies the package is up to date
    #[test(tokio::test)]
    async fn test_model() -> Result<(), Error> {
        let uri = "quilt+s3://data-yaml-spec-tests#package=scale/10u@f8216f57739c9824f22f1f7a1f8ded59fd50791c92bf9c317d06376811ecbfef";
        let (m, _, _temp_dir) = install_package_into_temp_dir(uri).await?;
        {
            let local_domain = m.get_local_domain();

            let output = model(
                local_domain,
                Input {
                    namespace: ("scale", "10u").into(),
                },
            )
            .await?;

            assert_eq!(
                output.hash,
                "ae239090f2a01de382e8af719fe4a451ef1d1fa4a3ef7b21c6b36513d42c6630"
            );
        }

        Ok(())
    }

    /// Verifies that pull command returns correct output after pulling latest version
    #[test(tokio::test)]
    async fn test_valid_command() -> Result<(), Error> {
        let uri = "quilt+s3://data-yaml-spec-tests#package=scale/10u@f8216f57739c9824f22f1f7a1f8ded59fd50791c92bf9c317d06376811ecbfef";
        let (m, _, _temp_dir) = install_package_into_temp_dir(uri).await?;

        if let Std::Out(output_str) = command(
            m,
            Input {
                namespace: ("scale", "10u").into(),
            },
        )
        .await
        {
            assert_eq!(
                output_str,
                r#"Revision "ae239090f2a01de382e8af719fe4a451ef1d1fa4a3ef7b21c6b36513d42c6630" pulled"#
            );
        } else {
            return Err(Error::Test("Failed to pull".to_string()));
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
