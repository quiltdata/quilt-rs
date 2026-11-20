use std::path::PathBuf;

use quilt_rs::dataspace::contract::CONTRACT_FILE;
use quilt_rs::dataspace::lock::Writer;
use quilt_rs::dataspace::publish::Published;
use quilt_rs::dataspace::publish_all::DataspacePublish;
use quilt_rs::dataspace::publish_all::EntryPublish;
use quilt_rs::dataspace::publish_all::PublishAllError;
use quilt_rs::dataspace::publish_all::publish_all;
use quilt_rs::dataspace::read_all::EntryRead;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::paths::DomainPaths;

use crate::cli::Error;
use crate::cli::output::Render;
use crate::cli::output::Std;
use crate::cli::read;
use crate::cli::text::printable;
use crate::cli::text::printable_line;

#[derive(Debug)]
pub struct Input {
    pub dataspace: PathBuf,
}

pub struct Output {
    dataspace: PathBuf,
    published: DataspacePublish,
}

/// How an entry's publish ended, as `--json` names it.
fn outcome(published: &EntryPublish) -> &'static str {
    match published {
        EntryPublish::Ran(Published::Advanced { .. }) => "published",
        EntryPublish::Ran(Published::Stored { .. }) => "stored",
        EntryPublish::Ran(Published::Unchanged { .. }) => "unchanged",
        EntryPublish::Ran(Published::Behind { .. }) => "behind",
        EntryPublish::NotRead => "not_read",
        EntryPublish::Unpushed => "unpushed",
        EntryPublish::Failed(_) => "failed",
        _ => "not_published",
    }
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.published.entries.is_empty() {
            return write!(
                f,
                "No `write` or `replace` entries in {}",
                printable_line(&self.dataspace.join(CONTRACT_FILE).display().to_string())
            );
        }
        for (index, entry) in self.published.entries.iter().enumerate() {
            if index > 0 {
                writeln!(f)?;
            }
            let package = entry.entry.key();
            let folder = printable_line(&entry.entry.folder.display().to_string());
            match &entry.published {
                EntryPublish::Ran(Published::Advanced {
                    revision,
                    latest_unchecked,
                }) => {
                    write!(f, "{package}: published {revision} from {folder}")?;
                    if *latest_unchecked {
                        write!(f, "; `latest` was moved without checking its base")?;
                    }
                }
                EntryPublish::Ran(Published::Stored { revision, latest }) => write!(
                    f,
                    "{package}: stored {revision}, which `latest` does not name; it names {}",
                    latest.as_deref().unwrap_or("another revision")
                )?,
                EntryPublish::Ran(Published::Unchanged { .. }) => {
                    write!(f, "{package}: nothing to publish from {folder}")?;
                }
                EntryPublish::Ran(Published::Behind { latest }) => write!(
                    f,
                    "{package}: not published; `latest` moved to {} after the read, so publish again",
                    latest.as_deref().unwrap_or("no revision")
                )?,
                EntryPublish::NotRead => write!(
                    f,
                    "{package}: not published; it is also under `read`, and its read did not complete"
                )?,
                EntryPublish::Unpushed => write!(
                    f,
                    "{package}: not published; the lock names local commits not yet pushed"
                )?,
                EntryPublish::Failed(err) => {
                    write!(f, "{package}: failed: {}", printable(&err.to_string()))?;
                }
                _ => write!(f, "{package}: not published")?,
            }
            let Some(first) = &entry.read else {
                continue;
            };
            match (read::revision(first), first) {
                (Some(revision), _) if first.completed() => {
                    write!(f, "\n  read {revision} first")?;
                }
                (Some(revision), _) => {
                    write!(f, "\n  read {revision} first: {}", read::outcome(first))?;
                }
                (None, EntryRead::Failed(err)) => {
                    write!(f, "\n  read first: failed: {}", printable(&err.to_string()))?;
                }
                (None, _) => write!(f, "\n  read first: {}", read::outcome(first))?,
            }
            for (_, heading, paths) in read::lists(first) {
                read::group(f, heading, paths)?;
            }
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let entries: Vec<serde_json::Value> = self
            .published
            .entries
            .iter()
            .map(|entry| {
                let published = &entry.published;
                let (revision, latest, unchecked) = match published {
                    EntryPublish::Ran(Published::Advanced {
                        revision,
                        latest_unchecked,
                    }) => (Some(revision), Some(revision), *latest_unchecked),
                    EntryPublish::Ran(Published::Stored { revision, latest }) => {
                        (Some(revision), latest.as_ref(), false)
                    }
                    EntryPublish::Ran(
                        Published::Unchanged { latest } | Published::Behind { latest },
                    ) => (None, latest.as_ref(), false),
                    _ => (None, None, false),
                };
                let error = match published {
                    EntryPublish::Failed(err) => Some(err.to_string()),
                    _ => None,
                };
                serde_json::json!({
                    "package": entry.entry.key().to_string(),
                    "folder": entry.entry.folder.display().to_string(),
                    "access": entry.entry.access.to_string(),
                    "outcome": outcome(published),
                    "completed": published.completed(),
                    "revision": revision,
                    "latest": latest,
                    "latest_unchecked": unchecked,
                    "read": entry.read.as_ref().map(read::read_json),
                    "error": error,
                })
            })
            .collect();
        serde_json::json!({
            "dataspace": self.dataspace.display().to_string(),
            "completed": self.published.completed(),
            "entries": entries,
        })
    }

    fn completed(&self) -> bool {
        self.published.completed()
    }
}

