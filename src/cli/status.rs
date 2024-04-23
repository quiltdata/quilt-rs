use quilt_rs::InstalledPackageStatus;
use quilt_rs::UpstreamDiscreteState;
use quilt_rs::DiscreteChange;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Debug)]
pub struct Input {
    pub namespace: String,
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
            UpstreamDiscreteState::UpToDate => "Installed package is up to date",
            UpstreamDiscreteState::Behind => "Your commits are behind the remote",
            UpstreamDiscreteState::Ahead => "Your commits are ahead of the remote",
            UpstreamDiscreteState::Diverged => "Your commits are detached from the remote",
        };

        output.push(discrete_state.to_string());

        if self.status.changes.is_empty() {
            output.push("No changes".to_string());
        } else {
            let entries = self
                .status
                .changes
                .iter()
                .map(|(name, change)| StatusEntry {
                    path: name.to_string(),
                    status: match change.state {
                        DiscreteChange::Modified => "Modified".to_string(),
                        DiscreteChange::Added => "Added".to_string(),
                        DiscreteChange::Removed => "Removed".to_string(),
                        DiscreteChange::Pristine => "".to_string(),
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
    namespace: String,
) -> Result<InstalledPackageStatus, Error> {
    let installed_package = local_domain.get_installed_package(&namespace).await?;

    match installed_package {
        Some(installed_package) => Ok(installed_package.status().await?),
        None => Err(Error::NamespaceNotFound(namespace.to_string())),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { namespace }: Input,
) -> Result<Output, Error> {
    let status = get_status(local_domain, namespace).await?;
    Ok(Output { status })
}
