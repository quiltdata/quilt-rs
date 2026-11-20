//! `quilt log --dataspace`: a package's revisions, newest first, from its
//! registry, with the local commits not pushed yet marked. Writes nothing.

use std::path::PathBuf;

use quilt_rs::dataspace::contract::PackageKey;
use quilt_rs::dataspace::history::Revision;
use quilt_rs::dataspace::history::history;
use quilt_rs::domain::DomainRoot;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::paths::DomainPaths;

use crate::cli::Error;
use crate::cli::dataspace::reader;
use crate::cli::output::Render;
use crate::cli::output::Std;
use crate::cli::text::printable_line;

#[derive(Debug)]
pub struct Input {
    pub dataspace: PathBuf,
    pub package: PackageKey,
}

pub struct Output {
    package: PackageKey,
    revisions: Vec<Revision>,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.revisions.is_empty() {
            return write!(f, "{}: no revisions", self.package);
        }
        write!(f, "revision  time (UTC)           message")?;
        for revision in &self.revisions {
            let short: String = revision.hash.chars().take(8).collect();
            let time = revision.time.map_or_else(
                || format!("{:<19}", "\u{2205}"),
                |time| time.strftime("%Y-%m-%d %H:%M:%S").to_string(),
            );
            let message = revision
                .message
                .as_deref()
                .map_or_else(|| "\u{2205}".to_string(), printable_line);
            let local = if revision.local {
                "  (local, not pushed)"
            } else {
                ""
            };
            write!(f, "\n{short}  {time}  {message}{local}")?;
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let revisions: Vec<_> = self
            .revisions
            .iter()
            .map(|revision| {
                serde_json::json!({
                    "hash": revision.hash,
                    "message": revision.message,
                    "time": revision.time.map(|time| time.to_string()),
                    "parent": revision.parent,
                    "local": revision.local,
                })
            })
            .collect();
        serde_json::json!({
            "package": self.package.to_string(),
            "revisions": revisions,
        })
    }
}

pub async fn command(credentials: PathBuf, domain: Option<DomainRoot>, args: Input) -> Std {
    let remote = RemoteS3::new(DomainPaths::new(credentials), LocalStorage::new());
    Std::from_result(history_with(&remote, domain.as_ref(), args).await)
}

async fn history_with(
    remote: &(impl Remote + Sync),
    domain: Option<&DomainRoot>,
    Input { dataspace, package }: Input,
) -> Result<Output, Error> {
    let writer = reader(domain).await?;
    let revisions = history(
        &LocalStorage::new(),
        remote,
        domain,
        &dataspace,
        &writer,
        &package,
    )
    .await?;
    Ok(Output { package, revisions })
}

#[cfg(test)]
mod tests {
    use super::*;

    use quilt_rs::io::remote::mocks::MockRemote;
    use test_log::test;

    use crate::cli::status::dataspace::tests::FIRST;
    use crate::cli::status::dataspace::tests::filled;
    use crate::cli::status::dataspace::tests::seed;

    const SECOND: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    /// The registry's revisions, newest first, each with its message and
    /// time where the Domain or the listing has them.
    #[test(tokio::test)]
    async fn a_package_log_lists_its_registry_revisions_newest_first() -> Result<(), Error> {
        let remote = MockRemote::default();
        let dataspace = filled(&remote).await?;
        seed(&remote, SECOND, &[("a.txt", b"v2")], 1_700_000_060).await?;
        let input = Input {
            dataspace: dataspace.path().to_path_buf(),
            package: "quilt+s3://b#package=team/raw".parse()?,
        };

        let output = history_with(&remote, None, input).await?;

        let json = output.to_json();
        assert_eq!(json["revisions"][0]["hash"], SECOND);
        assert_eq!(json["revisions"][0]["time"], "2023-11-14T22:14:20Z");
        assert_eq!(json["revisions"][1]["hash"], FIRST);
        assert_eq!(json["revisions"][1]["local"], false);
        let text = output.to_string();
        assert!(text.contains("\n22222222  2023-11-14 22:14:20  "), "{text}");
        Ok(())
    }
}
