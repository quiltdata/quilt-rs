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
    match m.status(args).await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

async fn get_status(
    local_domain: &quilt_rs::LocalDomain,
    namespace: Namespace,
) -> Result<InstalledPackageStatus, Error> {
    match local_domain.get_installed_package(&namespace).await? {
        Some(installed_package) => Ok(installed_package.status().await?),
        None => Err(Error::NamespaceNotFound(namespace)),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { namespace }: Input,
) -> Result<Output, Error> {
    let status = get_status(local_domain, namespace).await?;
    Ok(Output { status })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use temp_testdir::TempDir;

    use quilt_rs::io::storage::LocalStorage;
    use quilt_rs::io::storage::Storage;
    use quilt_rs::uri::ManifestUri;
    use quilt_rs::uri::S3PackageUri;
    use quilt_rs::LocalDomain;

    #[tokio::test]
    async fn test_model() -> Result<(), Error> {
        let uri = S3PackageUri::try_from("quilt+s3://udp-spec#package=spec/quiltcore@44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c")?;

        let temp_dir = TempDir::default();
        let local_path = PathBuf::from(temp_dir.as_ref());
        let local_domain = LocalDomain::new(local_path);

        let manifest_uri = ManifestUri::try_from(uri)?;
        let installed_package = local_domain.install_package(&manifest_uri).await?;

        let readme_logical_key = PathBuf::from("READ ME.md");
        let timestamp_logical_key = PathBuf::from("timestamp.txt");
        installed_package
            .install_paths(&vec![
                readme_logical_key.clone(),
                timestamp_logical_key.clone(),
            ])
            .await?;

        let output = model(
            &local_domain,
            Input {
                namespace: ("spec", "quiltcore").into(),
            },
        )
        .await?;

        assert_eq!(
            format!("{}", output),
            "Installed package is up to date\nNo changes"
        );

        let new_key = PathBuf::from("foo/bar.md");

        let working_dir = installed_package.working_folder();
        let storage = LocalStorage::new();

        let empty_content = Vec::new();
        storage
            .write_file(working_dir.join(new_key), &empty_content)
            .await?;

        storage
            .write_file(working_dir.join(&readme_logical_key), &empty_content)
            .await?;

        let status_new_files = model(
            &local_domain,
            Input {
                namespace: ("spec", "quiltcore").into(),
            },
        )
        .await?;

        let status_new_files_str = format!("{}", status_new_files);
        assert!(status_new_files_str.contains("Installed package is up to date"));
        assert!(status_new_files_str.contains("foo/bar.md | Added"));
        assert!(status_new_files_str.contains("READ ME.md | Modified"));

        if let Err(e) = storage
            .remove_file(working_dir.join(readme_logical_key))
            .await
        {
            return Err(Error::Test(format!("Failed to remove file: {}", e)));
        }

        let status_file_removed = model(
            &local_domain,
            Input {
                namespace: ("spec", "quiltcore").into(),
            },
        )
        .await?;
        let file_removed_status_str = format!("{}", status_file_removed);
        assert!(file_removed_status_str.contains("Installed package is up to date"));
        assert!(file_removed_status_str.contains("foo/bar.md | Added"));
        assert!(file_removed_status_str.contains("READ ME.md | Removed"));

        let not_found = model(
            &local_domain,
            Input {
                namespace: ("a", "b").into(),
            },
        )
        .await;
        assert_eq!(not_found.unwrap_err().to_string(), "Package a/b not found");

        installed_package
            .commit("Anything".to_string(), None, None)
            .await?;
        let status_ahead = model(
            &local_domain,
            Input {
                namespace: ("spec", "quiltcore").into(),
            },
        )
        .await?;
        assert_eq!(
            format!("{}", status_ahead),
            "Your commits are ahead of the remote\nNo changes"
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_model_when_latest_is_outdated() -> Result<(), Error> {
        let uri = S3PackageUri::try_from("quilt+s3://udp-spec#package=spec/quiltcore@681f1900320a0bb1de2d6aadd5288c727182ecc32b71115b0b29edc25474e43e")?;

        let temp_dir = TempDir::default();
        let local_path = PathBuf::from(temp_dir.as_ref());
        let local_domain = LocalDomain::new(local_path);

        let manifest_uri = ManifestUri::try_from(uri)?;
        let installed_package = local_domain.install_package(&manifest_uri).await?;

        let output = model(
            &local_domain,
            Input {
                namespace: ("spec", "quiltcore").into(),
            },
        )
        .await?;

        assert_eq!(
            format!("{}", output),
            "Your commits are behind the remote\nNo changes"
        );

        installed_package
            .commit("Anything".to_string(), None, None)
            .await?;

        let output = model(
            &local_domain,
            Input {
                namespace: ("spec", "quiltcore").into(),
            },
        )
        .await?;

        assert_eq!(
            format!("{}", output),
            "Your commits are detached from the remote\nNo changes"
        );

        Ok(())
    }
}
