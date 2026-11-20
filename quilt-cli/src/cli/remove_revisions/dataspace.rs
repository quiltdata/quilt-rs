//! `quilt remove-revisions --dataspace`: revisions of a package taken out of
//! its registry, their tags and manifests, never the data, as quilt3's
//! `delete_package` with a registry takes them.

use std::path::PathBuf;

use quilt_rs::dataspace::contract::PackageKey;
use quilt_rs::dataspace::lock::LOCK_FILE;
use quilt_rs::dataspace::remove_revisions::LatestAfter;
use quilt_rs::dataspace::remove_revisions::Removed;
use quilt_rs::dataspace::remove_revisions::remove_revisions;
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
    /// Each a whole top hash, or a prefix of only one.
    pub revisions: Vec<String>,
}

pub struct Output {
    package: PackageKey,
    removed: Removed,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let Self { package, removed } = self;
        let count = removed.revisions.len();
        let plural = if count == 1 { "" } else { "s" };
        write!(
            f,
            "{package}: removed {count} revision{plural} from the registry; their data stays"
        )?;
        for revision in &removed.revisions {
            write!(f, "\n  {revision}")?;
        }
        match &removed.latest {
            LatestAfter::Moved(hash) => write!(f, "\nlatest now names {hash}")?,
            LatestAfter::Removed => write!(f, "\nno revision is left, so latest was removed")?,
            _ => {}
        }
        if !removed.locked.is_empty() {
            write!(
                f,
                "\n{LOCK_FILE} still names a removed revision; `quilt reset` reads latest in its place"
            )?;
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let latest = match &self.removed.latest {
            LatestAfter::Moved(hash) => serde_json::json!({ "moved": hash }),
            LatestAfter::Removed => serde_json::json!({ "removed": true }),
            _ => serde_json::json!({ "kept": true }),
        };
        serde_json::json!({
            "package": self.package.to_string(),
            "removed": self.removed.revisions,
            "latest": latest,
            "locked": self.removed.locked,
        })
    }
}

pub async fn command(credentials: PathBuf, domain: Option<DomainRoot>, args: Input) -> Std {
    let remote = RemoteS3::new(DomainPaths::new(credentials), LocalStorage::new());
    Std::from_result(remove_with(&remote, domain.as_ref(), args).await)
}

async fn remove_with(
    remote: &impl Remote,
    domain: Option<&DomainRoot>,
    Input {
        dataspace,
        package,
        revisions,
    }: Input,
) -> Result<Output, Error> {
    let storage = LocalStorage::new();
    let local = domain.map(DomainRoot::local);
    let writer = dataspace::writer(local.as_ref()).await?;
    let package = package.key(&dataspace, &writer).await?;
    let removed = remove_revisions(
        &storage, remote, domain, &dataspace, &writer, &package, &revisions,
    )
    .await?;
    Ok(Output { package, removed })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use quilt_rs::io::remote::mocks::MockRemote;
    use quilt_uri::S3Uri;

    const FIRST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const SECOND: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    #[test(tokio::test)]
    async fn removing_latest_moves_it_and_says_so() -> Result<(), Error> {
        let dir = tempfile::TempDir::new()?;
        std::fs::write(
            dir.path().join("data.toml"),
            "registry = \"s3://b\"\n[packages]\nsync = [\"team/raw\"]\n",
        )?;
        let remote = MockRemote::default();
        for (name, hash) in [("1000", FIRST), ("2000", SECOND), ("latest", SECOND)] {
            let tag = format!("s3://b/.quilt/named_packages/team/raw/{name}");
            remote
                .put_object(
                    None,
                    &S3Uri::try_from(tag.as_str())?,
                    hash.as_bytes().to_vec(),
                )
                .await?;
        }
        let input = Input {
            dataspace: dir.path().to_path_buf(),
            package: Package::Key("quilt+s3://b#package=team/raw".parse()?),
            revisions: vec!["2222".to_string()],
        };

        let output = remove_with(&remote, None, input).await?;

        assert_eq!(
            output.to_string(),
            format!(
                "quilt+s3://b#package=team/raw: removed 1 revision from the registry; their data stays\n  {SECOND}\nlatest now names {FIRST}"
            )
        );
        assert_eq!(output.to_json()["latest"]["moved"], FIRST);
        let latest = remote
            .current("s3://b/.quilt/named_packages/team/raw/latest")
            .await?;
        assert_eq!(latest.as_deref(), Some(FIRST.as_bytes()));
        Ok(())
    }
}
