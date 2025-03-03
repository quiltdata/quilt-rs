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

    use test_log::test;

    use crate::cli::fixtures::packages::default as pkg;
    use crate::cli::model::install_package_into_temp_dir;
    use crate::cli::model::Model;

    /// Verifies that push command returns error when push a non-existent package
    #[test(tokio::test)]
    async fn test_namespace_not_found() -> Result<(), Error> {
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

    /// Verifies that push command returns error when there are no commits:
    ///   * installs a package but makes no commits
    ///   * attempts to push without commits
    #[test(tokio::test)]
    async fn test_no_commit() -> Result<(), Error> {
        let uri = pkg::URI;
        let (m, _, _temp_dir) = install_package_into_temp_dir(uri).await?;

        if let Std::Err(error_str) = command(
            m,
            Input {
                namespace: pkg::NAMESPACE.into(),
            },
        )
        .await
        {
            assert_eq!(
                error_str.to_string(),
                "quilt_rs error: Push error: No commits to push"
            );
        } else {
            return Err(Error::Test("Expected no changes error".to_string()));
        }

        Ok(())
    }
}
