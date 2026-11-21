//! `quilt list --dataspace`: the packages a dataspace declares, each with a
//! summary of its status. Writes nothing.

use std::path::Path;
use std::path::PathBuf;

use quilt_rs::dataspace::contract::CONTRACT_FILE;
use quilt_rs::dataspace::list::DeclaredPackage;
use quilt_rs::dataspace::list::list;
use quilt_rs::dataspace::status::LatestSource;
use quilt_rs::domain::DomainRoot;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::lineage::PackageState;
use quilt_rs::paths::DomainPaths;

use crate::cli::Error;
use crate::cli::dataspace::reader;
use crate::cli::output::Render;
use crate::cli::output::Std;
use crate::cli::status::dataspace::existing_queue;
use crate::cli::status::dataspace::kept_kind;
use crate::cli::status::dataspace::state_kind;
use crate::cli::status::dataspace::state_words;
use crate::cli::text::printable_line;

#[derive(Debug)]
pub struct Input {
    pub dataspace: PathBuf,
    /// Ask each bucket for its `latest` now.
    pub fetch: bool,
}

pub struct Output {
    dataspace: PathBuf,
    fetch: bool,
    packages: Vec<DeclaredPackage>,
}

#[derive(tabled::Tabled)]
struct Row {
    package: String,
    access: String,
    status: String,
    folder: String,
}

/// Printed under a table not checked against the registries now.
const RECORDED_HINT: &str = "Statuses are against the latest last known here. \
Run with --fetch to ask each registry now.";

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.packages.is_empty() {
            return write!(
                f,
                "No entries in {}",
                printable_line(&self.dataspace.join(CONTRACT_FILE).display().to_string())
            );
        }
        let rows = self.packages.iter().map(|declared| {
            let mut status = state_words(declared.state);
            if declared.unpushed > 0 && declared.state != PackageState::PendingCommit {
                status.push_str(", local commits not pushed");
            }
            if declared.conflicts > 0 {
                status = format!("{status}, {} in conflict", declared.conflicts);
            }
            if declared.kept.is_some() {
                status.push_str(", a revision kept");
            }
            if self.fetch && declared.latest_source != LatestSource::Registry {
                status.push_str(" (registry unreachable)");
            }
            Row {
                package: declared.entry.key().to_string(),
                access: declared.entry.access.to_string(),
                status,
                folder: printable_line(&declared.entry.folder.display().to_string()),
            }
        });
        write!(f, "{}", tabled::Table::new(rows))?;
        if !self.fetch {
            write!(f, "\n{RECORDED_HINT}")?;
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let packages: Vec<_> = self
            .packages
            .iter()
            .map(|declared| {
                serde_json::json!({
                    "package": declared.entry.key().to_string(),
                    "access": declared.entry.access.to_string(),
                    "folder": declared.entry.folder.display().to_string(),
                    "state": state_kind(declared.state),
                    "upstream_state": declared.upstream_state,
                    "unpushed": declared.unpushed,
                    "conflicts": declared.conflicts,
                    "kept": declared.kept.map(kept_kind),
                    "problems": declared.problems,
                })
            })
            .collect();
        serde_json::json!({
            "dataspace": self.dataspace.display().to_string(),
            "packages": packages,
        })
    }
}

pub async fn command(credentials: PathBuf, domain: Option<DomainRoot>, args: Input) -> Std {
    let paths = DomainPaths::new(credentials);
    let user_rules = paths.user_quiltignore();
    let remote = RemoteS3::new(paths, LocalStorage::new());
    Std::from_result(list_with(&remote, domain.as_ref(), args, &user_rules).await)
}

async fn list_with(
    remote: &impl Remote,
    domain: Option<&DomainRoot>,
    Input { dataspace, fetch }: Input,
    user_rules: &Path,
) -> Result<Output, Error> {
    let writer = reader(domain).await?;
    let queue = existing_queue(domain).await;
    let packages = Box::pin(list(
        &LocalStorage::new(),
        remote,
        domain,
        queue.as_ref(),
        &dataspace,
        &writer,
        user_rules,
        fetch,
    ))
    .await?;
    Ok(Output {
        dataspace,
        fetch,
        packages,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use quilt_rs::io::remote::mocks::MockRemote;
    use test_log::test;

    use crate::cli::status::dataspace::tests::filled;
    use crate::cli::status::dataspace::tests::seed;

    const SECOND: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    /// Without `--fetch`, the list is against the revision last read here;
    /// with it, against the bucket's `latest` now.
    #[test(tokio::test)]
    async fn a_dataspace_list_shows_each_package_fetched_or_not() -> Result<(), Error> {
        let remote = MockRemote::default();
        let dataspace = filled(&remote).await?;
        seed(&remote, SECOND, &[("a.txt", b"v2")], 1_700_000_060).await?;
        let no_rules = dataspace.path().join("no-user-rules");
        let input = |fetch| Input {
            dataspace: dataspace.path().to_path_buf(),
            fetch,
        };

        let recorded = list_with(&remote, None, input(false), &no_rules).await?;
        let fetched = list_with(&remote, None, input(true), &no_rules).await?;

        assert_eq!(recorded.to_json()["packages"][0]["state"], "latest");
        assert!(recorded.to_string().contains(RECORDED_HINT), "{recorded}");
        assert_eq!(fetched.to_json()["packages"][0]["state"], "behind");
        let text = fetched.to_string();
        assert!(text.contains("quilt+s3://b#package=team/raw"), "{text}");
        assert!(text.contains("behind latest"), "{text}");
        assert_eq!(
            recorded.to_json()["packages"][0]["package"],
            "quilt+s3://b#package=team/raw"
        );
        Ok(())
    }
}
