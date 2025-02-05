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
        let message = header.display_message().unwrap_or_default();
        let user_meta = header.display_user_meta().map_or(String::default(), |v| {
            serde_json::to_string(&v).expect("Failed to stringify user_meta")
        });
        let workflow = header.display_workflow().map_or(String::default(), |v| {
            serde_json::to_string(&v).expect("Failed to stringify workflow")
        });
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
    let remote = quilt_rs::io::remote::RemoteS3::new();
    let uri: quilt_rs::uri::S3PackageUri = uri.parse()?;
    let manifest_uri = quilt_rs::io::manifest::resolve_manifest_uri(&remote, &uri).await?;

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

    use crate::cli::model::Model;

    /// Verifies that the remote Quilt registry has the expected manifest.
    /// Test actually fetch the manifest from Quilt, without mocks.
    #[tokio::test]
    async fn test_model() -> Result<(), Error> {
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md".to_string();

        let readme_logical_key = PathBuf::from("READ ME.md");
        let readme_uri =
            "s3://udp-spec/spec/quiltcore/READ%20ME.md?versionId=.l3tAGbfEBC4c.L2ywTpWbnweSpYLe8a";
        let timestamp_logical_key = PathBuf::from("timestamp.txt");
        let timestamp_uri =
            "s3://udp-spec/spec/quiltcore/timestamp.txt?versionId=lifktjQgrgewg1FGXxls3UKtJSjl2shy";

        let (m, _temp_dir) = Model::from_temp_dir()?;
        let local_domain = m.get_local_domain().lock().await;

        let output = model(&local_domain, Input { uri }).await?;

        let output_str = format!("{}", output);
        assert!(output_str.contains(r#"| test_spec_write 1697916638 | {"Author":"Ernest","Count":1,"Date":"2023-07-12"} |          |"#));
        assert!(output_str.contains(r#"| READ ME.md    | s3://udp-spec/spec/quiltcore/READ%20ME.md?versionId=.l3tAGbfEBC4c.L2ywTpWbnweSpYLe8a  | 33   |"#));
        assert!(output_str.contains(r#"| timestamp.txt | s3://udp-spec/spec/quiltcore/timestamp.txt?versionId=lifktjQgrgewg1FGXxls3UKtJSjl2shy | 10   |"#));

        assert_eq!(
            output.manifest.header.display_message().unwrap(),
            "test_spec_write 1697916638",
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
        Ok(())
    }

    /// Verifies that CLI throws error if `quilt+s3` URI is invalid.
    #[tokio::test]
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

    #[tokio::test]
    async fn test_command() -> Result<(), Error> {
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md".to_string();

        let (model, _temp_dir) = Model::from_temp_dir()?;

        if let Std::Out(output_str) = command(model, Input { uri }).await {
            assert!(output_str.contains(r#"| test_spec_write 1697916638 | {"Author":"Ernest","Count":1,"Date":"2023-07-12"} |          |"#));
            assert!(output_str.contains(r#"| READ ME.md    | s3://udp-spec/spec/quiltcore/READ%20ME.md?versionId=.l3tAGbfEBC4c.L2ywTpWbnweSpYLe8a  | 33   |"#));
            assert!(output_str.contains(r#"| timestamp.txt | s3://udp-spec/spec/quiltcore/timestamp.txt?versionId=lifktjQgrgewg1FGXxls3UKtJSjl2shy | 10   |"#));
        } else {
            return Err(Error::Test("Failed to browse".to_string()));
        }
        Ok(())
    }
}
