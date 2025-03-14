use quilt_rs::uri::ManifestUri;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Debug)]
pub struct Input {
    pub message: Option<String>,
    pub target: String,
    pub uri: String,
    pub user_meta: Option<quilt_rs::manifest::JsonObject>,
}

#[derive(Debug)]
pub struct Output {
    manifest_uri: ManifestUri,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Manifest {} created",
            quilt_rs::uri::S3Uri::from(&self.manifest_uri)
        )
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.package(args).await)
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input {
        message,
        target,
        uri,
        user_meta,
    }: Input,
) -> Result<Output, Error> {
    let uri = uri.parse()?;
    let target_uri = target.parse()?;

    let manifest_uri = local_domain
        .package_s3_prefix(&uri, target_uri, message, user_meta)
        .await?;
    Ok(Output { manifest_uri })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::cli::model::Model;

    /// Verifies that CLI throws error if source `s3://` URI is invalid:
    #[test(tokio::test)]
    async fn test_invalid_source() -> Result<(), Error> {
        let uri = "should-be-s3://anything".to_string();

        let (m, _) = Model::from_temp_dir()?;

        if let Std::Err(error_str) = command(
            m,
            Input {
                message: None,
                target: "anything".to_string(),
                uri,
                user_meta: None,
            },
        )
        .await
        {
            assert!(error_str
                .to_string()
                .ends_with("Expected s3:// scheme in should-be-s3://anything"));
        } else {
            return Err(Error::Test("Failed to fail".to_string()));
        }

        Ok(())
    }

    /// Verifies that CLI throws error if target `quilt+s3://` URI is invalid:
    #[test(tokio::test)]
    async fn test_invalid_target() -> Result<(), Error> {
        use crate::cli::fixtures::packages::invalid as pkg;

        let uri = pkg::SOURCE_PK.to_string();
        let target = pkg::URI.to_string();

        let (m, _) = Model::from_temp_dir()?;

        if let Std::Err(error_str) = command(
            m,
            Input {
                message: None,
                target,
                uri,
                user_meta: None,
            },
        )
        .await
        {
            assert!(error_str
                .to_string()
                .starts_with("quilt_rs error: Invalid package URI"),);
        } else {
            return Err(Error::Test("Failed to fail".to_string()));
        }

        Ok(())
    }
}
