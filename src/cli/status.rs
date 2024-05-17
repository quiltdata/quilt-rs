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
