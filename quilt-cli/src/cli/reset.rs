//! `quilt reset --dataspace`: a package's local commits and changes
//! discarded, and `latest` read whole into its working folder as its base.

use std::path::PathBuf;

use quilt_rs::dataspace::contract::PackageKey;
use quilt_rs::dataspace::reset_to_latest::Reset;
use quilt_rs::dataspace::reset_to_latest::reset_to_latest;
use quilt_rs::domain::DomainRoot;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::paths::DomainPaths;

use crate::cli::Error;
use crate::cli::dataspace;
use crate::cli::dataspace::Package;
use crate::cli::output::Render;
use crate::cli::output::Std;

#[derive(Debug)]
pub struct Input {
    /// The dataspace: the folder holding `data.toml`.
    pub dataspace: PathBuf,
    pub package: Package,
}

pub struct Output {
    package: PackageKey,
    reset: Reset,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { package, reset } = self;
        write!(f, "{package}: reset to {}", reset.revision)?;
        let discarded = &reset.discarded;
        if !discarded.is_empty() {
            let plural = if discarded.len() == 1 { "" } else { "s" };
            write!(
                f,
                "\n  {} local commit{plural} discarded stay in the Domain; `quilt reattach` lists them:",
                discarded.len()
            )?;
            for commit in discarded {
                write!(f, "\n    {commit}")?;
            }
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let paths = |keys: &[PathBuf]| -> Vec<String> {
            keys.iter().map(|key| key.display().to_string()).collect()
        };
        serde_json::json!({
            "package": self.package.to_string(),
            "revision": self.reset.revision,
            "discarded": self.reset.discarded,
            "placed": paths(&self.reset.placed),
            "removed": paths(&self.reset.removed),
        })
    }
}

pub async fn command(credentials: PathBuf, domain: DomainRoot, args: Input) -> Std {
    let remote = RemoteS3::new(DomainPaths::new(credentials), LocalStorage::new());
    Std::from_result(reset_with(&remote, &domain, args).await)
}

async fn reset_with(
    remote: &impl Remote,
    domain: &DomainRoot,
    Input { dataspace, package }: Input,
) -> Result<Output, Error> {
    let storage = LocalStorage::new();
    let writer = dataspace::writer(Some(&domain.local())).await?;
    let package = package.key(&dataspace, &writer).await?;
    let reset = Box::pin(reset_to_latest(
        &storage, remote, domain, &dataspace, &writer, &package,
    ))
    .await?;
    Ok(Output { package, reset })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use quilt_rs::dataspace::reset_to_latest::ResetError;
    use quilt_rs::io::remote::mocks::MockRemote;

    #[test(tokio::test)]
    async fn a_package_with_no_latest_is_not_reset() -> Result<(), Error> {
        let dir = tempfile::TempDir::new()?;
        let space = dir.path().join("space");
        std::fs::create_dir_all(&space)?;
        std::fs::write(
            space.join("data.toml"),
            "registry = \"s3://b\"\n[packages]\nsync = [\"team/raw\"]\n",
        )?;
        let input = Input {
            dataspace: space,
            package: Package::Key("quilt+s3://b#package=team/raw".parse()?),
        };

        let result = reset_with(
            &MockRemote::default(),
            &DomainRoot::new(dir.path().join("domain")),
            input,
        )
        .await;

        assert!(
            matches!(result, Err(Error::Reset(ResetError::NoLatest))),
            "{:?}",
            result.err()
        );
        Ok(())
    }
}
