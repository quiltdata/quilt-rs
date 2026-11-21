//! `quilt commit --dataspace`: a `sync` entry's working folder recorded as a
//! local commit in the Domain and appended to the lock, for the next sync to
//! push.

use std::path::Path;
use std::path::PathBuf;

use quilt_rs::dataspace::commit::Committed;
use quilt_rs::dataspace::commit::commit;
use quilt_rs::dataspace::contract::PackageKey;
use quilt_rs::dataspace::lock::LOCK_FILE;
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
    pub message: String,
}

pub struct Output {
    package: PackageKey,
    committed: Committed,
}

impl Output {
    fn commits(&self) -> &[String] {
        self.committed
            .locked
            .as_ref()
            .map_or(&[], |locked| locked.commits.as_slice())
    }
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let package = &self.package;
        let Some(commit) = &self.committed.commit else {
            return write!(f, "{package}: nothing to commit");
        };
        write!(f, "{package}: committed {commit}")?;
        let pending = self.commits().len();
        if pending > 0 {
            let plural = if pending == 1 { "" } else { "s" };
            write!(
                f,
                "; {LOCK_FILE} names {pending} local commit{plural} for the next sync to push"
            )?;
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "package": self.package.to_string(),
            "commit": self.committed.commit,
            "base": self.committed.locked.as_ref().and_then(|locked| locked.base.clone()),
            "commits": self.commits(),
        })
    }
}

/// Commits with `domain`, which keeps the commit, and the credentials and
/// user-wide ignore rules the `credentials` folder keeps.
pub async fn command(credentials: PathBuf, domain: DomainRoot, args: Input) -> Std {
    let paths = DomainPaths::new(credentials);
    let user_rules = paths.user_quiltignore();
    let remote = RemoteS3::new(paths, LocalStorage::new());
    Std::from_result(commit_with(&remote, &domain, args, &user_rules).await)
}

async fn commit_with(
    remote: &impl Remote,
    domain: &DomainRoot,
    Input {
        dataspace,
        package,
        message,
    }: Input,
    user_rules: &Path,
) -> Result<Output, Error> {
    let storage = LocalStorage::new();
    let local = domain.local();
    let writer = dataspace::writer(Some(&local)).await?;
    let package = package.key(&dataspace, &writer).await?;
    let committed = Box::pin(commit(
        &storage, remote, domain, &dataspace, &writer, &package, &message, user_rules,
    ))
    .await?;
    Ok(Output { package, committed })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use quilt_rs::io::remote::mocks::MockRemote;
    use quilt_uri::Namespace;

    #[test(tokio::test)]
    async fn a_commit_of_a_package_never_published_is_locked_with_no_base() -> Result<(), Error> {
        let dir = tempfile::TempDir::new()?;
        let space = dir.path().join("space");
        std::fs::create_dir_all(space.join("team").join("raw"))?;
        std::fs::write(
            space.join("data.toml"),
            "registry = \"s3://b\"\n[packages]\nsync = [\"team/raw\"]\n",
        )?;
        std::fs::write(space.join("team").join("raw").join("a.txt"), b"alpha")?;
        let domain = DomainRoot::new(dir.path().join("domain"));
        let input = Input {
            dataspace: space.clone(),
            package: Package::Namespace(Namespace::try_from("team/raw")?),
            message: "first".to_string(),
        };

        let output = commit_with(
            &MockRemote::default(),
            &domain,
            input,
            &dir.path().join("no-rules"),
        )
        .await?;

        let commit = output
            .committed
            .commit
            .clone()
            .ok_or(Error::Test("no commit".into()))?;
        assert_eq!(
            output.to_string(),
            format!(
                "quilt+s3://b#package=team/raw: committed {commit}; data.lock.toml names 1 local commit for the next sync to push"
            )
        );
        let json = output.to_json();
        assert_eq!(json["base"], serde_json::Value::Null);
        assert_eq!(json["commits"], serde_json::json!([commit]));
        let lock = std::fs::read_to_string(space.join(LOCK_FILE))?;
        assert!(lock.contains(&commit), "{lock}");
        Ok(())
    }
}
