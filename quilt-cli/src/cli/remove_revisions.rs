use std::collections::BTreeSet;
use std::num::NonZeroUsize;

use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Render;
use crate::cli::output::Std;

use quilt_rs::InstalledPackage;
use quilt_rs::flow::HistoryEntry;
use quilt_rs::flow::RemovalReport;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::storage::Storage;
use quilt_rs::lineage::PackageLineage;
use quilt_uri::Namespace;

#[derive(Debug)]
pub struct Input {
    pub namespace: Namespace,
    /// Remove only this many of the oldest removable revisions; all of them
    /// when `None`.
    pub count: Option<NonZeroUsize>,
}

pub struct Output {
    namespace: Namespace,
    /// The removed revisions' hashes, oldest first.
    removed: Vec<String>,
    /// What the removal did, or `None` when nothing was removable.
    report: Option<RemovalReport>,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.report {
            Some(report) => write!(f, "{report}"),
            None => write!(f, "No old revisions of {} to remove", self.namespace),
        }
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let (revisions, objects, bytes, kept_for, kept_bytes) = match &self.report {
            Some(report) => (
                report.revisions,
                report.objects,
                report.bytes,
                report.kept_for.as_ref().map(ToString::to_string),
                report.kept_bytes,
            ),
            None => (0, 0, 0, None, 0),
        };
        serde_json::json!({
            "namespace": self.namespace.to_string(),
            "removed": self.removed,
            "revisions": revisions,
            "objects": objects,
            "bytes": bytes,
            "kept_for": kept_for,
            "kept_bytes": kept_bytes,
        })
    }
}

/// `--count`'s parser: a whole number of at least 1, so `--count 0` is an
/// argument error rather than a silent no-op.
pub fn parse_count(value: &str) -> Result<NonZeroUsize, String> {
    match value.parse::<usize>() {
        Ok(count) => NonZeroUsize::new(count).ok_or_else(|| "must be at least 1".to_string()),
        Err(_) => Err("must be a whole number of at least 1".to_string()),
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.remove_revisions(args).await)
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { namespace, count }: Input,
) -> Result<Output, Error> {
    let Some(package) = local_domain.get_installed_package(&namespace).await? else {
        return Err(Error::NamespaceNotFound(namespace));
    };
    remove_old(&package, count).await
}

/// The removable revisions of `history`, oldest first, cut to the `count`
/// oldest. `history` is newest first, as
/// [`InstalledPackage::revision_history`] lists it; removable is what
/// [`quilt_rs::flow::protection`] does not keep.
pub fn removable_oldest_first(
    lineage: &PackageLineage,
    history: &[HistoryEntry],
    count: Option<NonZeroUsize>,
) -> Vec<String> {
    let kept = quilt_rs::flow::protection(lineage, history);
    history
        .iter()
        .rev()
        .map(|entry| &entry.revision.hash)
        .filter(|hash| !kept.contains_key(*hash))
        .take(count.map_or(usize::MAX, NonZeroUsize::get))
        .cloned()
        .collect()
}

