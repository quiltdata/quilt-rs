use crate::cli::model::Commands;
use crate::cli::output::Std;

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

#[derive(Debug)]
pub struct CommandArgs {
    pub uri: String,
}

// TODO: instead of `fn command` output struct CommandOutput from model
//       and use `impl fmt::Display` for it
pub async fn command(m: impl Commands, args: CommandArgs) -> Std {
    match m.browse_remote_manifest(args).await {
        Ok(manifest_contents) => {
            let mut output: Vec<String> = Vec::new();
            let mut header_table = tabled::Table::new(vec![RemoteManifestHeader {
                info: manifest_contents.header.info.to_string(),
                meta: manifest_contents.header.meta.to_string(),
            }]);
            header_table.with(tabled::settings::Panel::header("Remote manifest header"));
            output.push(header_table.to_string());

            let mut entries = Vec::new();
            for (_name, entry) in manifest_contents.records {
                entries.push(RemoteManifestEntry {
                    name: entry.name.to_string(),
                    place: entry.place.to_string(),
                    size: entry.size,
                });
            }
            let mut entries_table = tabled::Table::new(&entries);
            entries_table.with(tabled::settings::Panel::header("Remote manifest entries"));
            output.push(entries_table.to_string());
            Std::Out(output.join("\n"))
        }
        Err(err) => Std::Err(err),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    CommandArgs { uri: uri_string }: CommandArgs,
) -> Result<quilt_rs::Table, String> {
    let uri = quilt_rs::S3PackageURI::try_from(uri_string.as_str())?;
    let remote_manifest = quilt_rs::RemoteManifest::resolve(&uri).await?;
    local_domain.browse_remote_manifest(&remote_manifest).await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use temp_testdir::TempDir;

    #[tokio::test]
    async fn test_model() -> Result<(), String> {
        let temp_dir = TempDir::default();
        let local_path = PathBuf::from(temp_dir.as_ref());
        let local_domain = quilt_rs::LocalDomain::new(local_path);
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md".to_string();
        let table = model(&local_domain, CommandArgs { uri }).await?;
        assert_eq!(
            table.header.info,
            serde_json::json!({
                "message": "test_spec_write 1697916638",
                "version":"v0"
            })
        );
        assert_eq!(
            table.records.get("READ ME.md").unwrap().place,
            "s3://udp-spec/spec/quiltcore/READ%20ME.md?versionId=.l3tAGbfEBC4c.L2ywTpWbnweSpYLe8a"
        );
        assert_eq!(
            table.records.get("timestamp.txt").unwrap().place,
            "s3://udp-spec/spec/quiltcore/timestamp.txt?versionId=lifktjQgrgewg1FGXxls3UKtJSjl2shy"
        );
        Ok(())
    }
}
