use quilt_rs::io::remote::HostConfig;
use quilt_rs::lineage::Change;
use quilt_rs::lineage::InstalledPackageStatus;
use quilt_rs::lineage::UpstreamState;
use quilt_rs::uri::Namespace;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Debug)]
pub struct Input {
    pub namespace: Namespace,
    pub host_config: Option<HostConfig>,
}

#[derive(Debug)]
pub struct Output {
    pub status: InstalledPackageStatus,
}

#[derive(tabled::Tabled)]
struct StatusEntry {
    path: String,
    status: String,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut output: Vec<String> = Vec::new();
        let discrete_state = match self.status.upstream_state {
            UpstreamState::UpToDate => "Installed package is up to date",
            UpstreamState::Behind => "Your commits are behind the remote",
            UpstreamState::Ahead => "Your commits are ahead of the remote",
            UpstreamState::Diverged => "Your commits are detached from the remote",
            // Currently only produced by quilt-sync for packages without origin;
            // the CLI's status() call errors out before reaching this state.
            UpstreamState::Error => "Unable to check remote status",
        };

        output.push(discrete_state.to_string());

        if self.status.changes.is_empty() {
            output.push("No changes".to_string());
        } else {
            let entries = self
                .status
                .changes
                .iter()
                .map(|(path, change)| StatusEntry {
                    path: path.display().to_string(),
                    status: match change {
                        Change::Modified(_) => "Modified".to_string(),
                        Change::Added(_) => "Added".to_string(),
                        Change::Removed(_) => "Removed".to_string(),
                    },
                });
            let mut entries_table = tabled::Table::new(entries);
            entries_table.with(tabled::settings::Panel::header("Changes:"));
            output.push(entries_table.to_string());
        }
        write!(f, "{}", output.join("\n"))
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.status(args).await)
}

async fn get_status(
    local_domain: &quilt_rs::LocalDomain,
    namespace: Namespace,
    host_config: Option<HostConfig>,
) -> Result<InstalledPackageStatus, Error> {
    match local_domain.get_installed_package(&namespace).await? {
        Some(installed_package) => Ok(installed_package.status(host_config).await?),
        None => Err(Error::NamespaceNotFound(namespace)),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input {
        namespace,
        host_config,
    }: Input,
) -> Result<Output, Error> {
    let status = get_status(local_domain, namespace, host_config).await?;
    Ok(Output { status })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use test_log::test;

    use crate::cli::model::install_package_into_temp_dir;

    use aws_sdk_s3::primitives::ByteStream;

    use quilt_rs::io::storage::LocalStorage;
    use quilt_rs::io::storage::Storage;

    #[test(tokio::test)]
    async fn test_model() -> Result<(), Error> {
        use crate::cli::fixtures::packages::default as pkg;

        let uri = pkg::URI;

        let (m, installed_package, _temp_dir) = install_package_into_temp_dir(uri).await?;

        let readme_logical_key = PathBuf::from(pkg::README_LK);
        let timestamp_logical_key = PathBuf::from(pkg::TIMESTAMP_LK);
        installed_package
            .install_paths(&[readme_logical_key.clone(), timestamp_logical_key.clone()])
            .await?;

        {
            let local_domain = m.get_local_domain();
            let output = model(
                local_domain,
                Input {
                    namespace: pkg::NAMESPACE.into(),
                    host_config: None,
                },
            )
            .await?;

            assert_eq!(
                format!("{output}"),
                "Installed package is up to date\nNo changes"
            );
        }

        let new_key = PathBuf::from("foo/bar.md");

        let working_dir = installed_package.package_home().await?;
        let storage = LocalStorage::new();

        storage
            .write_byte_stream(working_dir.join(new_key), ByteStream::default())
            .await?;

        storage
            .write_byte_stream(
                working_dir.join(&readme_logical_key),
                ByteStream::default(),
            )
            .await?;

        {
            let local_domain = m.get_local_domain();
            let status_new_files = model(
                local_domain,
                Input {
                    namespace: pkg::NAMESPACE.into(),
                    host_config: None,
                },
            )
            .await?;

            let status_new_files_str = format!("{status_new_files}");
            assert!(status_new_files_str.contains("Installed package is up to date"));
            assert!(
                status_new_files_str.contains("foo/bar.md                               | Added")
            );
            assert!(status_new_files_str
                .contains("one/two two/three three three/READ ME.md | Modified"));
        }

        storage
            .remove_file(working_dir.join(readme_logical_key))
            .await
            .map_err(|e| Error::Test(format!("Failed to remove file: {e}")))?;

        {
            let local_domain = m.get_local_domain();
            let status_file_removed = model(
                local_domain,
                Input {
                    namespace: pkg::NAMESPACE.into(),
                    host_config: None,
                },
            )
            .await?;

            let file_removed_status_str = format!("{status_file_removed}");
            assert!(file_removed_status_str.contains("Installed package is up to date"));
            assert!(file_removed_status_str
                .contains("foo/bar.md                               | Added"));
            assert!(file_removed_status_str
                .contains("one/two two/three three three/READ ME.md | Removed"));
        }

        {
            let local_domain = m.get_local_domain();
            let not_found = model(
                local_domain,
                Input {
                    namespace: ("a", "b").into(),
                    host_config: None,
                },
            )
            .await;

            assert_eq!(not_found.unwrap_err().to_string(), "Package a/b not found");
        }

        installed_package
            .commit("Anything".to_string(), None, None, None)
            .await?;
        {
            let local_domain = m.get_local_domain();
            let status_ahead = model(
                local_domain,
                Input {
                    namespace: pkg::NAMESPACE.into(),
                    host_config: None,
                },
            )
            .await?;
            assert_eq!(
                format!("{status_ahead}"),
                "Your commits are ahead of the remote\nNo changes"
            );
        }

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_model_when_latest_is_outdated() -> Result<(), Error> {
        use crate::cli::fixtures::packages::outdated as pkg;

        let uri = pkg::URI;

        let (m, installed_package, _temp_dir) = install_package_into_temp_dir(uri).await?;

        {
            let local_domain = m.get_local_domain();
            let output = model(
                local_domain,
                Input {
                    namespace: pkg::NAMESPACE.into(),
                    host_config: None,
                },
            )
            .await?;

            assert_eq!(
                format!("{output}"),
                "Your commits are behind the remote\nNo changes"
            );
        }

        installed_package
            .commit("Anything".to_string(), None, None, None)
            .await?;

        {
            let local_domain = m.get_local_domain();
            let output = model(
                local_domain,
                Input {
                    namespace: pkg::NAMESPACE.into(),
                    host_config: None,
                },
            )
            .await?;

            assert_eq!(
                format!("{output}"),
                "Your commits are detached from the remote\nNo changes"
            );
        }

        Ok(())
    }
}
