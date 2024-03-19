use std::fmt;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

pub struct Output {
    installed_packages_list: Vec<quilt_rs::InstalledPackage>,
}

impl fmt::Display for Output {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.installed_packages_list.is_empty() {
            return write!(f, "No installed packages");
        }
        let mut output: Vec<String> = Vec::new();
        for installed_package in &self.installed_packages_list {
            output.push(installed_package.to_string());
        }
        write!(f, "{}", output.join("\n"))
    }
}

pub async fn command(m: impl Commands) -> Std {
    match m.list().await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

pub async fn model(local_domain: &quilt_rs::LocalDomain) -> Result<Output, Error> {
    Ok(Output {
        installed_packages_list: local_domain
            .list_installed_packages(&local_domain.lineage_io)
            .await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use temp_testdir::TempDir;

    #[tokio::test]
    async fn list() -> Result<(), Error> {
        let temp_dir = TempDir::default();
        let local_path = PathBuf::from(temp_dir.as_ref());
        let local_domain = quilt_rs::LocalDomain::new(local_path);
        let uri: quilt_rs::S3PackageUri =
            "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md".parse()?;
        let remote_manifest = quilt_rs::RemoteManifest::resolve(&uri).await?;
        let _ = local_domain
            .install_package(&local_domain.lineage_io, &remote_manifest)
            .await?;
        let output = model(&local_domain).await?;
        assert_eq!(
            output.installed_packages_list[0].namespace,
            "spec/quiltcore"
        );
        Ok(())
    }
}
