//! Publish a revision whose entries are already in S3, with no local domain.
//!
//! For unattended writers (the lab-to-cloud agent) that land data themselves
//! and then name it. Unlike [`push_package`](super::push), this never moves
//! `latest` onto someone else's revision: it advances `latest` only while
//! `latest` is still the parent the caller built on.

use tracing::debug;
use tracing::info;

use crate::Res;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::manifest::tag_timestamp;
use crate::io::manifest::upload_manifest;
use crate::io::remote::PutCondition;
use crate::io::remote::Remote;
use crate::io::remote::entry_view;
use crate::io::remote::validate_workflow_against_current_config;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use quilt_uri::Host;
use quilt_uri::ManifestUri;
use quilt_uri::S3PackageHandle;
use quilt_uri::S3Uri;
use quilt_uri::TagUri;

/// What [`push_revision`] did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevisionPushed {
    /// The new revision's top hash.
    pub top_hash: String,
    /// True iff `latest` names `top_hash` when the push returns: this push
    /// moved it there, or it already was.
    pub latest_advanced: bool,
    /// What `latest` held when the push decided, if anything.
    pub latest_before: Option<String>,
}

/// Write `manifest` as a revision of `package` and record it in the
/// package's history. Then set `latest` to it if `latest` is absent or equals
/// `parent`; otherwise leave `latest` alone and report `latest_advanced: false`.
///
/// The bucket's workflow gate runs first, against its current config, exactly
/// as at [`push_package`](super::push): a revision the bucket would reject is
/// refused before anything is written. Resolve the header's workflow with
/// [`resolve_workflow`](crate::io::remote::resolve_workflow) to attach one.
///
/// Every row's `physical_key` must already point at uploaded bytes; nothing is
/// uploaded but the manifest and its tags. Re-running with the same manifest
/// is safe: the revision is content-addressed and the tags are rewritten with
/// the same value.
pub async fn push_revision(
    storage: &impl Storage,
    remote: &impl Remote,
    host: Option<&Host>,
    package: &S3PackageHandle,
    manifest: Manifest,
    parent: Option<&str>,
    timestamp: chrono::DateTime<chrono::Utc>,
) -> Res<RevisionPushed> {
    let mut sorted: Vec<&ManifestRow> = manifest.rows.iter().collect();
    sorted.sort_by(|a, b| a.logical_key.cmp(&b.logical_key));
    let entries: Vec<_> = sorted.into_iter().map(entry_view).collect();
    validate_workflow_against_current_config(
        remote,
        host,
        &package.bucket,
        &package.namespace.to_string(),
        manifest.header.message.as_deref(),
        manifest.header.user_meta.as_ref(),
        manifest.header.workflow.as_ref(),
        &entries,
    )
    .await?;

    let rows = manifest.rows.into_iter().map(Ok).collect::<Vec<_>>();
    let stream = Box::pin(tokio_stream::once(Ok(rows)));
    let dest_dir = tempfile::tempdir()?;
    let (path, top_hash) = build_manifest_from_rows_stream(
        storage,
        dest_dir.path().to_path_buf(),
        manifest.header,
        stream,
    )
    .await?;

    let manifest_uri = ManifestUri {
        bucket: package.bucket.clone(),
        namespace: package.namespace.clone(),
        hash: top_hash.clone(),
        origin: host.cloned(),
    };
    upload_manifest(storage, remote, &manifest_uri, &path).await?;
    tag_timestamp(remote, &manifest_uri, timestamp).await?;
    debug!("✔️ Revision {top_hash} recorded in history");

    let latest = S3Uri::from(TagUri::latest(&manifest_uri));
    let current = remote.get_object_with_etag(host, &latest).await?;
    let etag = current.as_ref().map(|(etag, _)| etag.clone());
    let latest_before = current.map(|(_, body)| String::from_utf8_lossy(&body).trim().to_string());

    let condition = match (&etag, &latest_before) {
        (None, _) => Some(PutCondition::Absent),
        (Some(_), Some(current)) if current == &top_hash => None,
        (Some(etag), Some(current)) if Some(current.as_str()) == parent => {
            Some(PutCondition::ETag(etag.clone()))
        }
        _ => {
            info!("latest moved past the parent; {top_hash} recorded without advancing latest");
            return Ok(RevisionPushed {
                top_hash,
                latest_advanced: false,
                latest_before,
            });
        }
    };
    let Some(condition) = condition else {
        return Ok(RevisionPushed {
            top_hash,
            latest_advanced: true,
            latest_before,
        });
    };

    match remote
        .put_object_if(host, &latest, top_hash.as_bytes().to_vec(), condition)
        .await
    {
        Ok(()) => Ok(RevisionPushed {
            top_hash,
            latest_advanced: true,
            latest_before,
        }),
        Err(e) if e.is_precondition_failed() => {
            info!(
                "another writer moved latest first; {top_hash} recorded without advancing latest"
            );
            Ok(RevisionPushed {
                top_hash,
                latest_advanced: false,
                latest_before,
            })
        }
        Err(e) => Err(e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::manifest::ManifestHeader;

    const BUCKET: &str = "b";

    fn manifest(name: &str) -> Manifest {
        Manifest {
            header: ManifestHeader::default(),
            rows: vec![ManifestRow {
                logical_key: name.into(),
                physical_key: format!("s3://{BUCKET}/landing/{name}?versionId=v1"),
                size: 3,
                ..ManifestRow::default()
            }],
        }
    }

    fn latest_uri() -> S3Uri {
        S3Uri {
            bucket: BUCKET.to_string(),
            key: ".quilt/named_packages/lab/run/latest".to_string(),
            version: None,
        }
    }

    async fn push(remote: &MockRemote, m: Manifest, parent: Option<&str>) -> Res<RevisionPushed> {
        push_revision(
            &MockStorage::default(),
            remote,
            None,
            &S3PackageHandle {
                bucket: BUCKET.to_string(),
                namespace: ("lab", "run").into(),
            },
            m,
            parent,
            chrono::Utc::now(),
        )
        .await
    }

    async fn latest(remote: &MockRemote) -> Res<String> {
        let (_, body) = remote
            .get_object_with_etag(None, &latest_uri())
            .await?
            .expect("latest exists");
        Ok(String::from_utf8(body).expect("utf8"))
    }

    #[test(tokio::test)]
    async fn first_revision_becomes_latest() -> Res {
        let remote = MockRemote::default();
        let pushed = push(&remote, manifest("a"), None).await?;
        assert!(pushed.latest_advanced);
        assert_eq!(pushed.latest_before, None);
        assert_eq!(latest(&remote).await?, pushed.top_hash);
        Ok(())
    }

    #[test(tokio::test)]
    async fn advances_from_the_named_parent() -> Res {
        let remote = MockRemote::default();
        let first = push(&remote, manifest("a"), None).await?;
        let second = push(&remote, manifest("b"), Some(&first.top_hash)).await?;
        assert!(second.latest_advanced);
        assert_eq!(latest(&remote).await?, second.top_hash);
        Ok(())
    }

    /// The case `push_package` gets wrong for an unattended writer: a fresh
    /// lineage must not move a `latest` someone else set.
    #[test(tokio::test)]
    async fn never_overwrites_another_writers_latest() -> Res {
        let remote = MockRemote::default();
        let theirs = push(&remote, manifest("scientist"), None).await?;
        let ours = push(&remote, manifest("agent"), None).await?;
        assert!(!ours.latest_advanced);
        assert_eq!(
            ours.latest_before.as_deref(),
            Some(theirs.top_hash.as_str())
        );
        assert_eq!(latest(&remote).await?, theirs.top_hash);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_stale_parent_does_not_advance() -> Res {
        let remote = MockRemote::default();
        let first = push(&remote, manifest("a"), None).await?;
        let _ = push(&remote, manifest("b"), Some(&first.top_hash)).await?;
        let stale = push(&remote, manifest("c"), Some(&first.top_hash)).await?;
        assert!(!stale.latest_advanced);
        Ok(())
    }

    /// A bucket that requires a workflow refuses a revision without one,
    /// before anything is written: the same gate `push_package` applies.
    #[test(tokio::test)]
    async fn a_required_workflow_refuses_before_writing() -> Res {
        let remote = MockRemote::default();
        remote
            .put_object(
                None,
                &S3Uri::try_from("s3://b/.quilt/workflows/config.yml")?,
                b"version: \"1\"\nis_workflow_required: true\nworkflows:\n  gate:\n    name: Gate\n".to_vec(),
            )
            .await?;
        let err = push(&remote, manifest("a"), None).await.unwrap_err();
        assert!(
            matches!(err, crate::Error::WorkflowValidation(_)),
            "expected a workflow refusal, got: {err:?}"
        );
        assert!(
            remote
                .get_object_with_etag(None, &latest_uri())
                .await?
                .is_none()
        );
        Ok(())
    }

    /// Another writer moves `latest` between our read and our write: the
    /// conditional put loses, and the push reports it rather than failing.
    #[test(tokio::test)]
    async fn losing_the_race_at_put_time_reports_not_advanced() -> Res {
        let remote = MockRemote::default();
        let first = push(&remote, manifest("a"), None).await?;
        let gate = remote.park(&latest_uri().to_string());
        let ours = push(&remote, manifest("b"), Some(&first.top_hash));
        let theirs = async {
            gate.arrived().await;
            remote
                .put_object(None, &latest_uri(), b"theirs".to_vec())
                .await
                .unwrap();
            gate.release();
        };
        let (ours, ()) = tokio::join!(ours, theirs);
        assert!(!ours?.latest_advanced);
        assert_eq!(latest(&remote).await?, "theirs");
        Ok(())
    }

    #[test(tokio::test)]
    async fn replay_is_idempotent() -> Res {
        let remote = MockRemote::default();
        let first = push(&remote, manifest("a"), None).await?;
        let again = push(&remote, manifest("a"), None).await?;
        assert_eq!(again.top_hash, first.top_hash);
        assert!(again.latest_advanced);
        assert_eq!(latest(&remote).await?, first.top_hash);
        Ok(())
    }
}
