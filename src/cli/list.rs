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
    use crate::cli::model::Model;
    use quilt_rs::uri::{S3PackageUri, ManifestUri};

    async fn install_package(uri_str: &str) -> Result<(TempDir, quilt_rs::LocalDomain), Error> {
        let uri = S3PackageUri::try_from(uri_str)?;
        let temp_dir = TempDir::default();
        let local_path = PathBuf::from(temp_dir.as_ref());
        let local_domain = quilt_rs::LocalDomain::new(local_path);
        let manifest_uri = ManifestUri::try_from(uri)?;
        let _ = local_domain.install_package(&manifest_uri).await?;
        Ok((temp_dir, local_domain))
    }

    #[tokio::test]
    async fn test_model() -> Result<(), Error> {
        // Test empty list
        let temp_dir = TempDir::default();
        let empty_local_domain = quilt_rs::LocalDomain::new(PathBuf::from(temp_dir.as_ref()));
        let empty_output = model(&empty_local_domain).await?;
        assert!(empty_output.installed_packages_list.is_empty());
        assert_eq!(format!("{}", empty_output), "No installed packages");

        // Test with one installed package
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md";
        let (_temp_dir, local_domain) = install_package(uri).await?;
        let output = model(&local_domain).await?;
        
        assert_eq!(
            output.installed_packages_list[0].namespace,
            ("spec", "quiltcore").into()
        );
        assert_eq!(
            format!("{}", output),
            "InstalledPackage<spec/quiltcore>"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_command() -> Result<(), Error> {
        // Test empty list via command
        let (model, _temp_dir) = Model::from_temp_dir()?;
        if let Std::Out(output_str) = command(model).await {
            assert_eq!(output_str, "No installed packages");
        } else {
            return Err(Error::Test("Failed to list packages".to_string()));
        }

        // Test with installed package via command
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md";
        let (_temp_dir, local_domain) = install_package(uri).await?;
        let model = Model::new(local_domain);
        
        if let Std::Out(output_str) = command(model).await {
            assert_eq!(output_str, "InstalledPackage<spec/quiltcore>");
        } else {
            return Err(Error::Test("Failed to list packages".to_string()));
        }

        Ok(())
    }
}
