use std::path::PathBuf;

use quilt_rs::dataspace::contract::PackageKey;
use quilt_rs::dataspace::lock::LOCK_FILE;
use quilt_rs::dataspace::lock::Writer;
use quilt_rs::dataspace::reattach::ReattachError;
use quilt_rs::dataspace::reattach::Unattached;
use quilt_rs::dataspace::reattach::reattach;
use quilt_rs::dataspace::reattach::unattached;
use quilt_rs::domain::DomainRoot;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::paths::DomainPaths;

use crate::cli::Error;
use crate::cli::output::Render;
use crate::cli::output::Std;
use crate::cli::text::printable_line;

#[derive(Debug)]
pub struct Input {
    pub dataspace: PathBuf,
    pub package: PackageKey,
    /// The commit to attach and the base it goes on; `None` lists.
    pub attach: Option<(String, String)>,
}

pub struct Output {
    package: PackageKey,
    done: Done,
}

enum Done {
    Listed(Vec<Unattached>),
    Attached { commit: String, base: String },
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let package = &self.package;
        let listed = match &self.done {
            Done::Listed(listed) => listed,
            Done::Attached { commit, base } => {
                return write!(
                    f,
                    "{package}: attached {commit} to {LOCK_FILE}, on base {base}"
                );
            }
        };
        if listed.is_empty() {
            return write!(f, "{package}: no local commits that {LOCK_FILE} leaves out");
        }
        let plural = if listed.len() == 1 { "" } else { "s" };
        write!(
            f,
            "{package}: {} local commit{plural} that {LOCK_FILE} leaves out:",
            listed.len()
        )?;
        for commit in listed {
            write!(f, "\n  {}", commit.hash)?;
            if let Some(message) = &commit.message {
                write!(f, "  {}", printable_line(message))?;
            }
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let (unattached, attached, base) = match &self.done {
            Done::Listed(listed) => {
                let listed: Vec<_> = listed
                    .iter()
                    .map(|commit| serde_json::json!({ "hash": commit.hash, "message": commit.message }))
                    .collect();
                (Some(listed), None, None)
            }
            Done::Attached { commit, base } => (None, Some(commit), Some(base)),
        };
        serde_json::json!({
            "package": self.package.to_string(),
            "unattached": unattached,
            "attached": attached,
            "base": base,
        })
    }
}

pub async fn command(credentials: PathBuf, domain: DomainRoot, args: Input) -> Std {
    let remote = RemoteS3::new(DomainPaths::new(credentials), LocalStorage::new());
    Std::from_result(reattach_with(&remote, &domain, args).await)
}

