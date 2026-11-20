//! `quilt gc --dataspace`: what each bucket's Package Store in the Domain
//! holds that its bucket holds too, cleared; with `--dry-run`, counted.

use std::path::PathBuf;

use quilt_rs::dataspace::gc::Collected;
use quilt_rs::dataspace::gc::StoreGc;
use quilt_rs::dataspace::gc::gc;
use quilt_rs::dataspace::gc::gc_estimate;
use quilt_rs::domain::DomainRoot;
use quilt_rs::flow::format_bytes;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::paths::DomainPaths;
use quilt_rs::settings;
use quilt_rs::settings::SharedSettings;
use quilt_uri::Host;

use crate::cli::Error;
use crate::cli::output::Render;
use crate::cli::output::Std;
use crate::cli::text::printable;

#[derive(Debug)]
pub struct Input {
    /// Count what would be cleared, clearing nothing.
    pub dry_run: bool,
}

pub struct Output {
    collected: Collected,
    dry_run: bool,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let total = self.collected.total();
        if self.dry_run {
            if total.is_empty() {
                write!(f, "Nothing to free")?;
            } else {
                write!(f, "Would free {}", format_bytes(total.bytes))?;
            }
        } else {
            write!(f, "{total}")?;
        }
        for (bucket, store) in &self.collected.stores {
            match store {
                StoreGc::Freed(report) if !report.is_empty() => {
                    write!(f, "\n  {bucket}: {}", format_bytes(report.bytes))?;
                }
                StoreGc::Busy(namespace) => write!(
                    f,
                    "\n  {bucket}: left as it is; {namespace} is busy in another quilt process"
                )?,
                StoreGc::Failed(err) => write!(
                    f,
                    "\n  {bucket}: left as it is; the bucket could not be asked: {}",
                    printable(&err.to_string())
                )?,
                _ => {}
            }
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let total = self.collected.total();
        let stores: serde_json::Map<String, serde_json::Value> = self
            .collected
            .stores
            .iter()
            .map(|(bucket, store)| {
                let value = match store {
                    StoreGc::Freed(report) => serde_json::json!({
                        "manifests": report.cached_manifests,
                        "objects": report.objects,
                        "bytes": report.bytes,
                    }),
                    StoreGc::Busy(namespace) => {
                        serde_json::json!({ "busy": namespace.to_string() })
                    }
                    StoreGc::Failed(err) => serde_json::json!({ "error": err.to_string() }),
                    _ => serde_json::json!({}),
                };
                (bucket.clone(), value)
            })
            .collect();
        serde_json::json!({
            "dry_run": self.dry_run,
            "manifests": total.cached_manifests,
            "objects": total.objects,
            "bytes": total.bytes,
            "stores": stores,
        })
    }

    /// A store left as it was is work not done.
    fn completed(&self) -> bool {
        !self.collected.incomplete()
    }
}

/// Clears in `domain`, where one exists; a machine with no Domain has
/// nothing to clear. Each bucket is asked through the default catalog, as
/// `login` and `role` use it, or this machine's AWS credentials where none
/// is set.
pub async fn command(credentials: PathBuf, domain: Option<DomainRoot>, args: Input) -> Std {
    let remote = RemoteS3::new(DomainPaths::new(credentials), LocalStorage::new());
    let catalog = match settings::shared_dir() {
        Some(dir) => SharedSettings::new(LocalStorage::new(), dir)
            .default_catalog()
            .await
            .ok()
            .flatten(),
        None => None,
    };
    Std::from_result(gc_with(&remote, domain.as_ref(), catalog, args).await)
}

async fn gc_with(
    remote: &impl Remote,
    domain: Option<&DomainRoot>,
    catalog: Option<Host>,
    Input { dry_run }: Input,
) -> Result<Output, Error> {
    let Some(domain) = domain else {
        return Ok(Output {
            collected: Collected::default(),
            dry_run,
        });
    };
    let storage = LocalStorage::new();
    let host = |_: &str| catalog.clone();
    let collected = if dry_run {
        Box::pin(gc_estimate(&storage, remote, domain, host)).await?
    } else {
        Box::pin(gc(&storage, remote, domain, host)).await?
    };
    Ok(Output { collected, dry_run })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use quilt_rs::io::remote::mocks::MockRemote;

    /// With no Domain there is nothing to clear, and saying so is no
    /// failure.
    #[test(tokio::test)]
    async fn no_domain_frees_nothing() -> Result<(), Error> {
        let output = gc_with(&MockRemote::default(), None, None, Input { dry_run: false }).await?;

        assert_eq!(output.to_string(), "Nothing to free");
        assert!(output.completed());
        assert_eq!(output.to_json()["bytes"], 0);
        Ok(())
    }

    /// A store whose bucket cannot be asked is named, left as it is, and
    /// makes the command fail.
    #[test(tokio::test)]
    async fn a_store_left_as_it_is_says_why() -> Result<(), Error> {
        let dir = tempfile::TempDir::new()?;
        let domain = DomainRoot::new(dir.path());
        let installed = domain
            .registry("b")?
            .installed_manifest(&("acme", "demo").into(), &"1".repeat(64));
        std::fs::create_dir_all(installed.parent().expect("a parent"))?;
        std::fs::write(&installed, b"{}\n")?;
        let remote = MockRemote::default();
        remote.deny_reads(&format!("s3://b/.quilt/packages/{}", "1".repeat(64)));

        let output = gc_with(&remote, Some(&domain), None, Input { dry_run: true }).await?;

        assert!(!output.completed());
        assert!(
            output
                .to_string()
                .contains("b: left as it is; the bucket could not be asked"),
            "{output}"
        );
        assert!(output.to_json()["stores"]["b"]["error"].is_string());
        assert!(installed.exists());
        Ok(())
    }
}
