use std::collections::BTreeMap;

use tracing::info;
use tracing::warn;
use url::Url;

use crate::Error;
use crate::LogicalKey;
use crate::Res;
use crate::error::PackageOpError;
use crate::installed_package::PushOutcome;
use crate::io::manifest::RowContent;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::manifest::resolve_tag;
use crate::io::manifest::tag_latest;
use crate::io::manifest::tag_timestamp;
use crate::io::manifest::upload_manifest;
use crate::io::manifest::upload_row;
use crate::io::manifest::upload_row_content;
use crate::io::remote::Remote;
use crate::io::remote::entry_view;
use crate::io::remote::validate_workflow_against_current_config;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use quilt_uri::Host;
use quilt_uri::ManifestUri;
use quilt_uri::S3PackageHandle;
use quilt_uri::Tag;

/// Where [`push_files`] publishes, and the revision the caller built on.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PushTarget {
    /// The bucket and package name to publish under.
    pub handle: S3PackageHandle,
    /// The catalog host that vends credentials for the bucket, if any.
    pub origin: Option<Host>,
    /// The top hash of the revision the manifest was built on, or `None` for
    /// the package's first revision.
    pub parent: Option<String>,
}

/// Publishes `manifest` as a new revision of `target`, with no package home,
/// lineage or working folder: the rows' bytes are whatever the caller hands in.
///
/// Where each row's bytes come from, in order:
///
/// 1. `files`, keyed by logical key: a file anywhere on disk, or the bytes
///    themselves ([`RowContent`]). The row's own `physical_key`, `size` and
///    `hash` are ignored; S3's checksum becomes the row's hash.
/// 2. A `file://` `physical_key`, uploaded as `push` does.
/// 3. An `s3://` `physical_key`, kept where it is: the bytes are already on the
///    remote, and nothing is copied.
///
/// Any other row is refused before anything is uploaded, and so is a key in
/// `files` that the manifest has no row for. The revision is checked against its
/// workflow before the first upload, as in [`push_onto`](super::push_onto).
///
/// Then the manifest is written and tagged with its timestamp, and `latest` is
/// moved to it only if `latest` still names `target.parent` (or names nothing,
/// or already names this revision). When `latest` has moved on, the push still
/// succeeds in the sense that the revision is stored and reachable by its hash,
/// but `latest` is left alone and the call returns
/// [`PackageOpError::PushedNotLatest`], which carries the pushed top hash.
pub async fn push_files(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    target: &PushTarget,
    manifest: Manifest,
    files: &BTreeMap<LogicalKey, RowContent>,
) -> Res<PushOutcome> {
    refuse_unplaceable_rows(&manifest, files)?;

    let mut sorted: Vec<&ManifestRow> = manifest.rows.iter().collect();
    sorted.sort_by(|a, b| a.logical_key.cmp(&b.logical_key));
    let entries: Vec<_> = sorted.into_iter().map(entry_view).collect();
    validate_workflow_against_current_config(
        remote,
        target.origin.as_ref(),
        &target.handle.bucket,
        &target.handle.namespace.to_string(),
        manifest.header.message(),
        manifest.header.user_meta.as_ref(),
        manifest.header.workflow.as_ref(),
        &entries,
    )
    .await?;

    let host_config = remote.host_config(target.origin.as_ref()).await?;
    let mut rows = Vec::with_capacity(manifest.rows.len());
    for row in manifest.rows {
        let uploaded = if let Some(content) = files.get(&row.logical_key) {
            upload_row_content(
                storage,
                remote,
                &host_config,
                target.handle.clone(),
                row,
                content,
            )
            .await?
        } else if row.physical_key.starts_with("s3://") {
            row
        } else {
            upload_row(remote, &host_config, target.handle.clone(), row).await?
        };
        rows.push(uploaded);
    }
    let manifest = Manifest {
        header: manifest.header,
        rows,
    };

    let staging = tempfile::tempdir()?;
    let stream = Box::pin(manifest.records_stream().await);
    let (cache_path, top_hash) = build_manifest_from_rows_stream(
        storage,
        staging.path().join("manifests"),
        manifest.header.clone(),
        stream,
    )
    .await?;
    let manifest_uri = ManifestUri {
        hash: top_hash,
        bucket: target.handle.bucket.clone(),
        namespace: target.handle.namespace.clone(),
        origin: target.origin.clone(),
    };
    upload_manifest(storage, remote, &manifest_uri, &cache_path).await?;
    tag_timestamp(remote, &manifest_uri, chrono::Utc::now()).await?;

    let latest = match resolve_tag(
        remote,
        target.origin.as_ref(),
        target.handle.clone(),
        Tag::Latest,
    )
    .await
    {
        Ok(latest) => Some(latest.hash),
        Err(err) if err.is_not_found() => None,
        Err(err) => return Err(err),
    };
    match latest {
        Some(latest) if latest != manifest_uri.hash && Some(&latest) != target.parent.as_ref() => {
            warn!(
                "Pushed {} but did not certify latest: latest is {latest}",
                manifest_uri.display()
            );
            Err(PackageOpError::PushedNotLatest {
                top_hash: manifest_uri.hash,
                latest,
                expected: target.parent.as_ref().map_or_else(
                    || "no earlier revision".to_string(),
                    |parent| format!("revision {parent}"),
                ),
            }
            .into())
        }
        _ => {
            tag_latest(remote, &manifest_uri).await?;
            info!("✔️ Certified {} as latest", manifest_uri.display());
            Ok(PushOutcome {
                manifest_uri,
                certified_latest: true,
            })
        }
    }
}

