use quilt_rs::manifest::ManifestRow;

use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Render;
use crate::cli::output::Std;

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
                    format!("⚠ (serialization error: {e})")
                }
            },
            None => "∅".to_string(),
        };

        let workflow = match &header.workflow {
            Some(w) => match serde_json::to_string(&w) {
                Ok(s) => s,
                Err(e) => {
                    tracing::error!("Failed to stringify workflow: {}", e);
                    format!("⚠ (serialization error: {e})")
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

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let header = &self.manifest.header;
        let entries: Vec<_> = self
            .manifest
            .rows
            .iter()
            .map(|row| {
                serde_json::json!({
                    "logical_key": row.logical_key.display().to_string(),
                    "physical_key": &row.physical_key,
                    "size": row.size,
                })
            })
            .collect();

        serde_json::json!({
            "header": {
                "message": header.message.as_deref(),
                "user_meta": &header.user_meta,
                "workflow": &header.workflow,
            },
            "entries": entries,
        })
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
    let uri: quilt_uri::S3PackageUri = uri.parse()?;

    let manifest_uri =
        quilt_rs::io::manifest::resolve_manifest_uri(remote, uri.catalog.as_ref(), &uri).await?;

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
    async fn live_model() -> Result<(), Error> {
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

    #[test]
    fn json_carries_header_and_entries_in_manifest_vocabulary() {
        let output = Output {
            manifest: quilt_rs::manifest::Manifest {
                header: quilt_rs::manifest::ManifestHeader {
                    version: "v0".to_string(),
                    message: Some("initial".to_string()),
                    user_meta: Some(serde_json::json!({"owner": "team"})),
                    workflow: None,
                },
                rows: vec![quilt_rs::manifest::ManifestRow {
                    logical_key: std::path::PathBuf::from("data/one.csv"),
                    physical_key: "s3://bucket/one.csv".to_string(),
                    hash: quilt_rs::object_hash::ObjectHash::default(),
                    size: 42,
                    meta: None,
                }],
            },
        };

        assert_eq!(
            output.to_json().to_string(),
            r#"{"header":{"message":"initial","user_meta":{"owner":"team"},"workflow":null},"entries":[{"logical_key":"data/one.csv","physical_key":"s3://bucket/one.csv","size":42}]}"#
        );
    }

    /// `∅` is a table affordance and would be indistinguishable from a real
    /// value; JSON says `null`.
    #[test]
    fn json_uses_null_not_the_empty_set_sentinel() {
        let output = Output {
            manifest: quilt_rs::manifest::Manifest {
                header: quilt_rs::manifest::ManifestHeader {
                    version: "v0".to_string(),
                    message: None,
                    user_meta: None,
                    workflow: None,
                },
                rows: Vec::new(),
            },
        };

        let rendered = output.to_json().to_string();
        assert_eq!(
            rendered,
            r#"{"header":{"message":null,"user_meta":null,"workflow":null},"entries":[]}"#
        );
        assert!(!rendered.contains('\u{2205}'));
    }
}
