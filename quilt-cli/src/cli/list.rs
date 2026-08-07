use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Std;
use quilt_rs::lineage::UpstreamState;

#[derive(tabled::Tabled)]
struct PackageEntry {
    namespace: String,
    revision: String,
    status: String,
}

pub struct Output {
    installed_packages_list: Vec<PackageEntry>,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.installed_packages_list.is_empty() {
            return write!(f, "No installed packages");
        }

        let table = tabled::Table::new(self.installed_packages_list.iter());
        write!(f, "{table}")
    }
}

pub async fn command(m: impl Commands) -> Std {
    Std::from_result(m.list().await)
}

pub async fn model(local_domain: &quilt_rs::LocalDomain) -> Result<Output, Error> {
    let installed_packages = local_domain.list_installed_packages().await?;
    let mut installed_packages_list = Vec::with_capacity(installed_packages.len());

    for installed_package in installed_packages {
        let lineage = installed_package.lineage().await?;
        let status = match installed_package.status(None).await {
            Ok(status) => status.upstream_state,
            Err(error) => {
                tracing::warn!(
                    "Failed to get status for {}: {error}",
                    installed_package.namespace
                );
                UpstreamState::Error
            }
        };

        installed_packages_list.push(PackageEntry {
            namespace: installed_package.namespace.to_string(),
            revision: lineage.current_hash().map_or_else(
                || "-".to_string(),
                |hash| hash.chars().take(8).collect(),
            ),
            status: status.to_string(),
        });
    }

    Ok(Output {
        installed_packages_list,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::cli::fixtures::packages::default as pkg;
    use crate::cli::model::Commands;
    use crate::cli::model::create_model_in_temp_dir;
    use crate::cli::model::install_package_into_temp_dir;

    #[test(tokio::test)]
    async fn test_empty_list() -> Result<(), Error> {
        let (m, _temp_dir) = create_model_in_temp_dir().await?;
        {
            let local_domain = m.get_local_domain();
            let empty_output = model(local_domain).await?;
            assert!(empty_output.installed_packages_list.is_empty());
            assert_eq!(format!("{empty_output}"), "No installed packages");
        }
        Ok(())
    }

    #[test]
    fn test_display_renders_package_table() {
        let output = Output {
            installed_packages_list: vec![PackageEntry {
                namespace: "example/package".to_string(),
                revision: "12345678".to_string(),
                status: "up_to_date".to_string(),
            }],
        };

        let output = format!("{output}");
        assert!(output.contains("namespace"));
        assert!(output.contains("revision"));
        assert!(output.contains("status"));
        assert!(output.contains("example/package"));
        assert!(output.contains("12345678"));
        assert!(output.contains("up_to_date"));
    }

    #[test(tokio::test)]
    async fn test_model_with_local_package() -> Result<(), Error> {
        let (m, _temp_dir) = create_model_in_temp_dir().await?;
        let created = m
            .create(crate::cli::create::Input {
                namespace: ("example", "local").into(),
                source: None,
                message: None,
            })
            .await?;
        let revision = created
            .installed_package
            .lineage()
            .await?
            .current_hash()
            .map(|hash| hash.chars().take(8).collect::<String>())
            .expect("created packages have a current revision");

        let output = model(m.get_local_domain()).await?;
        let output = format!("{output}");
        assert!(output.contains("example/local"));
        assert!(output.contains(&revision));
        assert!(output.contains("local"));

        Ok(())
    }

    /// Verifies that list model returns correct output for both empty and populated states:
    ///   * empty list shows "No installed packages" message
    ///   * after installing a package, shows the package details in a table
    #[test(tokio::test)]
    async fn test_model() -> Result<(), Error> {
        // Test with one installed package
        let uri = format!("{}&path={}", pkg::URI, pkg::README_LK_ESCAPED);
        let (m, _, _temp_dir) = install_package_into_temp_dir(&uri).await?;
        {
            let local_domain = m.get_local_domain();
            let output = model(local_domain).await?;

            assert_eq!(
                output.installed_packages_list[0].namespace,
                pkg::NAMESPACE_STR
            );
            let output = format!("{output}");
            assert!(output.contains("namespace"));
            assert!(output.contains("revision"));
            assert!(output.contains("status"));
            assert!(output.contains(&pkg::TOP_HASH[..8]));
            assert!(output.contains("up_to_date"));
        }

        Ok(())
    }

    /// Verifies that list command returns correct output after installing a package:
    ///   * shows the installed package namespace
    ///   * formats output according to display implementation
    // TODO: install and list multiple packages
    #[test(tokio::test)]
    async fn test_command_with_package() -> Result<(), Error> {
        let uri = format!("{}&path={}", pkg::URI, pkg::README_LK_ESCAPED);
        let (m, _, _temp_dir) = install_package_into_temp_dir(&uri).await?;

        if let Std::Out(output) = command(m).await {
            assert!(output.contains("namespace"));
            assert!(output.contains(pkg::NAMESPACE_STR));
            assert!(output.contains(&pkg::TOP_HASH[..8]));
            assert!(output.contains("up_to_date"));
        } else {
            return Err(Error::Test("Failed to list packages".to_string()));
        }

        Ok(())
    }
}