async fn reattach_with(
    remote: &impl Remote,
    domain: &DomainRoot,
    Input {
        dataspace,
        package,
        attach,
    }: Input,
) -> Result<Output, Error> {
    let storage = LocalStorage::new();
    let writer = Writer::here(String::new()).map_err(ReattachError::from)?;
    let done = match attach {
        None => {
            Done::Listed(unattached(&storage, remote, domain, &dataspace, &writer, &package).await?)
        }
        Some((commit, base)) => {
            reattach(
                &storage, remote, domain, &dataspace, &writer, &package, &commit, &base,
            )
            .await?;
            Done::Attached { commit, base }
        }
    };
    Ok(Output { package, done })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use test_log::test;

    use quilt_rs::LogicalKey;
    use quilt_rs::flow::WorkflowCheck;
    use quilt_rs::flow::record;
    use quilt_rs::io::manifest::RowContent;
    use quilt_rs::io::remote::mocks::MockRemote;
    use quilt_rs::lineage::PackageLineage;
    use quilt_rs::manifest::HeaderMessage;
    use quilt_rs::manifest::Manifest;
    use quilt_rs::manifest::ManifestHeader;
    use quilt_rs::manifest::ManifestRow;

    const BASE: &str = "1111111111111111111111111111111111111111111111111111111111111111";

    /// A dataspace syncing `team/raw` from bucket `b`, and a Domain holding
    /// one commit of it that no lock names; returns the commit's hash.
    async fn orphaned() -> Result<(tempfile::TempDir, DomainRoot, String), Error> {
        orphaned_in("b", ("team", "raw")).await
    }

    /// [`orphaned`], for `namespace` in `bucket`.
    async fn orphaned_in(
        bucket: &str,
        namespace: (&str, &str),
    ) -> Result<(tempfile::TempDir, DomainRoot, String), Error> {
        let dir = tempfile::TempDir::new()?;
        let (owner, name) = namespace;
        std::fs::write(
            dir.path().join("data.toml"),
            format!("registry = \"s3://{bucket}\"\n\n[packages]\nsync = [\"{owner}/{name}\"]\n"),
        )?;
        let domain = DomainRoot::new(dir.path().join("domain"));
        let logical_key = LogicalKey::new("a.txt")?;
        let manifest = Manifest {
            header: ManifestHeader {
                message: HeaderMessage::Text("mine".to_string()),
                ..ManifestHeader::default()
            },
            rows: vec![ManifestRow {
                logical_key: logical_key.clone(),
                ..ManifestRow::default()
            }],
            ..Manifest::default()
        };
        let files = BTreeMap::from([(logical_key, RowContent::Bytes(b"m".to_vec().into()))]);
        let (_, recorded) = record(
            &domain.registry(bucket)?,
            &LocalStorage::new(),
            &MockRemote::default(),
            WorkflowCheck::Warn { host: None },
            &namespace.into(),
            PackageLineage::default(),
            manifest,
            &files,
        )
        .await?;
        let hash = recorded.hash().expect("a revision").to_string();
        Ok((dir, domain, hash))
    }

    fn input(dir: &tempfile::TempDir, attach: Option<(&str, &str)>) -> Result<Input, Error> {
        Ok(Input {
            dataspace: dir.path().to_path_buf(),
            package: "quilt+s3://b#package=team/raw".parse()?,
            attach: attach.map(|(commit, base)| (commit.to_string(), base.to_string())),
        })
    }

    /// Runs against bucket `b` holding `BASE` alone.
    async fn run(domain: &DomainRoot, input: Input) -> Result<Std, Error> {
        let remote = MockRemote::default();
        let base = quilt_uri::S3Uri {
            bucket: "b".to_string(),
            key: quilt_uri::paths::get_manifest_key(BASE),
            version: None,
        };
        remote.put_object(None, &base, b"{}".to_vec()).await?;
        Ok(Std::from_result(
            reattach_with(&remote, domain, input).await,
        ))
    }

    #[test(tokio::test)]
    async fn a_commit_no_lock_names_is_listed_with_its_message() -> Result<(), Error> {
        let (dir, domain, hash) = orphaned().await?;

        let std = run(&domain, input(&dir, None)?).await?;

        let Std::Out(output) = std else {
            panic!("expected a listing, got {std:?}");
        };
        assert_eq!(
            output.to_json(),
            serde_json::json!({
                "package": "quilt+s3://b#package=team/raw",
                "unattached": [{ "hash": hash, "message": "mine" }],
                "attached": null,
                "base": null,
            })
        );
        assert_eq!(
            output.to_string(),
            format!(
                "quilt+s3://b#package=team/raw: 1 local commit that data.lock.toml leaves out:\n  {hash}  mine"
            )
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn an_attached_commit_is_reported_with_its_base() -> Result<(), Error> {
        let (dir, domain, hash) = orphaned().await?;

        let std = run(&domain, input(&dir, Some((&hash, BASE)))?).await?;

        let Std::Out(output) = std else {
            panic!("expected a success, got {std:?}");
        };
        assert_eq!(
            output.to_json(),
            serde_json::json!({
                "package": "quilt+s3://b#package=team/raw",
                "unattached": null,
                "attached": hash,
                "base": BASE,
            })
        );
        assert_eq!(
            output.to_string(),
            format!(
                "quilt+s3://b#package=team/raw: attached {hash} to data.lock.toml, on base {BASE}"
            )
        );
        let lock = std::fs::read_to_string(dir.path().join("data.lock.toml"))?;
        assert!(lock.contains(&hash) && lock.contains(BASE), "{lock}");
        Ok(())
    }

    /// Against the bucket itself: a commit it lacks is listed, and a revision
    /// it holds, kept beside the commit, is not.
    #[test(tokio::test)]
    async fn live_reattach_lists_only_what_the_bucket_lacks() -> Result<(), Error> {
        use crate::cli::fixtures::packages::default as pkg;
        let (dir, domain, hash) = orphaned_in(pkg::BUCKET, pkg::NAMESPACE).await?;
        let published = domain
            .registry(pkg::BUCKET)?
            .installed_manifest(&pkg::NAMESPACE.into(), pkg::TOP_HASH);
        std::fs::write(&published, b"{\"version\": \"v0\"}\n")?;
        let credentials = tempfile::TempDir::new()?;
        let input = Input {
            dataspace: dir.path().to_path_buf(),
            package: pkg::URI_LATEST.parse()?,
            attach: None,
        };

        let std = command(credentials.path().to_path_buf(), domain, input).await;

        let Std::Out(output) = std else {
            panic!("expected a listing, got {std:?}");
        };
        assert_eq!(
            output.to_json()["unattached"],
            serde_json::json!([{ "hash": hash, "message": "mine" }])
        );
        Ok(())
    }
}
