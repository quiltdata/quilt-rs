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

    use std::fs::Permissions;
    use std::os::unix::fs::PermissionsExt;
    use tempfile::Builder;

    use crate::cli::model::install_into_temp_dir;
    use crate::cli::model::Model;

    #[tokio::test]
    async fn test_empty_list() -> Result<(), Error> {
        let (m, _temp_dir) = Model::from_temp_dir()?;
        let local_domain = m.get_local_domain().lock().await;
        let empty_output = model(&local_domain).await?;
        assert!(empty_output.installed_packages_list.is_empty());
        assert_eq!(format!("{}", empty_output), "No installed packages");
        Ok(())
    }

    /// Verifies that list model returns correct output for both empty and populated states:
    ///   * empty list shows "No installed packages" message
    ///   * after installing a package, shows the package namespace
    #[tokio::test]
    async fn test_model() -> Result<(), Error> {
        // Test with one installed package
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore@44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c&path=READ%20ME.md";
        let (m, _, _temp_dir) = install_into_temp_dir(uri).await?;
        let local_domain = m.get_local_domain().lock().await;
        let output = model(&local_domain).await?;

        assert_eq!(
            output.installed_packages_list[0].namespace,
            ("spec", "quiltcore").into()
        );
        assert_eq!(format!("{}", output), "InstalledPackage<spec/quiltcore>");

        Ok(())
    }

    /// Verifies that list command returns correct output when no packages are installed
    #[tokio::test]
    async fn test_command_empty() -> Result<(), Error> {
        let (m, _temp_dir) = Model::from_temp_dir()?;

        if let Std::Out(output) = command(m).await {
            assert_eq!(output, "No installed packages");
        } else {
            return Err(Error::Test("Failed to list packages".to_string()));
        }

        Ok(())
    }

    /// Verifies that list command returns correct output after installing a package:
    ///   * shows the installed package namespace
    ///   * formats output according to display implementation
    // TODO: install and list multiple packages
    #[tokio::test]
    async fn test_command_with_package() -> Result<(), Error> {
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore@44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c&path=READ%20ME.md";
        let (m, _, _temp_dir) = install_into_temp_dir(uri).await?;

        if let Std::Out(output) = command(m).await {
            assert_eq!(output, "InstalledPackage<spec/quiltcore>");
        } else {
            return Err(Error::Test("Failed to list packages".to_string()));
        }

        Ok(())
    }

    /// Verifies that list command returns appropriate error when command fails
    /// (no permissions to the domain directory):
    #[tokio::test]
    async fn test_invalid_command() -> Result<(), Error> {
        let write_only = Permissions::from_mode(0o200);
        let temp_dir = Builder::new().permissions(write_only).tempdir()?;

        let m = Model::from(&temp_dir);

        if let Std::Err(Error::Quilt(quilt_rs::Error::Io(orig_err))) = command(m).await {
            assert_eq!(orig_err.kind(), std::io::ErrorKind::PermissionDenied);
        } else {
            return Err(Error::Test("Expected permission error".to_string()));
        }

        Ok(())
    }
}
