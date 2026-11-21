//! `quilt uninstall --dataspace`: the package taken out of a dataspace's
//! `data.toml` and lock, its working folder removed, and with `--prune` what
//! the Domain's store holds of it that its bucket holds too cleared.

use std::path::PathBuf;

use quilt_rs::dataspace::contract::CONTRACT_FILE;
use quilt_rs::dataspace::contract::Contract;
use quilt_rs::dataspace::contract::PackageKey;
use quilt_rs::dataspace::lock::LOCK_FILE;
use quilt_rs::dataspace::prune::Pruned;
use quilt_rs::dataspace::prune::prune;
use quilt_rs::dataspace::uninstall::UninstallError;
use quilt_rs::dataspace::uninstall::Uninstalled;
use quilt_rs::dataspace::uninstall::uninstall;
use quilt_rs::domain::DomainRoot;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::paths::DomainPaths;

use crate::cli::Error;
use crate::cli::dataspace;
pub use crate::cli::dataspace::Package;
use crate::cli::output::Render;
use crate::cli::output::Std;
use crate::cli::text::printable_line;

#[derive(Debug)]
pub struct Input {
    /// The dataspace: the folder holding `data.toml`.
    pub dataspace: PathBuf,
    /// The package, by its key or by a name the contract gives one package.
    pub package: Package,
    /// Also clear what the Domain's store holds of the package that its
    /// bucket holds too.
    pub prune: bool,
}

pub struct Output {
    package: PackageKey,
    uninstalled: Uninstalled,
    /// What `--prune` cleared; `None` without it, or with no Domain.
    pruned: Option<Pruned>,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let package = &self.package;
        write!(f, "{package}: removed from {CONTRACT_FILE} and {LOCK_FILE}")?;
        if let Some(folder) = &self.uninstalled.removed {
            let folder = printable_line(&folder.display().to_string());
            write!(f, "; removed {folder}")?;
        }
        if let Some(folder) = &self.uninstalled.kept {
            let folder = printable_line(&folder.display().to_string());
            write!(f, "; left {folder} as it is")?;
        }
        let unpushed = self.uninstalled.unpushed();
        if !unpushed.is_empty() {
            let plural = if unpushed.len() == 1 { "" } else { "s" };
            write!(
                f,
                "\n  {} local commit{plural} not pushed stay in the Domain; install the package again, then `quilt reattach` one:",
                unpushed.len()
            )?;
            for commit in unpushed {
                write!(f, "\n    {commit}")?;
            }
        }
        if let Some(pruned) = &self.pruned {
            write!(f, "\n  {pruned}")?;
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let path =
            |folder: &Option<PathBuf>| folder.as_ref().map(|folder| folder.display().to_string());
        let pruned = match &self.pruned {
            None => serde_json::Value::Null,
            Some(Pruned::Freed(report)) => serde_json::json!({
                "manifests": report.cached_manifests,
                "objects": report.objects,
                "bytes": report.bytes,
            }),
            Some(Pruned::Busy(namespace)) => serde_json::json!({ "busy": namespace.to_string() }),
            Some(Pruned::LocalRegistry) => serde_json::json!({ "local_registry": true }),
            Some(_) => serde_json::json!({}),
        };
        serde_json::json!({
            "package": self.package.to_string(),
            "removed": path(&self.uninstalled.removed),
            "kept": path(&self.uninstalled.kept),
            "unpushed": self.uninstalled.unpushed(),
            "pruned": pruned,
        })
    }
}

pub async fn command(credentials: PathBuf, domain: Option<DomainRoot>, args: Input) -> Std {
    let remote = RemoteS3::new(DomainPaths::new(credentials), LocalStorage::new());
    Std::from_result(uninstall_with(&remote, domain.as_ref(), args).await)
}

async fn uninstall_with(
    remote: &impl Remote,
    domain: Option<&DomainRoot>,
    Input {
        dataspace,
        package,
        prune: pruning,
    }: Input,
) -> Result<Output, Error> {
    let storage = LocalStorage::new();
    let local = domain.map(DomainRoot::local);
    let writer = dataspace::writer(local.as_ref()).await?;
    let contract = Contract::read(&storage, &dataspace, writer.local_registry_name())
        .await
        .map_err(UninstallError::from)?;
    let package = package.key_in(&contract)?;
    // The catalog signs the bucket's answers to the prune.
    let catalog = contract
        .entries()
        .iter()
        .find(|entry| entry.key() == package)
        .and_then(|entry| entry.catalog.clone());
    let uninstalled = uninstall(&storage, local.as_ref(), &dataspace, &writer, &package).await?;
    let pruned = match domain.filter(|_| pruning) {
        Some(domain) => Some(prune(&storage, remote, domain, &package, catalog.as_ref()).await?),
        None => None,
    };
    Ok(Output {
        package,
        uninstalled,
        pruned,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use quilt_rs::io::remote::mocks::MockRemote;
    use quilt_uri::Namespace;

    fn key() -> PackageKey {
        "quilt+s3://b#package=team/raw".parse().expect("a key")
    }

    #[test(tokio::test)]
    async fn an_uninstall_by_name_removes_the_package_and_says_so() -> Result<(), Error> {
        let dataspace = tempfile::TempDir::new()?;
        std::fs::write(
            dataspace.path().join("data.toml"),
            "registry = \"s3://b\"\n[packages]\nsync = [\"team/raw\"]\n",
        )?;
        // Joined part by part, so it displays as the contract's folder does
        // on Windows too.
        let folder = dataspace.path().join("team").join("raw");
        std::fs::create_dir_all(&folder)?;
        std::fs::write(folder.join("a.txt"), b"alpha")?;
        let input = Input {
            dataspace: dataspace.path().to_path_buf(),
            package: Package::Namespace(Namespace::try_from("team/raw")?),
            prune: true,
        };

        let output = uninstall_with(&MockRemote::default(), None, input).await?;

        assert_eq!(output.package, key());
        let json = output.to_json();
        assert_eq!(json["removed"], folder.display().to_string());
        assert_eq!(
            json["pruned"],
            serde_json::Value::Null,
            "no Domain to prune"
        );
        assert!(output.to_string().starts_with(
            "quilt+s3://b#package=team/raw: removed from data.toml and data.lock.toml; removed "
        ));
        assert!(!folder.exists());
        let contract = std::fs::read_to_string(dataspace.path().join("data.toml"))?;
        assert!(!contract.contains("team/raw"), "{contract}");
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_name_the_contract_does_not_give_is_not_found() -> Result<(), Error> {
        let dataspace = tempfile::TempDir::new()?;
        std::fs::write(
            dataspace.path().join("data.toml"),
            "registry = \"s3://b\"\n[packages]\nsync = [\"team/raw\"]\n",
        )?;
        let input = Input {
            dataspace: dataspace.path().to_path_buf(),
            package: Package::Namespace(Namespace::try_from("team/other")?),
            prune: false,
        };

        let result = uninstall_with(&MockRemote::default(), None, input).await;

        assert!(
            matches!(result, Err(Error::NamespaceNotFound(_))),
            "{:?}",
            result.err()
        );
        Ok(())
    }
}