pub async fn command(domain: PathBuf, args: Input) -> Std {
    Std::from_result(model(domain, args).await)
}

/// Publishes with no Domain: the domain folder supplies only the credentials
/// a catalog's login keeps there, and the user-wide ignore rules.
pub async fn model(domain: PathBuf, args: Input) -> Result<Output, Error> {
    let paths = DomainPaths::new(domain);
    let user_rules = paths.user_quiltignore();
    let remote = RemoteS3::new(paths, LocalStorage::new());
    publish_with(&remote, args, &user_rules).await
}

async fn publish_with(
    remote: &impl Remote,
    Input { dataspace }: Input,
    user_rules: &std::path::Path,
) -> Result<Output, Error> {
    let writer = Writer::without_domain().map_err(PublishAllError::from)?;
    let published = Box::pin(publish_all(
        &LocalStorage::new(),
        remote,
        &dataspace,
        &writer,
        user_rules,
    ))
    .await?;
    Ok(Output {
        dataspace,
        published,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::Path;

    use test_log::test;

    use quilt_rs::io::remote::mocks::MockRemote;
    use quilt_rs::manifest::Manifest;
    use quilt_rs::manifest::ManifestRow;
    use quilt_uri::S3Uri;

    const FIRST: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const SECOND: &str = "2222222222222222222222222222222222222222222222222222222222222222";

    /// Stores `files` as revision `hash` of `team/raw` in bucket `b`, and
    /// tags it `latest`.
    async fn seed(remote: &MockRemote, hash: &str, files: &[(&str, &[u8])]) -> Result<(), Error> {
        let mut rows = Vec::new();
        for (key, body) in files {
            let physical_key = format!("s3://b/team/raw/{key}?versionId={hash}");
            remote
                .put_object(
                    None,
                    &S3Uri::try_from(physical_key.as_str())?,
                    body.to_vec(),
                )
                .await?;
            rows.push(ManifestRow {
                logical_key: quilt_rs::LogicalKey::new(*key)?,
                physical_key,
                hash: quilt_rs::checksum::hash_reader(*body, body.len() as u64, 0x12).await?,
                size: body.len() as u64,
                ..ManifestRow::default()
            });
        }
        let manifest = Manifest {
            rows,
            ..Manifest::default()
        };
        let manifest_uri = format!("s3://b/.quilt/packages/{hash}");
        remote
            .put_object(
                None,
                &S3Uri::try_from(manifest_uri.as_str())?,
                manifest.to_jsonlines().into_bytes(),
            )
            .await?;
        remote
            .put_object(
                None,
                &S3Uri::try_from("s3://b/.quilt/named_packages/team/raw/latest")?,
                hash.as_bytes().to_vec(),
            )
            .await?;
        Ok(())
    }

    fn dataspace(packages: &str) -> Result<tempfile::TempDir, Error> {
        let dataspace = tempfile::TempDir::new()?;
        std::fs::write(
            dataspace.path().join("data.toml"),
            format!("registry = \"s3://b\"\n\n[packages]\n{packages}"),
        )?;
        Ok(dataspace)
    }

    fn write(dataspace: &Path, key: &str, body: &[u8]) -> Result<(), Error> {
        let path = dataspace.join("team/raw").join(key);
        std::fs::create_dir_all(path.parent().expect("a folder"))?;
        std::fs::write(path, body)?;
        Ok(())
    }

    async fn publish(remote: &MockRemote, dataspace: &Path) -> Std {
        let input = Input {
            dataspace: dataspace.to_path_buf(),
        };
        let no_user_rules = dataspace.join("no-user-rules");
        Std::from_result(publish_with(remote, input, &no_user_rules).await)
    }

    async fn read(remote: &MockRemote, dataspace: &Path) -> Result<(), Error> {
        let writer = Writer::without_domain().map_err(PublishAllError::from)?;
        let storage = LocalStorage::new();
        quilt_rs::dataspace::read_all::read_all(&storage, remote, dataspace, &writer)
            .await
            .map_err(|err| Error::Test(err.to_string()))?;
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_dataspace_published_whole_reports_each_package_and_succeeds() -> Result<(), Error> {
        let remote = MockRemote::default();
        seed(&remote, FIRST, &[("a.txt", b"alpha")]).await?;
        let dataspace = dataspace("write = [\"team/raw\"]\n")?;
        write(dataspace.path(), "new.txt", b"new")?;

        let std = publish(&remote, dataspace.path()).await;

        assert!(!std.failed(), "{std:?}");
        let Std::Out(output) = std else {
            panic!("expected a success, got {std:?}");
        };
        let json = output.to_json();
        assert_eq!(json["completed"], true);
        let entry = &json["entries"][0];
        assert_eq!(entry["package"], "quilt+s3://b#package=team/raw");
        assert_eq!(entry["access"], "write");
        assert_eq!(entry["outcome"], "published");
        assert_eq!(entry["read"], serde_json::Value::Null);
        let revision = entry["revision"].as_str().expect("a revision");
        assert_eq!(entry["latest"], revision);
        assert!(output.to_string().starts_with(&format!(
            "quilt+s3://b#package=team/raw: published {revision} from "
        )));
        let lock = std::fs::read_to_string(dataspace.path().join("data.lock.toml"))?;
        assert!(lock.contains(revision), "{lock}");
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_package_read_first_reports_its_read() -> Result<(), Error> {
        let remote = MockRemote::default();
        seed(&remote, FIRST, &[("in/a.txt", b"alpha")]).await?;
        let dataspace = dataspace(
            "read = [{ package = \"team/raw\", path = \"in/\" }]\nwrite = [{ package = \"team/raw\", path = \"out/\" }]\n",
        )?;
        read(&remote, dataspace.path()).await?;
        seed(&remote, SECOND, &[("in/a.txt", b"ALPHA")]).await?;
        write(dataspace.path(), "out/b.txt", b"beta")?;

        let std = publish(&remote, dataspace.path()).await;

        assert!(!std.failed(), "{std:?}");
        let Std::Out(output) = std else {
            panic!("expected a success, got {std:?}");
        };
        let entry = &output.to_json()["entries"][0];
        assert_eq!(entry["outcome"], "published");
        assert_eq!(entry["read"]["outcome"], "read");
        assert_eq!(entry["read"]["revision"], SECOND);
        assert_eq!(entry["read"]["placed"], serde_json::json!(["in/a.txt"]));
        assert!(
            output.to_string().contains(&format!(
                "\n  read {SECOND} first\n  1 file placed:\n    in/a.txt"
            )),
            "{output}"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_package_whose_read_first_is_blocked_is_reported_and_fails() -> Result<(), Error> {
        let remote = MockRemote::default();
        seed(&remote, FIRST, &[("in/a.txt", b"alpha")]).await?;
        let dataspace = dataspace(
            "read = [{ package = \"team/raw\", path = \"in/\" }]\nwrite = [{ package = \"team/raw\", path = \"out/\" }]\n",
        )?;
        read(&remote, dataspace.path()).await?;
        seed(&remote, SECOND, &[("in/a.txt", b"ALPHA")]).await?;
        write(dataspace.path(), "in/a.txt", b"edited")?;
        write(dataspace.path(), "out/b.txt", b"beta")?;

        let std = publish(&remote, dataspace.path()).await;

        assert!(std.failed());
        let Std::Out(output) = std else {
            panic!("expected the report, got {std:?}");
        };
        let entry = &output.to_json()["entries"][0];
        assert_eq!(entry["outcome"], "not_read");
        assert_eq!(entry["read"]["outcome"], "blocked");
        assert_eq!(entry["read"]["conflicts"], serde_json::json!(["in/a.txt"]));
        assert_eq!(
            output.to_string(),
            format!(
                "quilt+s3://b#package=team/raw: not published; it is also under `read`, and its read did not complete\n  read {SECOND} first: blocked\n  1 file in both:\n    in/a.txt"
            )
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn live_write_publish_leaves_out_what_a_commit_would() -> Result<(), Error> {
        let name = format!(
            "publish_{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .expect("clock")
                .as_nanos()
        );
        let domain = tempfile::TempDir::new()?;
        let dataspace = tempfile::TempDir::new()?;
        std::fs::write(
            dataspace.path().join("data.toml"),
            format!(
                "registry = \"s3://fiskus-us-east-1\"\n\n[packages]\nwrite = [\"cli_test/{name}\"]\n"
            ),
        )?;
        let folder = dataspace.path().join("cli_test").join(&name);
        std::fs::create_dir_all(&folder)?;
        std::fs::write(folder.join("data.txt"), b"hello from a dataspace\n")?;
        std::fs::write(folder.join(".env"), b"SECRET=1\n")?;

        let std = model(
            domain.path().to_path_buf(),
            Input {
                dataspace: dataspace.path().to_path_buf(),
            },
        )
        .await;

        let json = std?.to_json();
        assert_eq!(json["entries"][0]["outcome"], "published", "{json}");
        let revision = &json["entries"][0]["revision"];
        let reader = tempfile::TempDir::new()?;
        std::fs::write(
            reader.path().join("data.toml"),
            format!(
                "[packages]\nread = [{{ package = \"cli_test/{name}\", registry = \"s3://fiskus-us-east-1\", revision = {revision} }}]\n"
            ),
        )?;
        let input = read::Input {
            dataspace: reader.path().to_path_buf(),
        };
        let read = read::model(domain.path().to_path_buf(), input)
            .await?
            .to_json();
        assert_eq!(read["entries"][0]["revision"], *revision, "{read}");
        assert_eq!(
            read["entries"][0]["placed"],
            serde_json::json!(["data.txt", "metadata.toml"])
        );
        Ok(())
    }
}
