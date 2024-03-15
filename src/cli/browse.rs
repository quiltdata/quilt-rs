use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

pub struct Output {
    manifest: quilt_rs::Table,
}

#[derive(Debug)]
pub struct Input {
    pub uri: String,
}

#[derive(tabled::Tabled)]
struct RemoteManifestHeader {
    info: String,
    meta: String,
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
        let mut header_table = tabled::Table::new(vec![RemoteManifestHeader {
            info: self.manifest.header.info.to_string(),
            meta: self.manifest.header.meta.to_string(),
        }]);
        header_table.with(tabled::settings::Panel::header("Remote manifest header"));
        output.push(header_table.to_string());

        let entries = self.manifest.records.values().map(|e| RemoteManifestEntry {
            name: e.name.to_string(),
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
    let uri: quilt_rs::S3PackageURI = uri.parse()?;
    let remote_manifest = quilt_rs::RemoteManifest::resolve(&uri).await?;
    Ok(Output {
        manifest: local_domain
            .browse_remote_manifest(&remote_manifest)
            .await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use temp_testdir::TempDir;

    #[tokio::test]
    async fn test_model() -> Result<(), Error> {
        let temp_dir = TempDir::default();
        let local_path = PathBuf::from(temp_dir.as_ref());
        let local_domain = quilt_rs::LocalDomain::new(local_path);
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md".to_string();
        let output = model(&local_domain, Input { uri }).await?;
        assert_eq!(
            output.manifest.header.info,
            serde_json::json!({
                "message": "test_spec_write 1697916638",
                "version":"v0"
            })
        );
        assert_eq!(
            output.manifest.records.get("READ ME.md").unwrap().place,
            "s3://udp-spec/spec/quiltcore/READ%20ME.md?versionId=.l3tAGbfEBC4c.L2ywTpWbnweSpYLe8a"
        );
        assert_eq!(
            output.manifest.records.get("timestamp.txt").unwrap().place,
            "s3://udp-spec/spec/quiltcore/timestamp.txt?versionId=lifktjQgrgewg1FGXxls3UKtJSjl2shy"
        );
        Ok(())
    }
}
