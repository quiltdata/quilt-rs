//! `quilt create --dataspace`: a new package declared in a dataspace's
//! `data.toml`, with its folder's files as its first commit, which the next
//! sync publishes.

use std::path::Path;
use std::path::PathBuf;

use quilt_rs::dataspace::contract::Access;
use quilt_rs::dataspace::contract::CONTRACT_FILE;
use quilt_rs::dataspace::create::Create;
use quilt_rs::dataspace::create::Created;
use quilt_rs::dataspace::create::create;
use quilt_rs::domain::DomainRoot;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::paths::DomainPaths;

use crate::cli::Error;
use crate::cli::dataspace;
use crate::cli::output::Render;
use crate::cli::output::Std;
use crate::cli::text::printable_line;

#[derive(Debug)]
pub struct Input {
    /// The dataspace: the folder holding `data.toml`.
    pub dataspace: PathBuf,
    pub uri: String,
    pub access: Access,
    /// The working folder: an existing folder to package, relative to the
    /// dataspace unless absolute.
    pub folder: Option<PathBuf>,
    pub message: Option<String>,
}

pub struct Output {
    created: Created,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let entry = &self.created.entry;
        let package = entry.key();
        let access = entry.access;
        let folder = printable_line(&entry.folder.display().to_string());
        write!(
            f,
            "{package}: added to {CONTRACT_FILE} under `{access}`, in {folder}"
        )?;
        match &self.created.commit {
            Some(commit) => write!(
                f,
                "\n  committed {commit}, with no base; the next sync publishes it"
            ),
            None if access == Access::Sync => write!(
                f,
                "\n  nothing to commit yet; the next sync commits what the folder holds"
            ),
            None => write!(
                f,
                "\n  `{access}` entries publish their folder, not commits"
            ),
        }
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let entry = &self.created.entry;
        serde_json::json!({
            "package": entry.key().to_string(),
            "access": entry.access.to_string(),
            "folder": entry.folder.display().to_string(),
            "commit": self.created.commit,
        })
    }
}

/// Creates in `domain`, which it makes where it is not there: the commit
/// is kept in its store, or in its local registry. The `credentials`
/// folder supplies the catalog logins and the user-wide ignore rules.
pub async fn command(credentials: PathBuf, domain: DomainRoot, args: Input) -> Std {
    let paths = DomainPaths::new(credentials);
    let user_rules = paths.user_quiltignore();
    let remote = RemoteS3::new(paths, LocalStorage::new());
    Std::from_result(create_with(&remote, &domain, args, &user_rules).await)
}

async fn create_with(
    remote: &impl Remote,
    domain: &DomainRoot,
    args: Input,
    user_rules: &Path,
) -> Result<Output, Error> {
    let local = domain.local();
    let writer = dataspace::writer(Some(&local)).await?;
    let mut request = Create::new(args.uri);
    request.access = args.access;
    request.folder = args.folder;
    request.message = args.message;
    let created = Box::pin(create(
        &LocalStorage::new(),
        remote,
        domain,
        &args.dataspace,
        &writer,
        &request,
        user_rules,
    ))
    .await?;
    Ok(Output { created })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use quilt_rs::dataspace::lock::LOCK_FILE;
    use quilt_rs::io::remote::mocks::MockRemote;

    /// A package created from a folder outside the dataspace is declared
    /// there, and its files are its first commit.
    #[test(tokio::test)]
    async fn create_declares_the_package_and_commits_its_folder() -> Result<(), Error> {
        let dir = tempfile::TempDir::new()?;
        let dataspace = dir.path().join("work");
        std::fs::create_dir_all(&dataspace)?;
        let folder = dir.path().join("analysis");
        std::fs::create_dir_all(&folder)?;
        std::fs::write(folder.join("a.txt"), b"alpha")?;
        let domain = DomainRoot::new(dir.path().join("domain"));
        let input = Input {
            dataspace: dataspace.clone(),
            uri: "quilt+s3://b#package=me/analysis".to_string(),
            access: Access::Sync,
            folder: Some(folder.clone()),
            message: Some("first".to_string()),
        };

        let output = create_with(
            &MockRemote::default(),
            &domain,
            input,
            &dir.path().join("no-rules"),
        )
        .await?;

        let json = output.to_json();
        assert_eq!(json["package"], "quilt+s3://b#package=me/analysis");
        assert_eq!(json["folder"], folder.display().to_string());
        assert!(json["commit"].is_string(), "{json}");
        assert!(output.to_string().contains("with no base"), "{output}");
        assert!(dataspace.join(LOCK_FILE).exists());
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_package_already_declared_is_an_error() -> Result<(), Error> {
        let dir = tempfile::TempDir::new()?;
        let dataspace = dir.path().join("work");
        std::fs::create_dir_all(&dataspace)?;
        std::fs::write(
            dataspace.join("data.toml"),
            "registry = \"s3://b\"\n[packages]\nsync = [\"me/analysis\"]\n",
        )?;
        let input = Input {
            dataspace,
            uri: "quilt+s3://b#package=me/analysis".to_string(),
            access: Access::Sync,
            folder: None,
            message: None,
        };

        let std = Std::from_result(
            create_with(
                &MockRemote::default(),
                &DomainRoot::new(dir.path().join("domain")),
                input,
                &dir.path().join("no-rules"),
            )
            .await,
        );

        let Std::Err(err) = std else {
            panic!("expected an error, got {std:?}");
        };
        assert_eq!(err.kind(), "already_declared");
        Ok(())
    }
}
