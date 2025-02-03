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
    use quilt_rs::{InstalledPackage, LocalDomain};

    async fn install_package(
        uri_str: &str,
        temp_dir: Option<TempDir>,
    ) -> Result<(TempDir, InstalledPackage, LocalDomain), Error> {
        let uri = S3PackageUri::try_from(uri_str)?;

        let temp_dir = temp_dir.unwrap_or_else(|| TempDir::default());
        let local_path = PathBuf::from(temp_dir.as_ref());
        let local_domain = LocalDomain::new(local_path);

        let manifest_uri = ManifestUri::try_from(uri)?;
        let installed_package = local_domain.install_package(&manifest_uri).await?;

        // We must return `temp_dir` because otherwise it will be dropped and removed
        Ok((temp_dir, installed_package, local_domain))
    }

    /// Verifies that list model returns correct output for both empty and populated states:
    ///   * empty list shows "No installed packages" message
    ///   * after installing a package, shows the package namespace
    #[tokio::test]
    async fn test_model() -> Result<(), Error> {
        let (test_model, _temp_dir) = Model::from_temp_dir()?;
        let local_domain = test_model.get_local_domain().lock().await;
        
        // Test empty list
        let empty_output = model(&local_domain).await?;
        assert!(empty_output.installed_packages_list.is_empty());
        assert_eq!(format!("{}", empty_output), "No installed packages");

        // Test with one installed package
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore@44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c&path=READ%20ME.md";
        let (_temp_dir, _installed_package, _) = install_package(uri, None).await?;
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

    /// Verifies that list command returns correct output when no packages are installed
    #[tokio::test]
    async fn test_command_empty() -> Result<(), Error> {
        let (test_model, _temp_dir) = Model::from_temp_dir()?;
        
        if let Std::Out(output_str) = command(test_model).await {
            assert_eq!(output_str, "No installed packages");
        } else {
            return Err(Error::Test("Failed to list packages".to_string()));
        }

        Ok(())
    }

    /// Verifies that list command returns correct output after installing a package:
    ///   * shows the installed package namespace
    ///   * formats output according to display implementation
    #[tokio::test]
    async fn test_command_with_package() -> Result<(), Error> {
        let (test_model, temp_dir) = Model::from_temp_dir()?;
        
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore@44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c&path=READ%20ME.md";
        let (_temp_dir, _installed_package, _) = install_package(uri, Some(temp_dir)).await?;
        let test_model = Model::from(temp_dir.path().to_path_buf());
        
        if let Std::Out(output_str) = command(test_model).await {
            assert_eq!(output_str, "InstalledPackage<spec/quiltcore>");
        } else {
            return Err(Error::Test("Failed to list packages".to_string()));
        }

        Ok(())
    }
}
