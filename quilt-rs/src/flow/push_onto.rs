use tracing::info;
use tracing::warn;

use crate::Error;
use crate::Res;
use crate::error::PackageOpError;
use crate::installed_package::PushOutcome;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::manifest::resolve_tag;
use crate::io::manifest::tag_latest;
use crate::io::manifest::tag_timestamp;
use crate::io::manifest::upload_manifest;
use crate::io::remote::Remote;
use crate::io::remote::entry_view;
use crate::io::remote::validate_workflow_against_current_config;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::paths::DomainPaths;
use quilt_uri::ManifestUri;
use quilt_uri::Tag;

/// Publish `manifest`, whose rows are already on the remote, as the revision
/// after `parent`, and tag it `latest` only if `latest` still names `parent`
/// when read just before the tag write.
pub async fn push_onto(
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    parent: &ManifestUri,
    manifest: Manifest,
) -> Res<PushOutcome> {
    if let Some(row) = manifest
        .rows
        .iter()
        .find(|row| !row.physical_key.starts_with("s3://"))
    {
        return Err(Error::PackageOp(PackageOpError::Push(format!(
            "{} is not on the remote",
            row.logical_key.display()
        ))));
    }
    let mut sorted: Vec<&ManifestRow> = manifest.rows.iter().collect();
    sorted.sort_by(|a, b| a.logical_key.cmp(&b.logical_key));
    let entries: Vec<_> = sorted.into_iter().map(entry_view).collect();
    validate_workflow_against_current_config(
        remote,
        parent.origin.as_ref(),
        &parent.bucket,
        &parent.namespace.to_string(),
        manifest.header.message(),
        manifest.header.user_meta.as_ref(),
        manifest.header.workflow.as_ref(),
        &entries,
    )
    .await?;

    let stream = Box::pin(manifest.records_stream().await);
    let (cache_path, top_hash) = build_manifest_from_rows_stream(
        storage,
        paths.cached_manifests_dir(&parent.bucket),
        manifest.header.clone(),
        stream,
    )
    .await?;
    let manifest_uri = ManifestUri {
        hash: top_hash,
        ..parent.clone()
    };
    upload_manifest(storage, remote, &manifest_uri, &cache_path).await?;
    tag_timestamp(remote, &manifest_uri, chrono::Utc::now()).await?;

    let latest = resolve_tag(remote, parent.origin.as_ref(), parent.clone(), Tag::Latest).await?;
    let certified_latest = latest.hash == parent.hash;
    if certified_latest {
        tag_latest(remote, &manifest_uri).await?;
        info!("✔️ Certified {} as latest", manifest_uri.display());
    } else {
        warn!(
            "Pushed {} but did not certify latest: latest moved past {}",
            manifest_uri.display(),
            parent.hash
        );
    }
    Ok(PushOutcome {
        manifest_uri,
        certified_latest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::manifest::ManifestHeader;
    use quilt_uri::S3Uri;

    const LATEST: &str = "s3://b/.quilt/named_packages/a/c/latest";
    const PARENT: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const SOMEONE_ELSE: &str = "5555555555555555555555555555555555555555555555555555555555555555";

    fn parent() -> ManifestUri {
        ManifestUri {
            bucket: "b".to_string(),
            namespace: ("a", "c").into(),
            hash: PARENT.to_string(),
            origin: None,
        }
    }

    fn manifest(physical_key: &str) -> Res<Manifest> {
        Ok(Manifest {
            header: ManifestHeader {
                message: Some(Some("Task TASK.md done".to_string())),
                ..ManifestHeader::default()
            },
            rows: vec![ManifestRow {
                logical_key: crate::LogicalKey::new("out.md")?,
                physical_key: physical_key.to_string(),
                hash: crate::object_hash::Sha256ChunkedHash::try_from(
                    "/UMjH1bsbrMLBKdd9cqGGvtjhWzawhz1BfrxgngUhVI=",
                )
                .expect("hash")
                .into(),
                size: 29,
                meta: None,
            }],
        })
    }

    async fn remote_with_latest(hash: &str) -> Res<MockRemote> {
        let remote = MockRemote::default();
        remote
            .put_object(None, &S3Uri::try_from(LATEST)?, hash.as_bytes().to_vec())
            .await?;
        Ok(remote)
    }

    async fn read(remote: &MockRemote, uri: &str) -> Res<String> {
        let stream = remote
            .get_object_stream(None, &S3Uri::try_from(uri)?)
            .await?;
        Ok(String::from_utf8(stream.body.collect().await?.to_vec()).expect("utf-8"))
    }

    async fn uploaded(remote: &MockRemote, pushed: &PushOutcome) -> Res<bool> {
        let manifest = format!("s3://b/.quilt/packages/{}", pushed.manifest_uri.hash);
        remote
            .exists(None, &S3Uri::try_from(manifest.as_str())?)
            .await
    }

    async fn push(remote: &MockRemote, manifest: Manifest) -> Res<PushOutcome> {
        let paths = DomainPaths::new(std::path::absolute("/foo")?);
        push_onto(&paths, &MockStorage::default(), remote, &parent(), manifest).await
    }

    #[test(tokio::test)]
    async fn a_revision_onto_the_latest_it_read_becomes_latest() -> Res {
        let remote = remote_with_latest(PARENT).await?;

        let pushed = push(&remote, manifest("s3://b/a/c/out.md?versionId=v1")?).await?;

        assert!(pushed.certified_latest);
        assert_eq!(read(&remote, LATEST).await?, pushed.manifest_uri.hash);
        assert!(uploaded(&remote, &pushed).await?);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_latest_that_moved_past_the_parent_is_kept() -> Res {
        let remote = remote_with_latest(SOMEONE_ELSE).await?;

        let pushed = push(&remote, manifest("s3://b/a/c/out.md?versionId=v1")?).await?;

        assert!(!pushed.certified_latest);
        assert_eq!(read(&remote, LATEST).await?, SOMEONE_ELSE);
        assert!(uploaded(&remote, &pushed).await?, "the revision is kept");
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_row_not_on_the_remote_publishes_nothing() -> Res {
        let remote = remote_with_latest(PARENT).await?;

        let pushed = push(&remote, manifest("file:///foo/objects/out.md")?).await;

        assert!(
            matches!(pushed, Err(Error::PackageOp(PackageOpError::Push(_)))),
            "{pushed:?}"
        );
        assert_eq!(read(&remote, LATEST).await?, PARENT);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_revision_its_workflow_rejects_publishes_nothing() -> Res {
        let remote = remote_with_latest(PARENT).await?;
        remote
            .put_object(
                None,
                &S3Uri::try_from("s3://b/.quilt/workflows/config.yml")?,
                b"version: \"1\"\nworkflows:\n  gate:\n    name: Gate\n    metadata_schema: meta\nschemas:\n  meta:\n    url: s3://b/schemas/meta.json\n".to_vec(),
            )
            .await?;
        remote
            .put_object(
                None,
                &S3Uri::try_from("s3://b/schemas/meta.json")?,
                br#"{"type": "object", "required": ["owner"]}"#.to_vec(),
            )
            .await?;
        let mut manifest = manifest("s3://b/a/c/out.md?versionId=v1")?;
        manifest.header.workflow = Some(crate::manifest::Workflow {
            config: "s3://b/.quilt/workflows/config.yml".parse()?,
            id: Some(crate::manifest::WorkflowId {
                id: "gate".to_string(),
                schemas: std::collections::BTreeMap::new(),
            }),
        });

        let pushed = push(&remote, manifest).await;

        assert!(
            matches!(pushed, Err(Error::WorkflowValidation(_))),
            "expected a workflow rejection, got: {pushed:?}"
        );
        assert_eq!(read(&remote, LATEST).await?, PARENT);
        Ok(())
    }
}
