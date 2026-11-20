//! `quilt certify --dataspace`: a package's `latest` forced to its current
//! revision, its last local commit pushed first; today's Promote.

use std::path::PathBuf;

use quilt_rs::dataspace::certify_latest::Certified;
use quilt_rs::dataspace::certify_latest::certify_latest;
use quilt_rs::dataspace::contract::PackageKey;
use quilt_rs::dataspace::publish::Published;
use quilt_rs::domain::DomainRoot;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::paths::DomainPaths;
use quilt_rs::queue::Queue;

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
    certified: Certified,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { package, certified } = self;
        let revision = &certified.revision;
        match &certified.previous {
            Some(previous) if previous == revision => {
                write!(f, "{package}: latest already names {revision}")
            }
            Some(previous) => write!(f, "{package}: latest names {revision}, not {previous}"),
            None => write!(f, "{package}: latest names {revision}"),
        }?;
        if let Some(Published::WorkflowBlocked { reason, .. }) = &certified.pushed {
            write!(
                f,
                "\n  the bucket's workflow refused it, and it was certified all the same: {reason}"
            )?;
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let workflow_refused = match &self.certified.pushed {
            Some(Published::WorkflowBlocked { reason, .. }) => Some(reason.as_str()),
            _ => None,
        };
        serde_json::json!({
            "package": self.package.to_string(),
            "revision": self.certified.revision,
            "previous": self.certified.previous,
            "pushed": self.certified.pushed.is_some(),
            "workflow_refused": workflow_refused,
        })
    }
}

pub async fn command(credentials: PathBuf, domain: DomainRoot, args: Input) -> Std {
    let remote = RemoteS3::new(DomainPaths::new(credentials), LocalStorage::new());
    Std::from_result(certify_with(&remote, &domain, args).await)
}

async fn certify_with(
    remote: &impl Remote,
    domain: &DomainRoot,
    Input { dataspace, package }: Input,
) -> Result<Output, Error> {
    let storage = LocalStorage::new();
    let writer = dataspace::writer(Some(&domain.local())).await?;
    let package = package.key(&dataspace, &writer).await?;
    std::fs::create_dir_all(domain.root())?;
    let queue = Queue::open(&domain.queue_db())
        .await
        .map_err(|err| Error::Io(std::io::Error::other(err.to_string())))?;
    let certified = Box::pin(certify_latest(
        &storage, remote, domain, &queue, &dataspace, &writer, &package,
    ))
    .await?;
    Ok(Output { package, certified })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use quilt_rs::dataspace::certify_latest::CertifyError;
    use quilt_rs::io::remote::mocks::MockRemote;

    #[test(tokio::test)]
    async fn a_package_with_no_revision_is_not_certified() -> Result<(), Error> {
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

        let result = certify_with(
            &MockRemote::default(),
            &DomainRoot::new(dir.path().join("domain")),
            input,
        )
        .await;

        assert!(
            matches!(
                result,
                Err(Error::Certify(CertifyError::NothingToCertify(_)))
            ),
            "{:?}",
            result.err()
        );
        Ok(())
    }
}
