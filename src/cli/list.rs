use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

pub struct Output {
    installed_packages_list: Vec<quilt_rs::InstalledPackage>,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.installed_packages_list.is_empty() {
            return write!(f, "No installed packages");
        }
        let mut output: Vec<String> = Vec::new();
        for installed_package in &self.installed_packages_list {
            output.push(format!("InstalledPackage<{}>", installed_package.namespace));
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
        installed_packages_list: local_domain.list_installed_packages().await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use temp_testdir::TempDir;

    #[tokio::test]
    async fn list() -> Result<(), Error> {
        let remote = quilt_rs::io::remote::s3::RemoteS3::new();
        let temp_dir = TempDir::default();
        let local_path = PathBuf::from(temp_dir.as_ref());
        let local_domain = quilt_rs::LocalDomain::new(local_path);
        let uri: quilt_rs::uri::S3PackageUri =
            "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md".parse()?;
        let manifest_uri = quilt_rs::io::manifest::resolve_manifest_uri(&remote, &uri).await?;
        let _ = local_domain.install_package(&manifest_uri).await?;
        let output = model(&local_domain).await?;
        assert_eq!(
            output.installed_packages_list[0].namespace,
            ("spec", "quiltcore").into()
        );
        Ok(())
    }
}
