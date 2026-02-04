use quilt_rs::manifest::ManifestRow;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

pub struct Output {
    manifest: quilt_rs::manifest::Manifest,
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

impl From<&ManifestRow> for RemoteManifestEntry {
    fn from(row: &ManifestRow) -> Self {
        Self {
            name: row.logical_key.display().to_string(),
            place: row.physical_key.clone(),
            size: row.size,
        }
    }
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut output: Vec<String> = Vec::new();
        let header = self.manifest.header.clone();

        let message = match &header.message {
            Some(msg) => msg.clone(),
            None => "∅".to_string(),
        };

        let user_meta = match &header.user_meta {
            Some(meta) => match serde_json::to_string(&meta) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("Failed to stringify user_meta: {}", e);
                    format!("⚠ (serialization error: {})", e)
                }
            },
            None => "∅".to_string(),
        };

        let workflow = match &header.workflow {
            Some(w) => match serde_json::to_string(&w) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("Failed to stringify workflow: {}", e);
                    format!("⚠ (serialization error: {})", e)
                }
            },
            None => "∅".to_string(),
        };
        let mut header_table = tabled::Table::new(vec![RemoteManifestHeader {
            message,
            user_meta,
            workflow,
        }]);
        header_table.with(tabled::settings::Panel::header("Remote manifest header"));
        output.push(header_table.to_string());

        let mut entries_table =
            tabled::Table::new(self.manifest.rows.iter().map(RemoteManifestEntry::from));
        entries_table.with(tabled::settings::Panel::header("Remote manifest entries"));
        output.push(entries_table.to_string());
        write!(f, "{}", output.join("\n"))
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.browse(args).await)
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

    Ok(Output { manifest })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use test_log::test;

    use crate::cli::fixtures::get_browse_output;
    use crate::cli::fixtures::packages::default as pkg;
    use crate::cli::model::Model;

    /// Verifies that the remote Quilt registry has the expected manifest.
    /// Test actually fetch the manifest from Quilt, without mocks.
    #[test(tokio::test)]
    async fn test_model() -> Result<(), Error> {
        let uri = pkg::URI_LATEST.to_string();

        let readme_logical_key = PathBuf::from(pkg::README_LK);
        let readme_uri = pkg::README_PK;
        let timestamp_logical_key = PathBuf::from(pkg::TIMESTAMP_LK);
        let timestamp_uri = pkg::TIMESTAMP_PK;

        let (m, _temp_dir) = Model::from_temp_dir()?;
        {
            let local_domain = m.get_local_domain();

            let output = model(local_domain, Input { uri }).await?;

            let output_str = format!("{output}");
            assert_eq!(output_str, get_browse_output()?);

            assert_eq!(
                output.manifest.header.message.as_ref(),
                Some(&"Test message".to_string()),
            );
            assert_eq!(
                output
                    .manifest
                    .get_record(&readme_logical_key)
                    .unwrap()
                    .physical_key,
                readme_uri
            );
            assert_eq!(
                output
                    .manifest
                    .get_record(&timestamp_logical_key)
                    .unwrap()
                    .physical_key,
                timestamp_uri
            );
        }
        Ok(())
    }
}
