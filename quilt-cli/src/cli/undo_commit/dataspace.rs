//! `quilt undo-commit --dataspace`: a package's last local commit dropped
//! from the lock and its working folder restored to the revision before it;
//! the commit stays in the Domain.

use std::path::PathBuf;

use quilt_rs::dataspace::contract::PackageKey;
use quilt_rs::dataspace::lock::LOCK_FILE;
use quilt_rs::dataspace::undo_commit::Undone;
use quilt_rs::dataspace::undo_commit::undo_commit;
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
    undone: Undone,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { package, undone } = self;
        write!(
            f,
            "{package}: dropped {} from {LOCK_FILE}, and the Domain keeps it",
            undone.commit
        )?;
        match undone.locked.as_ref().and_then(|locked| locked.current()) {
            Some(current) => write!(f, "\n  the folder is at {current} again"),
            None => write!(f, "\n  no commit left, and never published"),
        }
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let locked = self.undone.locked.as_ref();
        let paths = |keys: &[PathBuf]| -> Vec<String> {
            keys.iter().map(|key| key.display().to_string()).collect()
        };
        serde_json::json!({
            "package": self.package.to_string(),
            "undone": self.undone.commit,
            "current": locked.and_then(|locked| locked.current()),
            "commits": locked.map_or(&[][..], |locked| locked.commits.as_slice()),
            "restored": paths(&self.undone.restored),
            "removed": paths(&self.undone.removed),
        })
    }
}

pub async fn command(credentials: PathBuf, domain: DomainRoot, args: Input) -> Std {
    let remote = RemoteS3::new(DomainPaths::new(credentials), LocalStorage::new());
    Std::from_result(undo_with(&remote, &domain, args).await)
}

async fn undo_with(
    remote: &impl Remote,
    domain: &DomainRoot,
    Input { dataspace, package }: Input,
) -> Result<Output, Error> {
    let storage = LocalStorage::new();
    let writer = dataspace::writer(Some(&domain.local())).await?;
    let package = package.key(&dataspace, &writer).await?;
    let undone = Box::pin(undo_commit(
        &storage, remote, domain, &dataspace, &writer, &package,
    ))
    .await?;
    Ok(Output { package, undone })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use quilt_rs::dataspace::undo_commit::UndoError;
    use quilt_rs::io::remote::mocks::MockRemote;

    #[test(tokio::test)]
    async fn a_package_with_no_commit_has_nothing_to_undo() -> Result<(), Error> {
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

        let result = undo_with(
            &MockRemote::default(),
            &DomainRoot::new(dir.path().join("domain")),
            input,
        )
        .await;

        assert!(
            matches!(result, Err(Error::Undo(UndoError::NothingToUndo(_)))),
            "{:?}",
            result.err()
        );
        Ok(())
    }
}
