use tokio_stream::StreamExt;

use quilt_rs::manifest::Row;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

pub struct Output {
    manifest: quilt_rs::manifest::Table,
    rows: Vec<Row>,
}

#[derive(Debug)]
pub struct Input {
    pub uri: String,
}

#[derive(tabled::Tabled)]
struct RemoteManifestHeader {
    message: String,
    user_meta: String,
    workflow: String,
}

#[derive(tabled::Tabled)]
struct RemoteManifestEntry {
    name: String,
    place: String,
    size: u64,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut output: Vec<String> = Vec::new();
        let header = self.manifest.header.clone();

        let message = match header.get_message() {
            Ok(Some(msg)) => msg,
            Ok(None) => "∅".to_string(),
            Err(e) => {
                tracing::error!("Failed to get message: {}", e);
                "⚠".to_string()
            }
        };

        let user_meta = match header.get_user_meta() {
            Ok(Some(meta)) => match serde_json::to_string(&meta) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("Failed to stringify user_meta: {}", e);
                    "⚠".to_string()
                }
            },
            Ok(None) => "∅".to_string(),
            Err(e) => {
                tracing::error!("Failed to get user_meta: {}", e);
                "⚠".to_string()
            }
        };

        let workflow = match header.get_workflow() {
            Ok(Some(w)) => match serde_json::to_string(&w) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("Failed to stringify workflow: {}", e);
                    "⚠".to_string()
                }
            },
            Ok(None) => "∅".to_string(),
            Err(e) => {
                tracing::error!("Failed to get workflow: {}", e);
                "⚠".to_string()
            }
        };
        let mut header_table = tabled::Table::new(vec![RemoteManifestHeader {
            message,
            user_meta,
            workflow,
        }]);
        header_table.with(tabled::settings::Panel::header("Remote manifest header"));
        output.push(header_table.to_string());

        let entries = self.rows.clone().into_iter().map(|e| RemoteManifestEntry {
            name: e.name.display().to_string(),
            place: e.place.to_string(),
            size: e.size,
        });
        let mut entries_table = tabled::Table::new(entries);
        entries_table.with(tabled::settings::Panel::header("Remote manifest entries"));
        output.push(entries_table.to_string());
        write!(f, "{}", output.join("\n"))
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    match m.browse(args).await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { uri }: Input,
) -> Result<Output, Error> {
    let remote = local_domain.get_remote();
    let uri: quilt_rs::uri::S3PackageUri = uri.parse()?;
    let manifest_uri =
        quilt_rs::io::manifest::resolve_manifest_uri(remote, &uri.catalog, &uri).await?;

    let manifest = local_domain.browse_remote_manifest(&manifest_uri).await?;

    let mut rows = Vec::new();
    let mut stream = manifest.records_stream().await;
    while let Some(records) = stream.next().await {
        for row in records? {
            rows.push(row?)
        }
    }

    Ok(Output { manifest, rows })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use test_log::test;

    use crate::cli::fixtures;
    use crate::cli::model::Model;

    pub fn get_browse_output() -> Result<String, std::io::Error> {
        let path = std::env::current_dir()?.join("fixtures/reference-quilt-rs-browse-output.txt");
        std::fs::read_to_string(path)
    }

    /// Verifies that the remote Quilt registry has the expected manifest.
    /// Test actually fetch the manifest from Quilt, without mocks.
    #[test(tokio::test)]
    async fn test_model() -> Result<(), Error> {
        let uri = fixtures::DEFAULT_PACKAGE_URI_LATEST.to_string();

        let readme_logical_key = PathBuf::from(fixtures::DEFAULT_PACKAGE_README_LK);
        let readme_uri = fixtures::DEFAULT_PACKAGE_README_PK;
        let timestamp_logical_key = PathBuf::from(fixtures::DEFAULT_PACKAGE_TIMESTAMP_LK);
        let timestamp_uri = fixtures::DEFAULT_PACKAGE_TIMESTAMP_PK;

        let (m, _temp_dir) = Model::from_temp_dir()?;
        {
            let local_domain = m.get_local_domain();

            let output = model(local_domain, Input { uri }).await?;

            let output_str = format!("{}", output);
            assert_eq!(output_str, get_browse_output()?);

            assert_eq!(
                output.manifest.header.get_message()?,
                Some("Test message".to_string()),
            );
            assert_eq!(
                output
                    .manifest
                    .get_record(&readme_logical_key)
                    .await?
                    .unwrap()
                    .place,
                readme_uri
            );
            assert_eq!(
                output
                    .manifest
                    .get_record(&timestamp_logical_key)
                    .await?
                    .unwrap()
                    .place,
                timestamp_uri
            );
        }
        Ok(())
    }

    /// Verifies that CLI throws error if `quilt+s3` URI is invalid.
    #[test(tokio::test)]
    async fn test_if_uri_is_invalid() -> Result<(), Error> {
        let uri = "quilt+s3://some-nonsense".to_string();

        let (model, _temp_dir) = Model::from_temp_dir()?;

        if let Std::Err(error_str) = command(model, Input { uri }).await {
            assert_eq!(
                format!("{}", error_str),
                "quilt_rs error: Invalid package URI: S3 package URI must contain a fragment: quilt+s3://some-nonsense".to_string()
            );
        } else {
            return Err(Error::Test("Failed to fail".to_string()));
        }

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_command() -> Result<(), Error> {
        let uri = format!(
            "{}&path={}",
            fixtures::DEFAULT_PACKAGE_URI_LATEST,
            fixtures::DEFAULT_PACKAGE_README_LK_ESCAPED
        );

        let (model, _temp_dir) = Model::from_temp_dir()?;

        if let Std::Out(output_str) = command(model, Input { uri }).await {
            assert_eq!(output_str, get_browse_output()?);
        } else {
            return Err(Error::Test("Failed to browse".to_string()));
        }
        Ok(())
    }
}
