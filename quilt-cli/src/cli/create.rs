use std::path::PathBuf;

use quilt_rs::uri::Namespace;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Debug)]
pub struct Input {
    pub namespace: Namespace,
    pub source: Option<PathBuf>,
    pub message: Option<String>,
}

#[derive(Debug)]
pub struct Output {
    pub installed_package: quilt_rs::InstalledPackage,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Created package \"{}\"",
            self.installed_package.namespace
        )
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.create(args).await)
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input {
        namespace,
        source,
        message,
    }: Input,
) -> Result<Output, Error> {
    let installed_package = local_domain
        .create_package(namespace, source, message)
        .await?;
    Ok(Output { installed_package })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::cli::model::create_model_in_temp_dir;
    use crate::cli::status;

    #[test(tokio::test)]
    async fn test_create_empty() -> Result<(), Error> {
        let (m, _temp_dir) = create_model_in_temp_dir().await?;

        let output = m
            .create(Input {
                namespace: ("test", "pkg").into(),
                source: None,
                message: None,
            })
            .await?;

        assert_eq!(output.installed_package.namespace.to_string(), "test/pkg");
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_create_then_status_clean() -> Result<(), Error> {
        let (m, _temp_dir) = create_model_in_temp_dir().await?;

        m.create(Input {
            namespace: ("test", "clean").into(),
            source: None,
            message: None,
        })
        .await?;

        let status_output = m
            .status(status::Input {
                namespace: ("test", "clean").into(),
                host_config: None,
            })
            .await?;

        assert!(
            status_output.status.changes.is_empty(),
            "Freshly created package should have no changes"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_create_duplicate() -> Result<(), Error> {
        let (m, _temp_dir) = create_model_in_temp_dir().await?;

        m.create(Input {
            namespace: ("test", "dup").into(),
            source: None,
            message: None,
        })
        .await?;

        let result = m
            .create(Input {
                namespace: ("test", "dup").into(),
                source: None,
                message: None,
            })
            .await;

        assert!(result.is_err());
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_create_with_source() -> Result<(), Error> {
        let (m, temp_dir) = create_model_in_temp_dir().await?;

        let source_dir = temp_dir.path().join("my_source");
        std::fs::create_dir_all(&source_dir)?;
        std::fs::write(source_dir.join("data.csv"), "a,b,c\n1,2,3")?;
        std::fs::write(source_dir.join("readme.txt"), "Hello")?;

        let output = m
            .create(Input {
                namespace: ("test", "src").into(),
                source: Some(source_dir),
                message: Some("Import data".to_string()),
            })
            .await?;

        assert_eq!(output.installed_package.namespace.to_string(), "test/src");

        // Verify status is clean after create with source
        let status_output = m
            .status(status::Input {
                namespace: ("test", "src").into(),
                host_config: None,
            })
            .await?;

        assert!(
            status_output.status.changes.is_empty(),
            "Package created with source should have clean status"
        );
        Ok(())
    }
}
