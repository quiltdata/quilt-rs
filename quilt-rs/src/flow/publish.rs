use std::path::PathBuf;

use tracing::debug;
use tracing::info;

use crate::error::PackageOpError;
use crate::flow;
use crate::flow::push::PushResult;
use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::InstalledPackageStatus;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::manifest::Workflow;
use crate::paths::DomainPaths;
use crate::uri::Namespace;
use crate::Error;
use crate::Res;

/// Options passed to the commit half of [`publish_package`].
///
/// All fields are already resolved by the caller (template rendered,
/// metadata parsed, workflow looked up) — the library does not know
/// about templates or UI state.
pub struct CommitOptions {
    pub message: String,
    pub user_meta: Option<serde_json::Value>,
    pub workflow: Option<Workflow>,
}

/// Result of a successful publish — one variant per branch of the
/// three-state decision tree (the "nothing to do" branch returns `Err`).
///
/// Generic over the push payload: the flow layer returns
/// `PublishOutcome<PushResult>`; the public API (`InstalledPackage::publish`)
/// maps it to `PublishOutcome<PushOutcome>` via the
/// `quilt::PublishOutcome` type alias.
#[derive(Debug)]
pub enum PublishOutcome<P> {
    /// Committed pending changes, then pushed the new revision.
    CommittedAndPushed(P),
    /// Pushed a previously-committed revision without a new commit
    /// (working directory had no changes).
    PushedOnly(P),
}

impl<P> PublishOutcome<P> {
    pub fn push(&self) -> &P {
        match self {
            Self::CommittedAndPushed(p) | Self::PushedOnly(p) => p,
        }
    }
}