/// Remove `package`'s oldest removable revisions. The removal checks their
/// protection again under the package's lock, so a revision that became
/// protected meanwhile refuses it rather than going.
pub async fn remove_old<S: Storage + Clone + Sync, R: Remote>(
    package: &InstalledPackage<S, R>,
    count: Option<NonZeroUsize>,
) -> Result<Output, Error> {
    let namespace = package.namespace.clone();
    let lineage = package.lineage().await?;
    let history = package.revision_history(&lineage).await?;
    let removed = removable_oldest_first(&lineage, &history, count);
    if removed.is_empty() {
        return Ok(Output {
            namespace,
            removed,
            report: None,
        });
    }
    let hashes: BTreeSet<String> = removed.iter().cloned().collect();
    match package.remove_revisions(&hashes).await {
        Ok(report) => Ok(Output {
            namespace,
            removed,
            report: Some(report),
        }),
        Err(quilt_rs::Error::PackageBusy(busy)) => Err(Error::PackageBusy(busy)),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;
    use std::time::Duration;
    use std::time::SystemTime;

    use tempfile::TempDir;
    use test_log::test;

    use quilt_rs::io::remote::mocks::MockRemote;
    use quilt_rs::io::storage::LocalStorage;
    use quilt_rs::lineage::DomainLineageIo;
    use quilt_rs::lineage::Home;
    use quilt_rs::lineage::PackageLineageIo;
    use quilt_rs::paths::DomainPaths;
    use quilt_uri::paths::tag_key;

    use crate::cli::create;
    use crate::cli::model::create_model_in_temp_dir;

    /// The published revisions, oldest first. `current-rev` is current,
    /// latest and base, so it is kept even though it is published.
    const PUBLISHED: [&str; 4] = ["old-1", "old-2", "old-3", "current-rev"];

    /// `test/history`, its remote at `current-rev` on a catalog, holding
    /// `PUBLISHED` and an unpublished `local-rev`, each obtained a minute
    /// after the one before it in that order. The temp dirs must outlive the
    /// package.
    async fn package_with_old_revisions()
    -> Result<(InstalledPackage<LocalStorage, MockRemote>, [TempDir; 2]), Error> {
        let remote = MockRemote::default();
        let namespace: Namespace = ("test", "history").into();
        for (n, hash) in PUBLISHED.iter().enumerate() {
            remote
                .put_object(
                    None,
                    &format!(
                        "s3://bucket/{}",
                        tag_key(&namespace, &format!("175850000{n}"))
                    )
                    .parse()?,
                    hash.as_bytes().to_vec(),
                )
                .await?;
        }

        let home_dir = TempDir::new()?;
        let paths_dir = TempDir::new()?;
        let home = Home::new(home_dir.path().to_path_buf());
        let paths = DomainPaths::new(paths_dir.path().to_path_buf());
        let storage = LocalStorage::new();
        paths
            .scaffold_for_installing(&storage, &home, &namespace)
            .await?;
        let lineage_json = format!(
            r#"{{
                "packages": {{
                    "test/history": {{
                        "commit": null,
                        "remote": {{
                            "bucket": "bucket",
                            "namespace": "test/history",
                            "hash": "current-rev",
                            "origin": "quilt.test"
                        }},
                        "base_hash": "current-rev",
                        "latest_hash": "current-rev",
                        "paths": {{}}
                    }}
                }},
                "home": "{}"
            }}"#,
            home_dir.path().display()
        );
        storage
            .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
            .await?;

        let start = SystemTime::now() - Duration::from_secs(3600);
        for (n, hash) in PUBLISHED.iter().chain(&["local-rev"]).enumerate() {
            let path = paths.installed_manifest(&namespace, hash);
            std::fs::write(&path, r#"{"version":"v0"}"#)?;
            let minutes = u64::try_from(n).expect("a few revisions") * 60;
            std::fs::File::options()
                .write(true)
                .open(&path)?
                .set_modified(start + Duration::from_secs(minutes))?;
        }

        let package = InstalledPackage {
            lineage: PackageLineageIo::new(
                DomainLineageIo::new(paths.lineage()),
                namespace.clone(),
            ),
            paths,
            remote: Arc::new(remote),
            storage,
            namespace,
        };
        Ok((package, [home_dir, paths_dir]))
    }

    async fn held(
        package: &InstalledPackage<LocalStorage, MockRemote>,
    ) -> Result<Vec<String>, Error> {
        let mut held: Vec<String> = package
            .revisions()
            .await?
            .into_iter()
            .map(|revision| revision.hash)
            .collect();
        held.sort();
        Ok(held)
    }

    /// Without a count, every removable revision goes, and the kept ones stay:
    /// the current one and the unpublished one.
    #[test(tokio::test)]
    async fn removes_every_removable_revision_without_a_count() -> Result<(), Error> {
        let (package, _dirs) = package_with_old_revisions().await?;

        let output = remove_old(&package, None).await?;

        assert_eq!(output.removed, ["old-1", "old-2", "old-3"]);
        assert_eq!(
            output.to_string(),
            "Removed 3 old revisions of test/history \u{b7} freed no space"
        );
        assert_eq!(
            output.to_json(),
            serde_json::json!({
                "namespace": "test/history",
                "removed": ["old-1", "old-2", "old-3"],
                "revisions": 3,
                "objects": 0,
                "bytes": 0,
                "kept_for": null,
                "kept_bytes": 0,
            })
        );
        assert_eq!(held(&package).await?, ["current-rev", "local-rev"]);
        Ok(())
    }

    /// A count removes that many of the oldest removable revisions.
    #[test(tokio::test)]
    async fn a_count_removes_only_the_oldest() -> Result<(), Error> {
        let (package, _dirs) = package_with_old_revisions().await?;

        let output = remove_old(&package, NonZeroUsize::new(2)).await?;

        assert_eq!(output.removed, ["old-1", "old-2"]);
        assert_eq!(
            output.to_string(),
            "Removed 2 old revisions of test/history \u{b7} freed no space"
        );
        assert_eq!(held(&package).await?, ["current-rev", "local-rev", "old-3"]);
        Ok(())
    }

    /// A count past the removable ones removes what there is, and never a
    /// kept revision.
    #[test(tokio::test)]
    async fn a_count_past_the_removable_skips_the_kept() -> Result<(), Error> {
        let (package, _dirs) = package_with_old_revisions().await?;

        let output = remove_old(&package, NonZeroUsize::new(10)).await?;

        assert_eq!(output.removed, ["old-1", "old-2", "old-3"]);
        assert_eq!(held(&package).await?, ["current-rev", "local-rev"]);

        let again = remove_old(&package, NonZeroUsize::new(1)).await?;
        assert_eq!(
            again.to_string(),
            "No old revisions of test/history to remove"
        );
        Ok(())
    }

    /// A local-only package's revisions are all unpublished, so all kept:
    /// the command succeeds and says nothing was removable.
    #[test(tokio::test)]
    async fn nothing_to_remove_succeeds_and_says_so() -> Result<(), Error> {
        let (m, _temp_dir) = create_model_in_temp_dir().await?;
        let namespace: Namespace = ("test", "local").into();
        m.create(create::Input {
            namespace: namespace.clone(),
            source: None,
            message: None,
        })
        .await?;

        let output = m
            .remove_revisions(Input {
                namespace,
                count: None,
            })
            .await?;

        assert_eq!(
            output.to_string(),
            "No old revisions of test/local to remove"
        );
        assert_eq!(
            output.to_json(),
            serde_json::json!({
                "namespace": "test/local",
                "removed": [],
                "revisions": 0,
                "objects": 0,
                "bytes": 0,
                "kept_for": null,
                "kept_bytes": 0,
            })
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn an_uninstalled_package_is_not_found() -> Result<(), Error> {
        let (m, _temp_dir) = create_model_in_temp_dir().await?;

        let Err(err) = m
            .remove_revisions(Input {
                namespace: ("test", "absent").into(),
                count: None,
            })
            .await
        else {
            return Err(Error::Test("expected not found".to_string()));
        };
        assert_eq!(err.kind(), "namespace_not_found");
        Ok(())
    }

    #[test]
    fn a_count_is_at_least_one() {
        assert_eq!(parse_count("3"), Ok(NonZeroUsize::new(3).expect("3")));
        assert_eq!(parse_count("0"), Err("must be at least 1".to_string()));
        assert_eq!(
            parse_count("-1"),
            Err("must be a whole number of at least 1".to_string())
        );
    }
}
