use crate::cli::model::Commands;
use crate::cli::output::Std;

// TODO: instead of `fn command` output struct CommandOutput from model
//       and use `impl fmt::Display` for it
pub async fn command(m: impl Commands) -> Std {
    match m.get_installed_packages_list().await {
        Ok(installed_packages_list) => {
            let mut output: Vec<String> = Vec::new();
            for installed_package in installed_packages_list {
                output.push(format!("InstalledPackage<{}>", installed_package.namespace));
            }
            Std::Out(output.join("\n"))
        }
        Err(err) => Std::Err(err),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
) -> Result<Vec<quilt_rs::InstalledPackage>, String> {
    local_domain.list_installed_packages().await
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use temp_testdir::TempDir;

    #[tokio::test]
    async fn list() -> Result<(), String> {
        let temp_dir = TempDir::default();
        let local_path = PathBuf::from(temp_dir.as_ref());
        let local_domain = quilt_rs::LocalDomain::new(local_path);
        let uri_str = "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md";
        let uri = quilt_rs::S3PackageURI::try_from(uri_str)?;
        let remote_manifest = quilt_rs::RemoteManifest::resolve(&uri).await?;
        let _ = local_domain.install_package(&remote_manifest).await?;
        let list = model(&local_domain).await?;
        assert_eq!(list[0].namespace, "spec/quiltcore");
        Ok(())
    }
}