/// Commit any pending working-directory changes and then push the resulting
/// revision to the remote in one step.
///
/// Behavior matches the three-state decision in the plan:
///
/// - `status.changes` non-empty → commit + push
/// - `status.changes` empty but `lineage.commit` exists → push only
/// - neither changes nor a pending commit → focused error (nothing to do)
#[allow(clippy::too_many_arguments)]
pub async fn publish_package(
    lineage: PackageLineage,
    manifest: &mut Manifest,
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    working_dir: PathBuf,
    status: InstalledPackageStatus,
    namespace: Namespace,
    host_config: HostConfig,
    commit_opts: CommitOptions,
) -> Res<PublishOutcome<PushResult>> {
    let has_changes = !status.changes.is_empty();
    let has_pending_commit = lineage.commit.is_some();

    if !has_changes && !has_pending_commit {
        return Err(Error::PackageOp(PackageOpError::Publish(
            "Nothing to publish".to_string(),
        )));
    }

    let (lineage, push_manifest, committed) = if has_changes {
        debug!("⏳ Publish: committing local changes");
        let lineage = flow::commit(
            lineage,
            manifest,
            paths,
            storage,
            working_dir,
            status,
            namespace.clone(),
            commit_opts.message,
            commit_opts.user_meta,
            commit_opts.workflow,
        )
        .await?;
        // commit wrote a new manifest to disk; reload it so push uploads
        // the new rows, not the pre-commit manifest we were handed.
        let new_hash = lineage.commit.as_ref().ok_or_else(|| {
            Error::PackageOp(PackageOpError::Publish(
                "commit succeeded but lineage has no commit".to_string(),
            ))
        })?;
        let committed_path = paths.installed_manifest(&namespace, &new_hash.hash);
        let committed_manifest = Manifest::from_path(storage, &committed_path).await?;
        debug!("✔️ Publish: commit done");
        (lineage, committed_manifest, true)
    } else {
        debug!("✔️ Publish: no changes, skipping commit");
        (lineage, manifest.clone(), false)
    };

    info!("⏳ Publish: pushing revision");
    let push = flow::push(
        lineage,
        push_manifest,
        paths,
        storage,
        remote,
        Some(namespace),
        host_config,
    )
    .await?;
    info!("✔️ Publish: push done");

    Ok(if committed {
        PublishOutcome::CommittedAndPushed(push)
    } else {
        PublishOutcome::PushedOnly(push)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use aws_sdk_s3::primitives::ByteStream;
    use test_log::test;

    use crate::fixtures;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::Change;
    use crate::lineage::CommitState;
    use crate::lineage::PathState;
    use crate::manifest::ManifestRow;
    use crate::uri::ManifestUri;
    use crate::uri::S3Uri;

    fn manifest_uri(hash: &str) -> ManifestUri {
        ManifestUri {
            bucket: "b".to_string(),
            namespace: ("foo", "bar").into(),
            hash: hash.to_string(),
            origin: None,
        }
    }

    fn first_push_uri() -> ManifestUri {
        // Empty hash triggers the "first push" branch in push_package, so
        // no remote manifest fetch is required to run the round trip.
        manifest_uri("")
    }

    async fn seed_remote_latest(remote: &MockRemote, latest_hash: &str) -> Res {
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/named_packages/foo/bar/latest")?,
                latest_hash.as_bytes().to_vec(),
            )
            .await?;
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_publish_nothing_to_do() -> Res {
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        let err = publish_package(
            PackageLineage::default(),
            &mut Manifest::default(),
            &DomainPaths::default(),
            &storage,
            &remote,
            PathBuf::default(),
            InstalledPackageStatus::default(),
            ("foo", "bar").into(),
            HostConfig::default(),
            CommitOptions {
                message: String::new(),
                user_meta: None,
                workflow: None,
            },
        )
        .await
        .unwrap_err();
        assert_eq!(
            err.to_string(),
            "Publish error: Nothing to publish".to_string()
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_publish_skips_commit_when_no_changes() -> Res {
        // Package with a pending local commit, first push. No new
        // working-dir changes — commit should be skipped.
        let hash = fixtures::top_hash::EMPTY_NULL_TOP_HASH.to_string();
        let lineage = PackageLineage {
            commit: Some(CommitState {
                timestamp: chrono::Utc::now(),
                hash: hash.clone(),
                prev_hashes: Vec::new(),
            }),
            remote_uri: Some(first_push_uri()),
            ..PackageLineage::default()
        };

        let storage = MockStorage::default();
        storage
            .write_byte_stream(
                PathBuf::from(format!(".quilt/packages/b/{hash}")),
                ByteStream::from_static(b"foo"),
            )
            .await?;

        let remote = MockRemote::default();
        seed_remote_latest(&remote, &hash).await?;

        let mut manifest = Manifest::default();
        manifest.header.user_meta = Some(serde_json::Value::Null);

        let outcome = publish_package(
            lineage,
            &mut manifest,
            &DomainPaths::default(),
            &storage,
            &remote,
            PathBuf::default(),
            InstalledPackageStatus::default(),
            ("foo", "bar").into(),
            HostConfig::default(),
            CommitOptions {
                message: String::new(),
                user_meta: None,
                workflow: None,
            },
        )
        .await?;

        let push = match &outcome {
            PublishOutcome::PushedOnly(p) => p,
            PublishOutcome::CommittedAndPushed(_) => {
                panic!("should skip commit when no changes");
            }
        };
        assert!(push.certified_latest);
        assert_eq!(push.lineage.remote()?.hash, hash);
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_publish_push_fails_without_remote() -> Res {
        // Commit succeeds, but lineage has no remote — push bails out.
        let manifest_src = fixtures::manifest_with_objects_all_sizes::manifest().await?;
        let base_record = manifest_src.get_record(&PathBuf::from("0mb.bin")).unwrap();
        let added = ManifestRow {
            logical_key: PathBuf::from("foo"),
            hash: base_record.hash.clone(),
            size: base_record.size,
            physical_key: base_record.physical_key.clone(),
            ..ManifestRow::default()
        };

        let storage = MockStorage::default();
        storage
            .write_byte_stream(PathBuf::from("/working-dir/foo"), ByteStream::default())
            .await?;

        let status = InstalledPackageStatus {
            changes: BTreeMap::from([(PathBuf::from("foo"), Change::Added(added))]),
            ..InstalledPackageStatus::default()
        };

        let lineage = PackageLineage {
            paths: BTreeMap::from([(PathBuf::from("foo"), PathState::default())]),
            ..PackageLineage::default()
        };

        let remote = MockRemote::default();

        let mut manifest = Manifest::default();

        let err = publish_package(
            lineage,
            &mut manifest,
            &DomainPaths::new(PathBuf::from("/")),
            &storage,
            &remote,
            PathBuf::from("/working-dir"),
            status,
            ("foo", "bar").into(),
            HostConfig::default(),
            CommitOptions {
                message: "published".to_string(),
                user_meta: None,
                workflow: None,
            },
        )
        .await
        .unwrap_err();
        assert!(
            err.to_string().contains("remote"),
            "expected remote-missing error, got: {err}"
        );
        Ok(())
    }
}
