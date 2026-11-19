use std::collections::BTreeMap;
use std::collections::HashSet;

use tracing::info;
use tracing::warn;
use url::Url;

use crate::Error;
use crate::LogicalKey;
use crate::Res;
use crate::error::PackageOpError;
use crate::installed_package::PushOutcome;
use crate::io::manifest::LatestAdvance;
use crate::io::manifest::RowContent;
use crate::io::manifest::advance_latest;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::manifest::read_latest;
use crate::io::manifest::tag_timestamp;
use crate::io::manifest::upload_manifest;
use crate::io::manifest::upload_row;
use crate::io::manifest::upload_row_content;
use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::remote::entry_view;
use crate::io::remote::validate_workflow_against_current_config;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use quilt_uri::Host;
use quilt_uri::ManifestUri;
use quilt_uri::S3PackageHandle;

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
///    themselves ([`RowContent`]). The row's own `physical_key` is ignored and
///    its `size` is measured from the content. Its `hash` decides the checksum:
///    a row with no hash ([`ObjectHash::default`](crate::object_hash::ObjectHash))
///    gets S3's checksum of the host's configured type (sha2-256-chunked unless
///    the host enables `crc64Checksums`, which is the preferred direction); a row that already
///    carries a real hash, such as one quilt3 computed in memory, keeps it with
///    its labeled type, and the upload is an error unless the bytes hash to it.
/// 2. A `file://` `physical_key`, uploaded as `push` does.
/// 3. An `s3://` `physical_key`, kept where it is: the bytes are already on the
///    remote, and nothing is copied. Its `hash` and `size` are trusted as the
///    caller gives them; nothing re-reads the object to check them.
///
/// Any other row is refused before anything is uploaded, and so is a key in
/// `files` that the manifest has no row for. The sizes of supplied content are
/// measured first, so the revision's workflow, including its entries schema, is
/// checked against real sizes before the first upload, as in
/// [`push_onto`](super::push_onto). A file whose size or modified time changes
/// while it is uploading, or between that measurement and its upload, is an
/// error.
///
/// The header's `user_meta`, each row's `meta` and the manifest's `dir_meta` are
/// published as given: integers beyond 64 bits keep their digits, and folder
/// metadata, which the top hash leaves out, is written beside the rows.
///
/// Then the manifest is written and tagged with its timestamp, and `latest` is
/// moved to it only if `latest` still names `target.parent` when read just
/// before the tag write. `latest` also moves when it names nothing, even if the
/// push has a parent: there is no revision for the push to conflict with. The
/// tag write is conditional on that read; see [`advance_latest`].
///
/// When `latest` names another revision, the push still succeeds in the sense
/// that the revision is stored and reachable by its hash, but `latest` is left
/// alone and the call returns [`PackageOpError::PushedNotLatest`], which carries
/// the pushed top hash. If instead writing the timestamp tag, or reading or
/// writing `latest`, fails after the manifest is stored, the call returns
/// [`PackageOpError::PushTagFailed`], which carries the top hash and the
/// failure: the revision is stored, and `latest` may or may not have moved. A
/// failure before the manifest is stored has no top hash to report, and leaves
/// the rows already uploaded as unreferenced objects.
pub async fn push_files(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    target: &PushTarget,
    mut manifest: Manifest,
    files: &BTreeMap<LogicalKey, RowContent>,
) -> Res<PushOutcome> {
    refuse_unplaceable_rows(&manifest, files)?;

    // Measured before validation, so the entries schema sees what will be
    // uploaded rather than the placeholder size of a row the caller never sized.
    let mut measured: BTreeMap<&LogicalKey, u64> = BTreeMap::new();
    for (key, content) in files {
        measured.insert(key, content.size(storage).await?);
    }
    for row in &mut manifest.rows {
        if let Some(size) = measured.get(&row.logical_key) {
            row.size = *size;
        }
    }

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
    let rows = upload_rows(storage, remote, target, &host_config, manifest.rows, files).await?;
    let manifest = Manifest {
        header: manifest.header,
        rows,
        dir_meta: manifest.dir_meta,
    };

    let staging = tempfile::tempdir()?;
    let stream = Box::pin(manifest.records_stream().await);
    let (cache_path, top_hash) = build_manifest_from_rows_stream(
        storage,
        staging.path().join("manifests"),
        manifest.header.clone(),
        manifest.dir_meta.clone(),
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

    tag_stored(remote, target, manifest_uri).await
}

/// Tags the stored revision; see [`push_files`].
async fn tag_stored(
    remote: &impl Remote,
    target: &PushTarget,
    manifest_uri: ManifestUri,
) -> Res<PushOutcome> {
    // From here the revision is stored, so a failure names its hash.
    let tag_failed = |err: Error| {
        Error::from(PackageOpError::PushTagFailed {
            top_hash: manifest_uri.hash.clone(),
            reason: err.to_string(),
        })
    };
    tag_timestamp(remote, &manifest_uri, chrono::Utc::now())
        .await
        .map_err(tag_failed)?;

    let latest = read_latest(remote, target.origin.as_ref(), target.handle.clone())
        .await
        .map_err(tag_failed)?;
    let not_latest = |latest: String| -> Error {
        warn!(
            "Pushed {} but did not certify latest: latest is {latest}",
            manifest_uri.display()
        );
        PackageOpError::PushedNotLatest {
            top_hash: manifest_uri.hash.clone(),
            latest,
            expected: target.parent.as_ref().map_or_else(
                || "no earlier revision".to_string(),
                |parent| format!("revision {parent}"),
            ),
        }
        .into()
    };
    if let Some(hash) = latest.hash()
        && hash != manifest_uri.hash
        && Some(hash) != target.parent.as_deref()
    {
        return Err(not_latest(hash.to_owned()));
    }
    match advance_latest(remote, &manifest_uri, &latest)
        .await
        .map_err(tag_failed)?
    {
        LatestAdvance::Moved {
            latest: Some(latest),
        } => Err(not_latest(latest)),
        LatestAdvance::Moved { latest: None } => Err(PackageOpError::PushTagFailed {
            top_hash: manifest_uri.hash,
            reason: "latest changed while this push ran".to_string(),
        }
        .into()),
        written => {
            info!("✔️ Certified {} as latest", manifest_uri.display());
            Ok(PushOutcome {
                manifest_uri,
                certified_latest: true,
                latest_unchecked: written == LatestAdvance::Unchecked,
            })
        }
    }
}

/// Uploads every row that is not already on the remote, and returns the rows
/// as the published manifest will hold them.
async fn upload_rows(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    target: &PushTarget,
    host_config: &HostConfig,
    rows: Vec<ManifestRow>,
    files: &BTreeMap<LogicalKey, RowContent>,
) -> Res<Vec<ManifestRow>> {
    let mut uploaded_rows = Vec::with_capacity(rows.len());
    for row in rows {
        let uploaded = if let Some(content) = files.get(&row.logical_key) {
            let measured_size = row.size;
            let uploaded = upload_row_content(
                storage,
                remote,
                host_config,
                target.handle.clone(),
                row,
                content,
            )
            .await?;
            if uploaded.size != measured_size {
                return Err(Error::PackageOp(PackageOpError::Push(format!(
                    "{} changed after its size was read: it was {measured_size} bytes, then {}",
                    uploaded.logical_key, uploaded.size
                ))));
            }
            uploaded
        } else if row.physical_key.starts_with("s3://") {
            row
        } else {
            upload_row(remote, host_config, target.handle.clone(), row).await?
        };
        uploaded_rows.push(uploaded);
    }
    Ok(uploaded_rows)
}

/// Refuses, before any upload, a file the manifest has no row for and a row
/// with no way to get its bytes to the remote.
fn refuse_unplaceable_rows(manifest: &Manifest, files: &BTreeMap<LogicalKey, RowContent>) -> Res {
    let known: HashSet<&LogicalKey> = manifest.rows.iter().map(|row| &row.logical_key).collect();
    if let Some(key) = files.keys().find(|key| !known.contains(key)) {
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
    use bytes::Bytes;
    use test_log::test;

    use crate::io::remote::mocks::MockRemote;
    use crate::manifest::DirMeta;
    use crate::manifest::HeaderMessage;
    use crate::manifest::ManifestHeader;
    use crate::manifest::RawNumbers;
    use quilt_uri::Host;
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
            dir_meta: Vec::new(),
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
            (
                key("a.txt"),
                RowContent::Bytes(Bytes::from_static(b"hello")),
            ),
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

    /// A manifest whose metadata a lossy copy would change: integers beyond 64
    /// bits in the header's and a row's, and folder metadata.
    fn manifest_with_metadata() -> Res<Manifest> {
        let (user_meta, user_meta_numbers) = RawNumbers::parse(r#"{"big": 18446744073709551616}"#)?;
        let (meta, meta_numbers) = RawNumbers::parse(r#"{"id": 18446744073709551617}"#)?;
        let mut manifest = manifest(vec![
            ManifestRow {
                meta: Some(meta),
                meta_numbers,
                ..bare_row("data/b.csv")
            },
            bare_row("a.txt"),
        ]);
        manifest.header.user_meta = Some(user_meta);
        manifest.header.user_meta_numbers = user_meta_numbers;
        let (dir_meta, dir_meta_numbers) =
            RawNumbers::parse(r#"{"n": 123456789012345678901234567890}"#)?;
        let mut dir = DirMeta::new(key("data"), dir_meta);
        dir.meta_numbers = dir_meta_numbers;
        manifest.dir_meta = vec![dir];
        Ok(manifest)
    }

    fn metadata_files() -> BTreeMap<LogicalKey, RowContent> {
        BTreeMap::from([
            (
                key("a.txt"),
                RowContent::Bytes(Bytes::from_static(b"hello")),
            ),
            (
                key("data/b.csv"),
                RowContent::Bytes(Bytes::from_static(b"x,y\n1,2\n")),
            ),
        ])
    }

    /// Folder metadata and integers beyond 64 bits are written as given, not
    /// dropped or rounded to floats on the way through.
    #[test(tokio::test)]
    async fn folder_metadata_and_big_integers_reach_the_stored_manifest() -> Res {
        let remote = MockRemote::default();
        let sent = manifest_with_metadata()?;

        let pushed = push_files(
            &remote.storage,
            &remote,
            &target(None),
            sent.clone(),
            &metadata_files(),
        )
        .await?;

        let text = read(
            &remote,
            &format!("s3://bkt/.quilt/packages/{}", pushed.manifest_uri.hash),
        )
        .await?;
        assert!(text.contains("18446744073709551616"), "{text}");
        assert!(text.contains("18446744073709551617"), "{text}");
        assert!(text.contains("123456789012345678901234567890"), "{text}");
        let stored = pushed_manifest(&remote, &pushed.manifest_uri.hash).await?;
        assert_eq!(stored.dir_meta, sent.dir_meta);
        assert!(stored.header.same_user_meta(&sent.header));
        let sent_row = sent
            .get_record(std::path::Path::new("data/b.csv"))
            .expect("sent");
        let stored_row = stored
            .get_record(std::path::Path::new("data/b.csv"))
            .expect("stored");
        assert!(stored_row.same_meta(sent_row));
        Ok(())
    }

    /// Folder metadata is not hashed, so the top hash is the one the rows give.
    #[test(tokio::test)]
    async fn folder_metadata_does_not_change_the_top_hash() -> Res {
        let with = MockRemote::default();
        let without = MockRemote::default();
        let sent = manifest_with_metadata()?;
        let mut bare = sent.clone();
        bare.dir_meta.clear();

        let a = push_files(&with.storage, &with, &target(None), sent, &metadata_files()).await?;
        let b = push_files(
            &without.storage,
            &without,
            &target(None),
            bare,
            &metadata_files(),
        )
        .await?;

        assert_eq!(a.manifest_uri.hash, b.manifest_uri.hash);
        Ok(())
    }

    /// What a push publishes installs to a folder, and installing leaves the
    /// manifest's metadata as it read.
    #[test(tokio::test)]
    async fn a_pushed_manifest_installs_to_a_folder_with_its_metadata_intact() -> Res {
        let remote = MockRemote::default();
        let pushed = push_files(
            &remote.storage,
            &remote,
            &target(None),
            manifest_with_metadata()?,
            &metadata_files(),
        )
        .await?;
        let stored = pushed_manifest(&remote, &pushed.manifest_uri.hash).await?;
        // The mock's upload keeps nothing, so put the bytes where the rows say.
        for row in &stored.rows {
            let contents = metadata_files();
            let RowContent::Bytes(bytes) = &contents[&row.logical_key] else {
                unreachable!("metadata_files holds bytes")
            };
            remote
                .put_object(
                    None,
                    &S3Uri::try_from(row.physical_key.as_str())?,
                    bytes.to_vec(),
                )
                .await?;
        }
        let dest = std::path::absolute("/installed/here")?;

        let report =
            crate::flow::install_to(&remote.storage, &remote, None, &stored, &dest, None).await?;

        assert_eq!(report.placed.len(), 2, "{report:?}");
        assert_eq!(stored.dir_meta.len(), 1);
        let placed = remote.storage.read_byte_stream(dest.join("a.txt")).await?;
        assert_eq!(placed.collect().await?.to_vec(), b"hello");
        // Installing only reads the manifest, so the one the remote holds is
        // still what was pushed. Compare it with a manifest built on its own,
        // not with a copy of the one that was read, so a push that loses
        // metadata fails here.
        let after = pushed_manifest(&remote, &pushed.manifest_uri.hash).await?;
        let expected = manifest_with_metadata()?;
        assert_eq!(after.dir_meta, expected.dir_meta);
        assert!(after.header.same_user_meta(&expected.header));
        assert_eq!(after.rows.len(), expected.rows.len());
        for want in &expected.rows {
            let got = after.get_record(want.logical_key.as_path()).expect("row");
            assert!(got.same_meta(want), "{:?}", want.logical_key);
        }
        let text = read(
            &remote,
            &format!("s3://bkt/.quilt/packages/{}", pushed.manifest_uri.hash),
        )
        .await?;
        assert!(text.contains("18446744073709551617"), "{text}");
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_revision_onto_the_latest_it_was_built_on_becomes_latest() -> Res {
        let remote = MockRemote::default();
        seed_latest(&remote, PARENT).await?;
        let files = BTreeMap::from([(key("a.txt"), RowContent::Bytes(Bytes::from_static(b"hi")))]);

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
        let files = BTreeMap::from([(key("a.txt"), RowContent::Bytes(Bytes::from_static(b"hi")))]);

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
    async fn a_latest_moved_between_the_read_and_the_write_keeps_the_revision() -> Res {
        let remote = MockRemote::default();
        seed_latest(&remote, PARENT).await?;
        let files = BTreeMap::from([(key("a.txt"), RowContent::Bytes(Bytes::from_static(b"hi")))]);
        let target = target(Some(PARENT));
        let read_gate = remote.park(LATEST);
        let compete = async {
            read_gate.arrived().await;
            let write_gate = remote.park(LATEST);
            read_gate.release();
            write_gate.arrived().await;
            seed_latest(&remote, SOMEONE_ELSE).await?;
            write_gate.release();
            Ok::<_, Error>(())
        };

        let (pushed, competed) = tokio::join!(
            push_files(
                &remote.storage,
                &remote,
                &target,
                manifest(vec![bare_row("a.txt")]),
                &files,
            ),
            compete
        );
        competed?;

        let Err(Error::PackageOp(PackageOpError::PushedNotLatest {
            top_hash, latest, ..
        })) = pushed
        else {
            panic!("expected PushedNotLatest, got {pushed:?}");
        };
        assert_eq!(latest, SOMEONE_ELSE);
        assert_eq!(read(&remote, LATEST).await?, SOMEONE_ELSE);
        assert_eq!(revisions(&remote).await?, vec![top_hash]);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_latest_moved_and_then_unreadable_names_the_stored_revision() -> Res {
        let remote = MockRemote::default();
        seed_latest(&remote, PARENT).await?;
        let files = BTreeMap::from([(key("a.txt"), RowContent::Bytes(Bytes::from_static(b"hi")))]);
        let target = target(Some(PARENT));
        let read_gate = remote.park(LATEST);
        let compete = async {
            read_gate.arrived().await;
            let write_gate = remote.park(LATEST);
            read_gate.release();
            write_gate.arrived().await;
            seed_latest(&remote, SOMEONE_ELSE).await?;
            let reread_gate = remote.park(LATEST);
            write_gate.release();
            reread_gate.arrived().await;
            remote.deny_reads(LATEST);
            reread_gate.release();
            Ok::<_, Error>(())
        };

        let (pushed, competed) = tokio::join!(
            push_files(
                &remote.storage,
                &remote,
                &target,
                manifest(vec![bare_row("a.txt")]),
                &files,
            ),
            compete
        );
        competed?;

        let Err(Error::PackageOp(PackageOpError::PushTagFailed { top_hash, .. })) = pushed else {
            panic!("expected PushTagFailed, got {pushed:?}");
        };
        assert_eq!(revisions(&remote).await?, vec![top_hash]);
        assert_eq!(
            remote.current(LATEST).await?,
            Some(SOMEONE_ELSE.as_bytes().to_vec())
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_first_revision_over_an_existing_latest_does_not_take_latest() -> Res {
        let remote = MockRemote::default();
        seed_latest(&remote, SOMEONE_ELSE).await?;
        let files = BTreeMap::from([(key("a.txt"), RowContent::Bytes(Bytes::from_static(b"hi")))]);

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
        let files = BTreeMap::from([(
            key("ghost.txt"),
            RowContent::Bytes(Bytes::from_static(b"?")),
        )]);

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
        let files = BTreeMap::from([(key("a.txt"), RowContent::Bytes(Bytes::from_static(b"ok")))]);

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

    /// A remote that acts like the mock but can break the publish step or change
    /// a file mid-push, so a test can land a failure or an edit where it wants.
    struct Meddling {
        inner: MockRemote,
        /// Fails writes to anything under `.quilt/named_packages/`: the tags.
        fail_tags: bool,
        /// A file to append to when the push asks for the host's config (after
        /// its size was measured, before it is uploaded).
        grow_on_host_config: Option<std::path::PathBuf>,
        /// A file to append to during its upload.
        grow_on_upload: Option<std::path::PathBuf>,
    }

    impl Meddling {
        fn new() -> Self {
            Self {
                inner: MockRemote::default(),
                fail_tags: false,
                grow_on_host_config: None,
                grow_on_upload: None,
            }
        }

        async fn append(&self, path: &std::path::Path) -> Res {
            let mut contents = self
                .inner
                .storage
                .read_byte_stream(path)
                .await?
                .collect()
                .await?
                .to_vec();
            contents.extend_from_slice(b" and more");
            self.inner
                .storage
                .write_byte_stream(path, ByteStream::from(contents))
                .await
        }
    }

    impl Remote for Meddling {
        async fn exists(&self, host: Option<&Host>, uri: &S3Uri) -> Res<bool> {
            self.inner.exists(host, uri).await
        }

        async fn get_object_stream(
            &self,
            host: Option<&Host>,
            uri: &S3Uri,
        ) -> Res<crate::io::remote::RemoteObjectStream> {
            self.inner.get_object_stream(host, uri).await
        }

        async fn resolve_url(&self, host: Option<&Host>, uri: &S3Uri) -> Res<S3Uri> {
            self.inner.resolve_url(host, uri).await
        }

        async fn put_object(
            &self,
            host: Option<&Host>,
            uri: &S3Uri,
            contents: impl Into<ByteStream>,
        ) -> Res {
            if self.fail_tags && uri.key.contains(".quilt/named_packages/") {
                return Err(Error::PackageOp(PackageOpError::Package(
                    "tag write refused".to_string(),
                )));
            }
            self.inner.put_object(host, uri, contents).await
        }

        async fn put_object_if(
            &self,
            host: Option<&Host>,
            uri: &S3Uri,
            contents: impl Into<ByteStream>,
            condition: &crate::io::remote::WriteCondition,
        ) -> Res<crate::io::remote::ConditionalPut> {
            if self.fail_tags && uri.key.contains(".quilt/named_packages/") {
                return Err(Error::PackageOp(PackageOpError::Package(
                    "tag write refused".to_string(),
                )));
            }
            self.inner
                .put_object_if(host, uri, contents, condition)
                .await
        }

        async fn upload_file(
            &self,
            host_config: &crate::io::remote::HostConfig,
            source_path: impl AsRef<std::path::Path>,
            dest_uri: &S3Uri,
            size: u64,
        ) -> Res<(S3Uri, crate::object_hash::ObjectHash)> {
            if let Some(path) = &self.grow_on_upload {
                self.append(path).await?;
            }
            self.inner
                .upload_file(host_config, source_path, dest_uri, size)
                .await
        }

        async fn host_config(&self, host: Option<&Host>) -> Res<crate::io::remote::HostConfig> {
            if let Some(path) = &self.grow_on_host_config {
                self.append(path).await?;
            }
            self.inner.host_config(host).await
        }

        async fn verify_bucket(&self, bucket: &str) -> Res {
            self.inner.verify_bucket(bucket).await
        }

        async fn published_revisions(
            &self,
            host: Option<&Host>,
            bucket: &str,
            namespace: &quilt_uri::Namespace,
        ) -> Res<Vec<String>> {
            self.inner
                .published_revisions(host, bucket, namespace)
                .await
        }

        async fn manifest_hashes_with_prefix(
            &self,
            host: Option<&Host>,
            bucket: &str,
            prefix: &str,
        ) -> Res<Vec<String>> {
            self.inner
                .manifest_hashes_with_prefix(host, bucket, prefix)
                .await
        }
    }

    const ENTRIES_CONFIG: &[u8] = b"version: \"1\"\nworkflows:\n  gate:\n    name: Gate\n    entries_schema: entries\nschemas:\n  entries:\n    url: s3://bkt/schemas/entries.json\n";

    fn gated(row: ManifestRow) -> Manifest {
        let mut manifest = manifest(vec![row]);
        manifest.header.workflow = Some(crate::manifest::Workflow {
            config: "s3://bkt/.quilt/workflows/config.yml".parse().expect("uri"),
            id: Some(crate::manifest::WorkflowId {
                id: "gate".to_string(),
                schemas: BTreeMap::new(),
            }),
        });
        manifest
    }

    async fn with_entries_schema(remote: &MockRemote, schema: &str) -> Res {
        remote
            .put_object(
                None,
                &S3Uri::try_from("s3://bkt/.quilt/workflows/config.yml")?,
                ENTRIES_CONFIG.to_vec(),
            )
            .await?;
        remote
            .put_object(
                None,
                &S3Uri::try_from("s3://bkt/schemas/entries.json")?,
                schema.as_bytes().to_vec(),
            )
            .await
    }

    /// The entries schema judges the size of what will be uploaded, not the
    /// placeholder size of a row the caller never sized.
    #[test(tokio::test)]
    async fn the_entries_schema_sees_the_real_size_of_supplied_content() -> Res {
        let files = BTreeMap::from([(
            key("a.txt"),
            RowContent::Bytes(Bytes::from_static(b"hello")),
        )]);
        // Five bytes satisfy "at least four"; the placeholder size, zero, would not.
        let remote = MockRemote::default();
        with_entries_schema(
            &remote,
            r#"{"type": "array", "items": {"properties": {"size": {"minimum": 4}}}}"#,
        )
        .await?;
        let pushed = push_files(
            &remote.storage,
            &remote,
            &target(None),
            gated(bare_row("a.txt")),
            &files,
        )
        .await;
        assert!(pushed.is_ok(), "{pushed:?}");

        // And five bytes break "at most three", which the placeholder would meet.
        let remote = MockRemote::default();
        with_entries_schema(
            &remote,
            r#"{"type": "array", "items": {"properties": {"size": {"maximum": 3}}}}"#,
        )
        .await?;
        let pushed = push_files(
            &remote.storage,
            &remote,
            &target(None),
            gated(bare_row("a.txt")),
            &files,
        )
        .await;
        assert!(
            matches!(pushed, Err(Error::WorkflowValidation(_))),
            "{pushed:?}"
        );
        assert_eq!(revisions(&remote).await?, Vec::<String>::new());
        Ok(())
    }

    /// A row with no hash is hashed with the host's configured type, and a row
    /// that carries a real hash keeps it, whatever the host's type is, so long
    /// as the bytes hash to it.
    #[test(tokio::test)]
    async fn supplied_content_keeps_a_real_hash_and_otherwise_takes_the_hosts_type() -> Res {
        use crate::object_hash::MULTIHASH_CRC64_NVME;
        use crate::object_hash::MULTIHASH_SHA256_CHUNKED;
        let remote = MockRemote::default();
        let files = BTreeMap::from([
            (
                key("bare.txt"),
                RowContent::Bytes(Bytes::from_static(b"hi")),
            ),
            (
                key("hashed.txt"),
                RowContent::Bytes(Bytes::from_static(b"yo")),
            ),
        ]);
        // quilt3 hashed `yo` with CRC64 while the host is configured for
        // sha2-256-chunked, which is what a row with no hash gets here.
        let crc64 = crate::io::manifest::hash_bytes(b"yo", MULTIHASH_CRC64_NVME).await?;
        let hashed = ManifestRow {
            hash: crc64.clone(),
            ..bare_row("hashed.txt")
        };

        let pushed = push_files(
            &remote.storage,
            &remote,
            &target(None),
            manifest(vec![bare_row("bare.txt"), hashed]),
            &files,
        )
        .await?;

        let stored = pushed_manifest(&remote, &pushed.manifest_uri.hash).await?;
        let bare = stored
            .get_record(std::path::Path::new("bare.txt"))
            .expect("bare");
        assert_eq!(bare.hash.algorithm(), MULTIHASH_SHA256_CHUNKED);
        let kept = stored
            .get_record(std::path::Path::new("hashed.txt"))
            .expect("hashed");
        assert_eq!(kept.hash, crc64);
        Ok(())
    }

    /// Bytes that do not hash to the hash the row carries are an error, and
    /// nothing is published.
    #[test(tokio::test)]
    async fn supplied_content_that_does_not_match_its_hash_is_an_error() -> Res {
        use crate::object_hash::MULTIHASH_CRC64_NVME;
        use crate::object_hash::MULTIHASH_SHA256;
        use crate::object_hash::MULTIHASH_SHA256_CHUNKED;
        for algorithm in [
            MULTIHASH_CRC64_NVME,
            MULTIHASH_SHA256_CHUNKED,
            MULTIHASH_SHA256,
        ] {
            let remote = MockRemote::default();
            let claimed = crate::io::manifest::hash_bytes(b"what was hashed", algorithm).await?;
            let row = ManifestRow {
                hash: claimed,
                ..bare_row("a.txt")
            };
            let files = BTreeMap::from([(
                key("a.txt"),
                RowContent::Bytes(Bytes::from_static(b"what was supplied")),
            )]);

            let pushed = push_files(
                &remote.storage,
                &remote,
                &target(None),
                manifest(vec![row]),
                &files,
            )
            .await;

            assert!(
                matches!(&pushed, Err(Error::PackageOp(PackageOpError::Push(message))) if message.contains("do not hash")),
                "algorithm {algorithm:#x}: {pushed:?}"
            );
            assert_eq!(revisions(&remote).await?, Vec::<String>::new());
        }
        Ok(())
    }

    /// A legacy SHA-256 hash, which S3 cannot compute, is verified here and kept.
    #[test(tokio::test)]
    async fn a_legacy_sha256_hash_is_verified_locally_and_kept() -> Res {
        use crate::object_hash::MULTIHASH_SHA256;
        let remote = MockRemote::default();
        let claimed = crate::io::manifest::hash_bytes(b"yo", MULTIHASH_SHA256).await?;
        let row = ManifestRow {
            hash: claimed.clone(),
            ..bare_row("a.txt")
        };
        let files = BTreeMap::from([(key("a.txt"), RowContent::Bytes(Bytes::from_static(b"yo")))]);

        let pushed = push_files(
            &remote.storage,
            &remote,
            &target(None),
            manifest(vec![row]),
            &files,
        )
        .await?;

        let stored = pushed_manifest(&remote, &pushed.manifest_uri.hash).await?;
        assert_eq!(stored.rows[0].hash, claimed);
        Ok(())
    }

    /// The type of a row with no hash is the host's: CRC64 when the host asks for it.
    #[test(tokio::test)]
    async fn a_row_with_no_hash_follows_the_hosts_type() -> Res {
        use crate::io::remote::HostConfig;
        use crate::object_hash::MULTIHASH_CRC64_NVME;
        use crate::object_hash::MULTIHASH_SHA256_CHUNKED;
        let remote = MockRemote::default();
        let handle = target(None).handle;
        let content = RowContent::Bytes(Bytes::from_static(b"hi"));
        for (host, expected) in [
            (HostConfig::default_crc64(), MULTIHASH_CRC64_NVME),
            (HostConfig::default(), MULTIHASH_SHA256_CHUNKED),
        ] {
            let uploaded = crate::io::manifest::upload_row_content(
                &remote.storage,
                &remote,
                &host,
                handle.clone(),
                bare_row("a.txt"),
                &content,
            )
            .await?;
            assert_eq!(uploaded.hash.algorithm(), expected);
        }
        Ok(())
    }

    /// With a parent but no `latest` there is nothing to conflict with.
    #[test(tokio::test)]
    async fn a_push_with_a_parent_takes_latest_when_there_is_none() -> Res {
        let remote = MockRemote::default();
        let files = BTreeMap::from([(key("a.txt"), RowContent::Bytes(Bytes::from_static(b"hi")))]);

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

    /// Once the manifest is stored, a failing tag write still names its hash.
    #[test(tokio::test)]
    async fn a_failed_tag_write_names_the_stored_revision() -> Res {
        let mut remote = Meddling::new();
        remote.fail_tags = true;
        let files = BTreeMap::from([(key("a.txt"), RowContent::Bytes(Bytes::from_static(b"hi")))]);

        let pushed = push_files(
            &remote.inner.storage,
            &remote,
            &target(None),
            manifest(vec![bare_row("a.txt")]),
            &files,
        )
        .await;

        let Err(Error::PackageOp(PackageOpError::PushTagFailed { top_hash, reason })) = pushed
        else {
            panic!("expected PushTagFailed, got {pushed:?}");
        };
        assert!(reason.contains("tag write refused"), "{reason}");
        assert!(
            pushed_manifest(&remote.inner, &top_hash).await.is_ok(),
            "the manifest named in the error is stored"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_file_that_changes_during_its_upload_is_an_error() -> Res {
        let mut remote = Meddling::new();
        let elsewhere = std::path::absolute("/not/a/package/home/c.txt")?;
        remote
            .inner
            .storage
            .write_byte_stream(&elsewhere, ByteStream::from_static(b"original"))
            .await?;
        remote.grow_on_upload = Some(elsewhere.clone());
        let files = BTreeMap::from([(key("c.txt"), RowContent::Path(elsewhere))]);

        let pushed = push_files(
            &remote.inner.storage,
            &remote,
            &target(None),
            manifest(vec![bare_row("c.txt")]),
            &files,
        )
        .await;

        assert!(
            matches!(&pushed, Err(Error::PackageOp(PackageOpError::Push(message))) if message.contains("changed while")),
            "{pushed:?}"
        );
        assert_eq!(revisions(&remote.inner).await?, Vec::<String>::new());
        Ok(())
    }

    /// The size the workflow was validated against is the size uploaded: a file
    /// that grows between the two is refused.
    #[test(tokio::test)]
    async fn a_file_that_changes_after_its_size_was_read_is_an_error() -> Res {
        let mut remote = Meddling::new();
        let elsewhere = std::path::absolute("/not/a/package/home/c.txt")?;
        remote
            .inner
            .storage
            .write_byte_stream(&elsewhere, ByteStream::from_static(b"original"))
            .await?;
        remote.grow_on_host_config = Some(elsewhere.clone());
        let files = BTreeMap::from([(key("c.txt"), RowContent::Path(elsewhere))]);

        let pushed = push_files(
            &remote.inner.storage,
            &remote,
            &target(None),
            manifest(vec![bare_row("c.txt")]),
            &files,
        )
        .await;

        assert!(
            matches!(&pushed, Err(Error::PackageOp(PackageOpError::Push(message))) if message.contains("changed after its size")),
            "{pushed:?}"
        );
        assert_eq!(revisions(&remote.inner).await?, Vec::<String>::new());
        Ok(())
    }
}