/// Refuses, before any upload, a file the manifest has no row for and a row
/// with no way to get its bytes to the remote.
fn refuse_unplaceable_rows(manifest: &Manifest, files: &BTreeMap<LogicalKey, RowContent>) -> Res {
    if let Some(key) = files
        .keys()
        .find(|key| !manifest.contains_record(key.as_path()))
    {
        return Err(Error::PackageOp(PackageOpError::Push(format!(
            "{key} was supplied, but the manifest has no row for it"
        ))));
    }
    for row in &manifest.rows {
        if files.contains_key(&row.logical_key) {
            continue;
        }
        let scheme = Url::parse(&row.physical_key).map(|url| url.scheme().to_string());
        if !matches!(scheme.as_deref(), Ok("file" | "s3")) {
            return Err(Error::PackageOp(PackageOpError::Push(format!(
                "{} has no file or bytes supplied, and its physical key {:?} is neither a file:// nor an s3:// location",
                row.logical_key, row.physical_key
            ))));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    use aws_sdk_s3::primitives::ByteStream;
    use test_log::test;

    use crate::io::remote::mocks::MockRemote;
    use crate::manifest::HeaderMessage;
    use crate::manifest::ManifestHeader;
    use quilt_uri::S3Uri;

    const LATEST: &str = "s3://bkt/.quilt/named_packages/test/survey/latest";
    const PARENT: &str = "1111111111111111111111111111111111111111111111111111111111111111";
    const SOMEONE_ELSE: &str = "5555555555555555555555555555555555555555555555555555555555555555";

    fn target(parent: Option<&str>) -> PushTarget {
        PushTarget {
            handle: S3PackageHandle {
                bucket: "bkt".to_string(),
                namespace: ("test", "survey").into(),
            },
            origin: None,
            parent: parent.map(str::to_string),
        }
    }

    fn key(name: &str) -> LogicalKey {
        LogicalKey::new(name).expect("key")
    }

    /// A row that names only its logical key: the caller supplies the rest.
    fn bare_row(name: &str) -> ManifestRow {
        ManifestRow {
            logical_key: key(name),
            ..ManifestRow::default()
        }
    }

    fn manifest(rows: Vec<ManifestRow>) -> Manifest {
        Manifest {
            header: ManifestHeader {
                message: HeaderMessage::Text("from memory".to_string()),
                ..ManifestHeader::default()
            },
            rows,
        }
    }

    async fn read(remote: &MockRemote, uri: &str) -> Res<String> {
        let stream = remote
            .get_object_stream(None, &S3Uri::try_from(uri)?)
            .await?;
        Ok(String::from_utf8(stream.body.collect().await?.to_vec()).expect("utf-8"))
    }

    async fn pushed_manifest(remote: &MockRemote, top_hash: &str) -> Res<Manifest> {
        let text = read(remote, &format!("s3://bkt/.quilt/packages/{top_hash}")).await?;
        Manifest::from_reader(text.as_bytes()).await
    }

    async fn seed_latest(remote: &MockRemote, hash: &str) -> Res {
        remote
            .put_object(None, &S3Uri::try_from(LATEST)?, hash.as_bytes().to_vec())
            .await
    }

    async fn revisions(remote: &MockRemote) -> Res<Vec<String>> {
        remote
            .published_revisions(None, "bkt", &("test", "survey").into())
            .await
    }

    #[test(tokio::test)]
    async fn bytes_and_a_file_anywhere_publish_a_first_revision() -> Res {
        let remote = MockRemote::default();
        let elsewhere = std::path::absolute("/not/a/package/home/b.csv")?;
        remote
            .storage
            .write_byte_stream(&elsewhere, ByteStream::from_static(b"x,y\n1,2\n"))
            .await?;
        let files = BTreeMap::from([
            (key("a.txt"), RowContent::Bytes(b"hello".to_vec())),
            (key("data/b.csv"), RowContent::Path(elsewhere)),
        ]);
        let manifest = manifest(vec![bare_row("a.txt"), bare_row("data/b.csv")]);

        let pushed = push_files(&remote.storage, &remote, &target(None), manifest, &files).await?;

        assert!(pushed.certified_latest);
        assert_eq!(read(&remote, LATEST).await?, pushed.manifest_uri.hash);
        assert_eq!(
            revisions(&remote).await?,
            vec![pushed.manifest_uri.hash.clone()]
        );
        let stored = pushed_manifest(&remote, &pushed.manifest_uri.hash).await?;
        let a = stored.get_record(std::path::Path::new("a.txt")).expect("a");
        assert_eq!(a.size, 5);
        assert_eq!(
            a.physical_key,
            "s3://bkt/test/survey/a.txt?versionId=version"
        );
        let b = stored
            .get_record(std::path::Path::new("data/b.csv"))
            .expect("b");
        assert_eq!(b.size, 8);
        assert_eq!(
            b.physical_key,
            "s3://bkt/test/survey/data/b.csv?versionId=version"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_revision_onto_the_latest_it_was_built_on_becomes_latest() -> Res {
        let remote = MockRemote::default();
        seed_latest(&remote, PARENT).await?;
        let files = BTreeMap::from([(key("a.txt"), RowContent::Bytes(b"hi".to_vec()))]);

        let pushed = push_files(
            &remote.storage,
            &remote,
            &target(Some(PARENT)),
            manifest(vec![bare_row("a.txt")]),
            &files,
        )
        .await?;

        assert!(pushed.certified_latest);
        assert_eq!(read(&remote, LATEST).await?, pushed.manifest_uri.hash);
        Ok(())
    }

    /// The push succeeds as far as the revision goes: its manifest is stored
    /// and tagged, so the hash in the error reaches it. Only `latest` is not
    /// moved, and the error says so.
    #[test(tokio::test)]
    async fn a_latest_that_moved_keeps_the_revision_but_not_latest() -> Res {
        let remote = MockRemote::default();
        seed_latest(&remote, SOMEONE_ELSE).await?;
        let files = BTreeMap::from([(key("a.txt"), RowContent::Bytes(b"hi".to_vec()))]);

        let pushed = push_files(
            &remote.storage,
            &remote,
            &target(Some(PARENT)),
            manifest(vec![bare_row("a.txt")]),
            &files,
        )
        .await;

        let Err(Error::PackageOp(PackageOpError::PushedNotLatest {
            top_hash,
            latest,
            expected,
        })) = pushed
        else {
            panic!("expected PushedNotLatest, got {pushed:?}");
        };
        assert_eq!(latest, SOMEONE_ELSE);
        assert_eq!(expected, format!("revision {PARENT}"));
        assert_eq!(read(&remote, LATEST).await?, SOMEONE_ELSE);
        assert_eq!(revisions(&remote).await?, vec![top_hash.clone()]);
        let stored = pushed_manifest(&remote, &top_hash).await?;
        assert_eq!(stored.rows.len(), 1);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_first_revision_over_an_existing_latest_does_not_take_latest() -> Res {
        let remote = MockRemote::default();
        seed_latest(&remote, SOMEONE_ELSE).await?;
        let files = BTreeMap::from([(key("a.txt"), RowContent::Bytes(b"hi".to_vec()))]);

        let pushed = push_files(
            &remote.storage,
            &remote,
            &target(None),
            manifest(vec![bare_row("a.txt")]),
            &files,
        )
        .await;

        let Err(Error::PackageOp(PackageOpError::PushedNotLatest { expected, .. })) = pushed else {
            panic!("expected PushedNotLatest, got {pushed:?}");
        };
        assert_eq!(expected, "no earlier revision");
        assert_eq!(read(&remote, LATEST).await?, SOMEONE_ELSE);
        Ok(())
    }

    #[test(tokio::test)]
    async fn rows_already_on_s3_are_kept_where_they_are() -> Res {
        let remote = MockRemote::default();
        let row = ManifestRow {
            logical_key: key("old.bin"),
            physical_key: "s3://elsewhere/data/old.bin?versionId=v1".to_string(),
            size: 3,
            ..ManifestRow::default()
        };

        let pushed = push_files(
            &remote.storage,
            &remote,
            &target(None),
            manifest(vec![row.clone()]),
            &BTreeMap::new(),
        )
        .await?;

        let stored = pushed_manifest(&remote, &pushed.manifest_uri.hash).await?;
        assert_eq!(stored.rows[0].physical_key, row.physical_key);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_file_with_no_row_publishes_nothing() -> Res {
        let remote = MockRemote::default();
        let files = BTreeMap::from([(key("ghost.txt"), RowContent::Bytes(b"?".to_vec()))]);

        let pushed = push_files(
            &remote.storage,
            &remote,
            &target(None),
            manifest(vec![bare_row("a.txt")]),
            &files,
        )
        .await;

        assert!(
            matches!(pushed, Err(Error::PackageOp(PackageOpError::Push(_)))),
            "{pushed:?}"
        );
        assert_eq!(revisions(&remote).await?, Vec::<String>::new());
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_row_with_no_way_to_get_its_bytes_publishes_nothing() -> Res {
        let remote = MockRemote::default();
        let files = BTreeMap::from([(key("a.txt"), RowContent::Bytes(b"ok".to_vec()))]);

        // `bare_row` has an empty physical key and no bytes are supplied for it.
        let pushed = push_files(
            &remote.storage,
            &remote,
            &target(None),
            manifest(vec![bare_row("a.txt"), bare_row("b.txt")]),
            &files,
        )
        .await;

        assert!(
            matches!(pushed, Err(Error::PackageOp(PackageOpError::Push(_)))),
            "{pushed:?}"
        );
        assert_eq!(revisions(&remote).await?, Vec::<String>::new());
        Ok(())
    }
}
