//! Contains utility functions to work with manifests.

use std::marker::Unpin;
use std::path::PathBuf;

use aws_sdk_s3::primitives::ByteStream;
use bytes::Bytes;
use tokio::io::AsyncReadExt;
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use tracing::log;
use url::Url;

use crate::Error;
use crate::Res;
use crate::checksum::hash_file;
use crate::error::ManifestError;
use crate::error::PackageOpError;
use crate::io::remote::ConditionalPut;
use crate::io::remote::HostChecksums;
use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::remote::WriteCondition;
use crate::io::storage::Storage;
use crate::manifest::DirMeta;
use crate::manifest::Manifest;
use crate::manifest::ManifestHeader;
use crate::manifest::ManifestRow;
use crate::manifest::TopHasher;
#[cfg(test)]
use crate::manifest::Workflow;
#[cfg(test)]
use crate::manifest::WorkflowId;
use crate::object_hash::Crc64Hash;
use crate::object_hash::Hash;
use crate::object_hash::MULTIHASH_CRC64_NVME;
use crate::object_hash::MULTIHASH_SHA256;
use crate::object_hash::MULTIHASH_SHA256_CHUNKED;
use crate::object_hash::ObjectHash;
use crate::object_hash::Sha256ChunkedHash;
use crate::object_hash::Sha256Hash;
use quilt_uri::Host;
use quilt_uri::ManifestUri;
use quilt_uri::ObjectUri;
use quilt_uri::RevisionPointer;
use quilt_uri::S3PackageHandle;
use quilt_uri::S3PackageUri;
use quilt_uri::S3Uri;
use quilt_uri::Seconds;
use quilt_uri::Tag;
use quilt_uri::TagUri;
use quilt_uri::UriError;
use quilt_uri::is_top_hash;

/// Reads the manifest `manifest_uri` names from the remote, with no cache.
pub async fn fetch_jsonl(remote: &impl Remote, manifest_uri: &ManifestUri) -> Res<Manifest> {
    let s3_uri: S3Uri = manifest_uri.clone().into();
    let contents = remote
        .get_object_stream(manifest_uri.origin.as_ref(), &s3_uri)
        .await?;
    Manifest::from_reader(contents.body.into_async_read()).await
}

pub(crate) async fn bytestream_to_string(bytestream: ByteStream) -> Res<String> {
    let mut reader = bytestream.into_async_read();
    let mut contents = Vec::new();
    reader.read_to_end(&mut contents).await?;
    String::from_utf8(contents).map_err(|err| Error::Utf8(err.utf8_error()))
}

/// Upload manifest from the local path
/// We don't care about checksum of the resulted file.
async fn upload_from(
    storage: &impl Storage,
    remote: &impl Remote,
    manifest_path: &PathBuf,
    manifest_uri: &ManifestUri,
) -> Res {
    // TODO: FAIL if the manifest with this hash already exists?
    let body = storage.read_byte_stream(manifest_path).await?;
    log::info!("Writing remote manifest to {manifest_uri:?}");
    remote
        .put_object(
            manifest_uri.origin.as_ref(),
            &manifest_uri.clone().into(),
            body,
        )
        .await
}

/// Upload JSONL manifest to remote.
/// We don't care about checksum of the resulted file.
pub async fn upload_manifest(
    storage: &impl Storage,
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
    path: &PathBuf,
) -> Res {
    // Upload the JSONL manifest to the remote, don't tag it yet
    upload_from(storage, remote, path, manifest_uri).await?;
    log::info!("JSONL manifest uploaded");

    log::info!("Uploaded remote manifest: {manifest_uri:?}");
    Ok(())
}

/// Upload file containing hash of the manifest
/// "tagged" by timestamp.
///
/// This is the only place a revision's publish time reaches the remote: the
/// manifest format carries no timestamps (neither `ManifestHeader` nor
/// `ManifestRow` has a time field), so `.quilt/named_packages/<ns>/<epoch>` is
/// the registry's revision history.
///
/// This client reads back which revisions have one
/// ([`Remote::published_revisions`]), never the times. So nothing here can
/// answer "when was revision X published". The value written is the *local*
/// `CommitState::timestamp` — when the commit was made on the pushing machine,
/// not when the upload completed.
pub async fn tag_timestamp(
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
    timestamp: jiff::Timestamp,
) -> Res {
    // Tag the new commit.
    // If {self.commit.tag} does not already exist at
    // {self.remote}/.quilt/named_packages/{self.namespace},
    // create it with the value of {self.commit.hash}
    // TODO: Otherwise try again with the current timestamp as the tag
    // (e.g., try five times with exponential backoff, then Error)
    // NB: the key has one-second resolution and holds the *commit* time, so two
    // commits of the same namespace made within one second collide even if
    // pushed hours apart. `upload_tag` overwrites, so the second push silently
    // takes over the tag and the *first* revision is left with a manifest but no
    // history entry.
    let tag_timestamp =
        TagUri::timestamp(manifest_uri, Seconds(crate::time::whole_seconds(timestamp)));
    upload_tag(remote, manifest_uri, tag_timestamp).await
}

/// Upload file containing hash of the manifest
/// "tagged" as "latest", whatever `latest` names now; [`advance_latest`] is
/// the checked write.
pub async fn tag_latest(remote: &impl Remote, manifest_uri: &ManifestUri) -> Res {
    let tag_latest = TagUri::latest(manifest_uri);
    upload_tag(remote, manifest_uri, tag_latest).await
}

/// What a package's `latest` tag held when a publish read it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LatestSeen {
    /// No `latest` tag: nothing has been published under this name.
    Absent,
    /// The revision `latest` named, and its `ETag` for a conditional write.
    Tagged {
        uri: ManifestUri,
        etag: Option<String>,
    },
    /// The role may not read `latest`, so the base check cannot run.
    Unreadable,
}

impl LatestSeen {
    /// The hash `latest` named, if it was read and present.
    #[must_use]
    pub fn hash(&self) -> Option<&str> {
        match self {
            Self::Tagged { uri, .. } => Some(&uri.hash),
            Self::Absent | Self::Unreadable => None,
        }
    }
}

/// Reads a package's `latest` tag for a publish that may move it: absent,
/// what it names with the `ETag` a conditional write needs, or unreadable
/// when the role is denied it.
pub async fn read_latest(
    remote: &impl Remote,
    host: Option<&Host>,
    uri: impl Into<S3PackageHandle>,
) -> Res<LatestSeen> {
    match read_tag(remote, host, uri, Tag::Latest).await {
        Ok((uri, etag)) => Ok(LatestSeen::Tagged { uri, etag }),
        Err(err) if err.is_not_found() => Ok(LatestSeen::Absent),
        Err(err) if err.is_access_denied() => Ok(LatestSeen::Unreadable),
        Err(err) => Err(err),
    }
}

/// How a publish's write of `latest` went.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum LatestAdvance {
    /// `latest` names the revision, and was unchanged since it was read.
    Checked,
    /// `latest` names the revision, written without the check because it
    /// could not run. Another writer may have moved it since.
    Unchecked,
    /// `latest` moved since it was read, so it was left as it is.
    Moved {
        /// The revision `latest` moved to, when a read after the refused
        /// write found one other than the revision first read.
        latest: Option<String>,
    },
}

/// Moves `latest` to `manifest_uri` only if it still holds what `seen` read:
/// S3's conditional write, with the `ETag` from that read, or requiring the
/// tag to be absent when it was. When the check cannot run, because the role
/// may not read the tag or the store returned no `ETag`, `latest` is written
/// plainly and the write reported [`LatestAdvance::Unchecked`]: a publish
/// never fails because the check could not be made.
pub async fn advance_latest(
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
    seen: &LatestSeen,
) -> Res<LatestAdvance> {
    let condition = match seen {
        LatestSeen::Absent => Some(WriteCondition::Absent),
        LatestSeen::Tagged {
            etag: Some(etag), ..
        } => Some(WriteCondition::Matches(etag.clone())),
        LatestSeen::Tagged { etag: None, .. } | LatestSeen::Unreadable => None,
    };
    let tag_uri = S3Uri::from(TagUri::latest(manifest_uri));
    if let Some(condition) = condition {
        match remote
            .put_object_if(
                manifest_uri.origin.as_ref(),
                &tag_uri,
                manifest_uri.hash.as_bytes().to_vec(),
                &condition,
            )
            .await
        {
            Ok(ConditionalPut::Written) => return Ok(LatestAdvance::Checked),
            Ok(ConditionalPut::Moved) => {
                return Ok(latest_after_refusal(remote, manifest_uri, seen).await);
            }
            Err(err) if err.is_access_denied() => {
                log::warn!("The base check on {tag_uri} was refused; writing it unchecked");
            }
            Err(err) => return Err(err),
        }
    } else {
        log::warn!("{tag_uri} could not be read with an ETag; writing it unchecked");
    }
    tag_latest(remote, manifest_uri).await?;
    Ok(LatestAdvance::Unchecked)
}

/// Reads `latest` after S3 refused the conditional write. S3 also refuses an
/// SDK retry of a write that already landed, so a tag naming `manifest_uri`
/// counts as written. A tag still naming the revision `seen` read was
/// rewritten but not moved (a 409 whose rival write failed, or a new `ETag`
/// for the same bytes), so there is no revision it moved to.
async fn latest_after_refusal(
    remote: &impl Remote,
    manifest_uri: &ManifestUri,
    seen: &LatestSeen,
) -> LatestAdvance {
    let now = read_latest(remote, manifest_uri.origin.as_ref(), manifest_uri).await;
    match now.as_ref().ok().and_then(LatestSeen::hash) {
        Some(hash) if hash == manifest_uri.hash => LatestAdvance::Checked,
        Some(hash) if Some(hash) != seen.hash() => LatestAdvance::Moved {
            latest: Some(hash.to_owned()),
        },
        _ => LatestAdvance::Moved { latest: None },
    }
}

async fn upload_tag(remote: &impl Remote, manifest_uri: &ManifestUri, tag_uri: TagUri) -> Res {
    remote
        .put_object(
            manifest_uri.origin.as_ref(),
            &tag_uri.into(),
            manifest_uri.hash.as_bytes().to_vec(),
        )
        .await
}

/// Downloads the tagged package (latest or timestamp)
/// and returns its content: hash of the tagged package revision.
/// Then creates `ManifestUri`.
pub async fn resolve_tag(
    remote: &impl Remote,
    host: Option<&Host>,
    uri: impl Into<S3PackageHandle>,
    tag: Tag,
) -> Res<ManifestUri> {
    Ok(read_tag(remote, host, uri, tag).await?.0)
}

/// [`resolve_tag`], with the tag's `ETag` from the same read.
async fn read_tag(
    remote: &impl Remote,
    host: Option<&Host>,
    uri: impl Into<S3PackageHandle>,
    tag: Tag,
) -> Res<(ManifestUri, Option<String>)> {
    let tag_uri = TagUri::new(uri, tag);
    let stream = remote
        .get_object_stream(host, &S3Uri::from(&tag_uri))
        .await?;
    let etag = stream.etag;
    let hash = bytestream_to_string(stream.body).await?;
    if !is_top_hash(&hash) {
        return Err(UriError::Package(format!(
            "hash must be 64 lowercase hex characters, got {hash:?}"
        ))
        .into());
    }
    let S3PackageHandle { bucket, namespace } = tag_uri.into();
    let origin = host.cloned();
    Ok((
        ManifestUri {
            hash,
            bucket,
            namespace,
            origin,
        },
        etag,
    ))
}

/// `ManifestUri` should always have a whole `hash`.
/// But `S3PackageUri` can be tagged (e.g. "latest" or timestamp), or name a
/// short hash. So, we need to download the tag, or ask the registry which
/// revision the prefix names, and find out what the `hash` is
async fn resolve_top_hash(
    remote: &impl Remote,
    host: Option<&Host>,
    uri: &S3PackageUri,
) -> Res<String> {
    match &uri.revision {
        RevisionPointer::Hash(top_hash) => Ok(top_hash.clone()),
        RevisionPointer::HashPrefix(prefix) => {
            crate::flow::resolve_hash(remote, host, &uri.bucket, prefix).await
        }
        RevisionPointer::Tag(tag) => Ok(resolve_tag(remote, host, uri, tag.clone()).await?.hash),
    }
}

/// Converts `S3PackageUri` to `ManifestUri`
/// `ManifestUri` should always have a whole `hash`.
/// But `S3PackageUri` can be tagged (e.g. "latest" or timestamp), or name a
/// short hash. So, we need to download the tag, or ask the registry which
/// revision the prefix names, and find out what the `hash` is
pub async fn resolve_manifest_uri(
    remote: &impl Remote,
    host: Option<&Host>,
    uri: &S3PackageUri,
) -> Res<ManifestUri> {
    let bucket = uri.bucket.clone();
    let namespace = uri.namespace.clone();
    let hash = resolve_top_hash(remote, host, uri).await?;
    let origin = host.cloned();
    Ok(ManifestUri {
        bucket,
        namespace,
        hash,
        origin,
    })
}

/// Upload file associated with manifest's `ManifestRow`.
/// After uploading we get new hash,
/// though it should be the same as already calclulated during commit.
/// Response with the new `ManifestRow` with `physical_key` pointing to the place it was uploaded to.
///
/// A published row's `physical_key` is always remote: either rewritten below to
/// the `remote_url` this upload returns, or copied from the remote row by
/// `push::use_existing_row_or_upload`'s content-match branch. That pair — not
/// the scheme check — is what keeps a local key out of a published manifest.
///
/// The check guards the *input* instead: a row that is not local has no business
/// being uploaded, so a package re-pointed at another bucket hard-errors rather
/// than republishing another bucket's objects. (An `s3://` key would fail
/// `to_file_path` anyway; the check's own catch is host-less schemes like
/// `foo:///abs/path`.)
///
/// A failure is an [`Error::Upload`] that names the row's logical key.
pub async fn upload_row(
    remote: &impl Remote,
    host_config: &HostConfig,
    package_handle: S3PackageHandle,
    row: ManifestRow,
) -> Res<ManifestRow> {
    let key = row.logical_key.clone();
    upload_local_row(remote, host_config, package_handle, row)
        .await
        .map_err(|source| upload_error(key, source))
}

/// The error for a row whose upload failed.
fn upload_error(key: crate::LogicalKey, source: Error) -> Error {
    Error::Upload {
        key,
        source: Box::new(source),
    }
}

async fn upload_local_row(
    remote: &impl Remote,
    host_config: &HostConfig,
    package_handle: S3PackageHandle,
    row: ManifestRow,
) -> Res<ManifestRow> {
    let local_url = Url::parse(&row.physical_key)?;
    if local_url.scheme() != "file" {
        return Err(Error::Uri(UriError::FileScheme(local_url)));
    }
    let file_path = local_url
        .to_file_path()
        .map_err(|()| UriError::FileScheme(local_url))?;
    let host_config = committed_checksums(host_config, &row);
    upload_row_from_path(remote, &host_config, package_handle, row, &file_path).await
}

/// Where the bytes of a row come from when the caller supplies them, rather
/// than the row naming a `file://` path.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RowContent {
    /// A file on disk, which may be anywhere: it need not be under a package
    /// home or any managed working folder.
    Path(PathBuf),
    /// The bytes themselves, held in memory by the caller. Cloning shares the
    /// buffer.
    Bytes(Bytes),
}

impl RowContent {
    /// How many bytes this content holds now. For a file that is its size on
    /// disk at this moment.
    pub async fn size(&self, storage: &(impl Storage + Sync)) -> Res<u64> {
        match self {
            RowContent::Path(path) => Ok(file_stat(storage, path).await?.0),
            RowContent::Bytes(bytes) => Ok(bytes.len() as u64),
        }
    }
}

/// A file's size and modified time, from one open handle.
async fn file_stat(
    storage: &(impl Storage + Sync),
    path: &std::path::Path,
) -> Res<(u64, Option<std::time::SystemTime>)> {
    let meta = storage.open_file(path).await?.metadata().await?;
    Ok((meta.len(), meta.modified().ok()))
}

/// [`upload_row`], for a row whose bytes the caller supplies: the row's own
/// `physical_key` is ignored and its `size` is taken from the content.
///
/// What the row's `hash` means decides the checksum type:
///
/// - A row with no hash, [`ObjectHash::default`], was never hashed, so its
///   bytes are uploaded with the host's configured checksum type
///   (sha2-256-chunked unless the host enables `crc64Checksums`, the preferred
///   direction) and S3's
///   checksum becomes the row's hash.
/// - A row that already carries a real hash, such as one quilt3 computed in
///   memory, keeps it, with its labeled type. The bytes are uploaded with that
///   type and the checksum S3 answers with must equal the row's hash, or the
///   upload is an error: the supplied bytes are not the ones the hash was made
///   from. A type S3 cannot compute (a legacy plain SHA-256) is checked by
///   hashing the content here instead, and the row keeps its hash.
///
/// A [`RowContent::Path`] uploads straight from that file, which is read by
/// path and so cannot be read once for both its size and its content. Instead
/// its size and modified time are read before the upload and again after the
/// upload and any local hash check, and a file that changed in between is an
/// error rather than a manifest whose size and hash disagree with the bytes.
///
/// The multipart upload and the checksum S3 computes both read a file, so
/// [`RowContent::Bytes`] is staged to a private temporary file for the length
/// of the upload and removed after it, success or failure. Nothing lands in any
/// package home.
///
/// A failure is an [`Error::Upload`] that names the row's logical key.
pub async fn upload_row_content(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    host_config: &HostConfig,
    package_handle: S3PackageHandle,
    row: ManifestRow,
    content: &RowContent,
) -> Res<ManifestRow> {
    let key = row.logical_key.clone();
    upload_content(storage, remote, host_config, package_handle, row, content)
        .await
        .map_err(|source| upload_error(key, source))
}

async fn upload_content(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    host_config: &HostConfig,
    package_handle: S3PackageHandle,
    row: ManifestRow,
    content: &RowContent,
) -> Res<ManifestRow> {
    let claimed = (row.hash != ObjectHash::default()).then(|| row.hash.clone());
    let host_config = match &claimed {
        Some(_) => committed_checksums(host_config, &row),
        None => host_config.clone(),
    };
    let uploaded = match content {
        RowContent::Path(path) => {
            let before = file_stat(storage, path).await?;
            let row = ManifestRow {
                size: before.0,
                ..row
            };
            let uploaded =
                upload_row_from_path(remote, &host_config, package_handle, row, path).await?;
            // Verified inside the window the stat check covers, so the bytes
            // hashed here are the ones that were uploaded.
            let verdict = match claimed {
                Some(claimed) => verify_claimed_hash(storage, content, &uploaded, claimed).await,
                None => Ok(uploaded),
            };
            if file_stat(storage, path).await? != before {
                return Err(Error::PackageOp(PackageOpError::Push(format!(
                    "{} changed while it was being uploaded",
                    path.display()
                ))));
            }
            return verdict;
        }
        RowContent::Bytes(bytes) => {
            let staging = tempfile::tempdir()?;
            let staged = staging.path().join(uuid::Uuid::new_v4().to_string());
            storage
                .write_byte_stream(&staged, ByteStream::from(bytes.clone()))
                .await?;
            let row = ManifestRow {
                size: bytes.len() as u64,
                ..row
            };
            let uploaded =
                upload_row_from_path(remote, &host_config, package_handle, row, &staged).await;
            // Best effort: `staging` also removes the directory when it drops.
            let _ = storage.remove_file(&staged).await;
            uploaded?
        }
    };
    let Some(claimed) = claimed else {
        return Ok(uploaded);
    };
    verify_claimed_hash(storage, content, &uploaded, claimed).await
}

/// The hash of `bytes` under the multihash `algorithm`, as
/// [`hash_file`] hashes a file; an unknown algorithm is SHA-256 chunked.
pub(crate) async fn hash_bytes(bytes: &[u8], algorithm: u64) -> Res<ObjectHash> {
    let size = bytes.len() as u64;
    Ok(match algorithm {
        MULTIHASH_CRC64_NVME => Crc64Hash::from_reader(bytes, size).await?.into(),
        MULTIHASH_SHA256 => Sha256Hash::from_reader(bytes, size).await?.into(),
        _ => Sha256ChunkedHash::from_reader(bytes, size).await?.into(),
    })
}

/// Checks the bytes uploaded for `uploaded` against the hash the caller claimed
/// for them, and returns the row carrying the claimed hash.
async fn verify_claimed_hash(
    storage: &(impl Storage + Sync),
    content: &RowContent,
    uploaded: &ManifestRow,
    claimed: ObjectHash,
) -> Res<ManifestRow> {
    let algorithm = claimed.algorithm();
    let verified = if algorithm == MULTIHASH_CRC64_NVME || algorithm == MULTIHASH_SHA256_CHUNKED {
        // S3 computed the claimed type over the bytes it stored.
        uploaded.hash == claimed
    } else {
        let computed = match content {
            RowContent::Path(path) => hash_file(storage, path, algorithm).await?.0,
            RowContent::Bytes(bytes) => hash_bytes(bytes, algorithm).await?,
        };
        computed == claimed
    };
    if !verified {
        return Err(Error::PackageOp(PackageOpError::Push(format!(
            "its bytes do not hash to the hash the row carries ({claimed})"
        ))));
    }
    Ok(ManifestRow {
        hash: claimed,
        ..uploaded.clone()
    })
}

/// Uploads `row`'s bytes from `file_path` with `host_config`'s checksum type
/// as it stands: the caller picks the type.
async fn upload_row_from_path(
    remote: &impl Remote,
    host_config: &HostConfig,
    package_handle: S3PackageHandle,
    row: ManifestRow,
    file_path: &std::path::Path,
) -> Res<ManifestRow> {
    let object_uri = ObjectUri::new(package_handle, row.logical_key.to_path_buf());
    log::info!("Uploading to S3: {object_uri}");

    let (remote_url, hash) = remote
        .upload_file(host_config, file_path, &object_uri.into(), row.size)
        .await?;

    // Update the manifest with the sha2-256-chunked checksum
    // "Relax" the manifest by using those new remote keys
    let physical_key = remote_url.to_string();
    Ok(ManifestRow {
        physical_key,
        hash,
        ..row
    })
}

/// The host's config, with the checksum type the row was committed with, so
/// the uploaded row keeps the commit's hash. They differ when the remote's
/// catalog changed after the commit; S3 takes either type for any bucket. A
/// legacy SHA-256 row is uploaded with the host's type, as before.
fn committed_checksums(host_config: &HostConfig, row: &ManifestRow) -> HostConfig {
    let checksums = match row.hash.algorithm() {
        MULTIHASH_CRC64_NVME => HostChecksums::Crc64,
        MULTIHASH_SHA256_CHUNKED => HostChecksums::Sha256Chunked,
        _ => host_config.checksums.clone(),
    };
    HostConfig {
        checksums,
        ..host_config.clone()
    }
}

pub type StreamRowsChunk = Vec<Res<ManifestRow>>;

pub type StreamItem = Res<StreamRowsChunk>;

pub trait RowsStream: Stream<Item = StreamItem> {}

impl<T: Stream<Item = StreamItem>> RowsStream for T {}

/// Builds the manifest from `Stream<Result<Row>>` and the folders' `dir_meta`,
/// which the top hash leaves out.
/// It writes the manifest to temporary file using JSONL format.
/// Then it calculates `top_hash` and move the temporary file to the destination path.
pub async fn build_manifest_from_rows_stream(
    storage: &impl Storage,
    dest_dir: PathBuf,
    header: ManifestHeader,
    dir_meta: Vec<DirMeta>,
    mut stream: impl RowsStream + Unpin,
) -> Res<(PathBuf, String)> {
    let temp_dir = tempfile::tempdir()?;
    let temp_path = temp_dir.path().join("manifest.jsonl");
    log::info!("Temp path for creating manifest {}", temp_path.display());

    // Build manifest in memory
    let mut rows = Vec::new();
    let mut top_hasher = TopHasher::new();

    top_hasher.append_header(&header)?;

    while let Some(chunk) = stream.next().await {
        for row_result in &chunk? {
            match row_result {
                Ok(row) => {
                    top_hasher.append(row)?;
                    rows.push(row.clone());
                }
                Err(err) => return Err(Error::Manifest(ManifestError::Table(err.to_string()))),
            }
        }
    }

    // Create JSONL manifest
    let manifest = Manifest {
        header,
        rows,
        dir_meta,
    };
    let jsonl = manifest.try_to_jsonlines()?;

    storage
        .write_staged_byte_stream(&temp_path, ByteStream::from(jsonl.into_bytes()))
        .await?;

    let top_hash = top_hasher.finalize();
    let dest_path = dest_dir.join(&top_hash);
    storage.create_dir_all(&dest_dir).await?;
    storage.rename(temp_path, &dest_path).await?;

    Ok((dest_path, top_hash))
}

#[cfg(test)]
mod top_hash_header_tests;
#[cfg(test)]
mod top_hash_row_tests;

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use tokio_stream;

    use crate::error::HashPrefixError;
    use crate::fixtures;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::LocalStorage;
    use crate::io::storage::mocks::MockStorage;

    #[test(tokio::test)]
    async fn test_resolve_existing_hash() -> Res {
        let uri = S3PackageUri::try_from(
            "quilt+s3://b#package=foo/bar@0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef",
        )?;
        let remote = MockRemote::default();
        let top_hash = resolve_top_hash(&remote, None, &uri).await?;
        assert_eq!(
            top_hash,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_resolve_remote_hash() -> Res {
        let uri = S3PackageUri::try_from("quilt+s3://b#package=foo/bar")?;
        let remote = MockRemote::default();
        remote
            .put_object(
                None,
                &S3Uri::try_from("s3://b/.quilt/named_packages/foo/bar/latest")?,
                b"0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_vec(),
            )
            .await?;
        let top_hash = resolve_top_hash(&remote, None, &uri).await?;
        assert_eq!(
            top_hash,
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef".to_string(),
        );
        Ok(())
    }

    /// The tag's key is the commit's whole Unix seconds, rounded down, and it
    /// holds the top hash.
    #[test(tokio::test)]
    async fn a_timestamp_tag_is_named_by_whole_seconds() -> Res {
        let remote = MockRemote::default();
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("foo", "bar").into(),
            hash: ALPHA.to_string(),
            origin: None,
        };
        let time = "2024-02-20T16:00:00.999Z".parse().expect("a time");
        tag_timestamp(&remote, &manifest_uri, time).await?;
        let key = S3Uri::try_from("s3://b/.quilt/named_packages/foo/bar/1708444800")?;
        assert!(remote.exists(None, &key).await?);
        let before = "1969-12-31T23:59:59.5Z".parse().expect("a time");
        tag_timestamp(&remote, &manifest_uri, before).await?;
        let key = S3Uri::try_from("s3://b/.quilt/named_packages/foo/bar/-1")?;
        assert!(remote.exists(None, &key).await?);
        Ok(())
    }

    const ALPHA: &str = "abcdef01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const BETA: &str = "abcdef01bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn prefix_error(err: crate::Error) -> HashPrefixError {
        match err {
            crate::Error::HashPrefix(inner) => inner,
            other => panic!("expected a hash prefix error, got {other:?}"),
        }
    }

    async fn registry_with(hashes: &[&str]) -> Res<MockRemote> {
        let remote = MockRemote::default();
        for hash in hashes {
            let manifest = S3Uri::try_from(format!("s3://b/.quilt/packages/{hash}").as_str())?;
            remote.put_object(None, &manifest, b"{}".to_vec()).await?;
        }
        Ok(remote)
    }

    /// A short hash in the address resolves through the registry to the whole
    /// one, so the manifest address, and what it displays, carry all 64.
    #[test(tokio::test)]
    async fn a_short_hash_in_a_package_uri_resolves_to_the_whole_hash() -> Res {
        let remote = registry_with(&[ALPHA, BETA]).await?;
        let uri = S3PackageUri::try_from("quilt+s3://b#package=foo/bar@abcdef01a")?;

        let manifest = resolve_manifest_uri(&remote, None, &uri).await?;

        assert_eq!(manifest.hash, ALPHA);
        assert_eq!(
            manifest.to_string(),
            format!("quilt+s3://b#package=foo/bar@{ALPHA}")
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_short_hash_that_names_no_revision_or_several_is_an_error() -> Res {
        let remote = registry_with(&[ALPHA, BETA]).await?;

        let uri = S3PackageUri::try_from("quilt+s3://b#package=foo/bar@123456")?;
        let err = resolve_manifest_uri(&remote, None, &uri).await.unwrap_err();
        assert_eq!(
            prefix_error(err),
            HashPrefixError::NoMatch("123456".to_string())
        );

        let uri = S3PackageUri::try_from("quilt+s3://b#package=foo/bar@abcdef01")?;
        let err = resolve_manifest_uri(&remote, None, &uri).await.unwrap_err();
        assert_eq!(
            prefix_error(err),
            HashPrefixError::Ambiguous {
                prefix: "abcdef01".to_string(),
                matches: vec![ALPHA.to_string(), BETA.to_string()],
                more: 0,
            }
        );
        Ok(())
    }

    /// The parser refuses a bad hash, but a revision built by hand can hold
    /// one, and resolving it says what is wrong with it.
    #[test(tokio::test)]
    async fn a_hash_prefix_built_by_hand_is_checked_when_resolved() -> Res {
        let remote = registry_with(&[ALPHA]).await?;
        let mut uri = S3PackageUri::try_from("quilt+s3://b#package=foo/bar")?;

        uri.revision = RevisionPointer::HashPrefix("abc".to_string());
        let err = resolve_manifest_uri(&remote, None, &uri).await.unwrap_err();
        assert_eq!(
            prefix_error(err),
            HashPrefixError::InvalidLength("abc".to_string())
        );

        uri.revision = RevisionPointer::HashPrefix("ABCDEF".to_string());
        let err = resolve_manifest_uri(&remote, None, &uri).await.unwrap_err();
        assert_eq!(
            prefix_error(err),
            HashPrefixError::NotHex("ABCDEF".to_string())
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn resolve_tag_rejects_a_pointer_that_is_not_a_top_hash() -> Res {
        let pointer = S3Uri::try_from("s3://b/.quilt/named_packages/foo/bar/latest")?;
        let handle = S3PackageHandle {
            bucket: "b".to_string(),
            namespace: ("foo", "bar").into(),
        };
        for body in [
            "abcdef",
            "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde/",
            "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcde.",
            "",
        ] {
            let remote = MockRemote::default();
            remote
                .put_object(None, &pointer, body.as_bytes().to_vec())
                .await?;
            let result = resolve_tag(&remote, None, handle.clone(), Tag::Latest).await;
            assert!(result.is_err(), "accepted {body:?}");
        }
        Ok(())
    }

    /// A row is uploaded with the checksum type it was committed with, even
    /// when the host now asks for another, as after its catalog changed. Its
    /// hash, and so the pushed manifest's, stays the commit's.
    #[test(tokio::test)]
    async fn upload_row_keeps_the_committed_checksum_type() -> Res {
        let remote = MockRemote::default();
        let path = &std::path::absolute("/data.csv")?;
        remote
            .storage
            .write_byte_stream(path, ByteStream::from_static(b"a,b\n1,2\n"))
            .await?;
        let committed = crate::checksum::calculate_hash(
            &remote.storage,
            path,
            std::path::Path::new("data.csv"),
            &HostConfig::default(),
        )
        .await?;
        let handle = S3PackageHandle::from(&ManifestUri {
            bucket: "bkt".to_string(),
            namespace: ("test", "survey").into(),
            hash: String::new(),
            origin: None,
        });

        let uploaded = upload_row(
            &remote,
            &HostConfig::default_crc64(),
            handle,
            committed.clone(),
        )
        .await?;

        assert_eq!(uploaded.hash, committed.hash);
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_checksummed_manifest_build_from_stream() -> Res {
        let storage = LocalStorage::default();
        let manifest = Manifest::from_path(&storage, &fixtures::manifest::path()?).await?;

        let mock_storage = MockStorage::default();
        let dest_dir = mock_storage.temp_dir.path();

        let rows_stream = tokio_stream::iter(vec![Ok(manifest.rows.into_iter().map(Ok).collect())]);

        let (_, calculated_hash) = build_manifest_from_rows_stream(
            &mock_storage,
            dest_dir.to_path_buf(),
            manifest.header,
            manifest.dir_meta,
            rows_stream,
        )
        .await?;

        assert_eq!(calculated_hash, fixtures::manifest::TOP_HASH);

        Ok(())
    }

    /// A chunk that fails ends the build with its error, and no manifest is
    /// written from the rows read before it.
    #[test(tokio::test)]
    async fn test_build_fails_on_a_failed_chunk() -> Res {
        let storage = MockStorage::default();
        let dest_dir = storage.temp_dir.path().join("manifests");

        let row = ManifestRow {
            logical_key: crate::LogicalKey::new("a.txt")?,
            ..ManifestRow::default()
        };
        let rows_stream = tokio_stream::iter(vec![
            Ok(vec![Ok(row)]),
            Err(Error::PackageOp(PackageOpError::Push(
                "upload failed".to_string(),
            ))),
        ]);

        let err = build_manifest_from_rows_stream(
            &storage,
            dest_dir.clone(),
            ManifestHeader::default(),
            Vec::new(),
            rows_stream,
        )
        .await
        .unwrap_err();

        assert!(
            matches!(&err, Error::PackageOp(PackageOpError::Push(msg)) if msg == "upload failed"),
            "got: {err:?}"
        );
        assert!(!storage.exists(&dest_dir).await);
        Ok(())
    }

    /// The base check on `latest`: one package, its tag, and three revisions.
    mod base_check {
        use super::*;

        use test_log::test;

        const OURS: &str = "1111111111111111111111111111111111111111111111111111111111111111";
        const BASE: &str = "2222222222222222222222222222222222222222222222222222222222222222";
        const THEIRS: &str = "3333333333333333333333333333333333333333333333333333333333333333";
        const TAG: &str = "s3://b/.quilt/named_packages/foo/bar/latest";

        fn handle() -> S3PackageHandle {
            S3PackageHandle {
                bucket: "b".to_string(),
                namespace: ("foo", "bar").into(),
            }
        }

        fn revision(hash: &str) -> ManifestUri {
            ManifestUri {
                bucket: "b".to_string(),
                namespace: ("foo", "bar").into(),
                hash: hash.to_string(),
                origin: None,
            }
        }

        async fn set_latest(remote: &MockRemote, hash: &str) -> Res {
            remote
                .put_object(None, &S3Uri::try_from(TAG)?, hash.as_bytes().to_vec())
                .await
        }

        async fn latest(remote: &MockRemote) -> Res<String> {
            Ok(resolve_tag(remote, None, handle(), Tag::Latest).await?.hash)
        }

        #[test(tokio::test)]
        async fn an_unchanged_latest_is_advanced_checked() -> Res {
            let remote = MockRemote::default();
            set_latest(&remote, BASE).await?;
            let seen = read_latest(&remote, None, handle()).await?;
            assert_eq!(seen.hash(), Some(BASE));

            let advance = advance_latest(&remote, &revision(OURS), &seen).await?;

            assert_eq!(advance, LatestAdvance::Checked);
            assert_eq!(latest(&remote).await?, OURS);
            Ok(())
        }

        #[test(tokio::test)]
        async fn a_latest_moved_since_the_read_is_left_as_it_is() -> Res {
            let remote = MockRemote::default();
            set_latest(&remote, BASE).await?;
            let seen = read_latest(&remote, None, handle()).await?;
            // Another publisher moves `latest` between this read and the write.
            set_latest(&remote, THEIRS).await?;

            let advance = advance_latest(&remote, &revision(OURS), &seen).await?;

            assert_eq!(
                advance,
                LatestAdvance::Moved {
                    latest: Some(THEIRS.to_string())
                }
            );
            assert_eq!(latest(&remote).await?, THEIRS);
            Ok(())
        }

        /// S3 refuses an SDK retry of a write that already landed; the tag
        /// then names this revision, so the write counts.
        #[test(tokio::test)]
        async fn a_refused_write_that_already_landed_is_checked() -> Res {
            let remote = MockRemote::default();
            set_latest(&remote, BASE).await?;
            let seen = read_latest(&remote, None, handle()).await?;
            set_latest(&remote, OURS).await?;

            let advance = advance_latest(&remote, &revision(OURS), &seen).await?;

            assert_eq!(advance, LatestAdvance::Checked);
            Ok(())
        }

        /// A refusal while `latest` still names the revision first read
        /// reports no revision it moved to.
        #[test(tokio::test)]
        async fn a_refused_write_over_the_same_revision_names_none() -> Res {
            let remote = MockRemote::default();
            set_latest(&remote, BASE).await?;
            let seen = LatestSeen::Tagged {
                uri: revision(BASE),
                etag: Some("\"rewritten since\"".to_string()),
            };

            let advance = advance_latest(&remote, &revision(OURS), &seen).await?;

            assert_eq!(advance, LatestAdvance::Moved { latest: None });
            assert_eq!(latest(&remote).await?, BASE);
            Ok(())
        }

        #[test(tokio::test)]
        async fn an_absent_latest_is_created_checked() -> Res {
            let remote = MockRemote::default();
            let seen = read_latest(&remote, None, handle()).await?;
            assert_eq!(seen, LatestSeen::Absent);

            let advance = advance_latest(&remote, &revision(OURS), &seen).await?;

            assert_eq!(advance, LatestAdvance::Checked);
            assert_eq!(latest(&remote).await?, OURS);
            Ok(())
        }

        /// Two first publishers both read no `latest`; only the first to write
        /// it creates it.
        #[test(tokio::test)]
        async fn a_latest_created_since_the_read_is_left_as_it_is() -> Res {
            let remote = MockRemote::default();
            let seen = read_latest(&remote, None, handle()).await?;
            set_latest(&remote, THEIRS).await?;

            let advance = advance_latest(&remote, &revision(OURS), &seen).await?;

            assert_eq!(
                advance,
                LatestAdvance::Moved {
                    latest: Some(THEIRS.to_string())
                }
            );
            assert_eq!(latest(&remote).await?, THEIRS);
            Ok(())
        }

        /// A role that may write the tag and not read it cannot make the
        /// check, so it writes plainly and says so.
        #[test(tokio::test)]
        async fn an_unreadable_latest_is_written_unchecked() -> Res {
            let remote = MockRemote::default();
            set_latest(&remote, THEIRS).await?;
            remote.deny_reads(TAG);
            let seen = read_latest(&remote, None, handle()).await;
            assert_eq!(seen.ok(), Some(LatestSeen::Unreadable));

            let advance = advance_latest(&remote, &revision(OURS), &LatestSeen::Unreadable).await?;

            assert_eq!(advance, LatestAdvance::Unchecked);
            assert_eq!(remote.current(TAG).await?, Some(OURS.as_bytes().to_vec()));
            Ok(())
        }

        /// The read succeeded but the conditional form is refused: the write
        /// still happens, unchecked, rather than failing the publish.
        #[test(tokio::test)]
        async fn a_refused_conditional_write_falls_back_unchecked() -> Res {
            let remote = MockRemote::default();
            set_latest(&remote, BASE).await?;
            let seen = read_latest(&remote, None, handle()).await?;
            remote.deny_reads(TAG);

            let advance = advance_latest(&remote, &revision(OURS), &seen).await;

            assert_eq!(advance.ok(), Some(LatestAdvance::Unchecked));
            assert_eq!(remote.current(TAG).await?, Some(OURS.as_bytes().to_vec()));
            Ok(())
        }

        /// With no `ETag` from the read there is nothing to condition on.
        #[test(tokio::test)]
        async fn a_read_without_an_etag_is_written_unchecked() -> Res {
            let remote = MockRemote::default();
            set_latest(&remote, THEIRS).await?;
            let seen = LatestSeen::Tagged {
                uri: revision(BASE),
                etag: None,
            };

            let advance = advance_latest(&remote, &revision(OURS), &seen).await?;

            assert_eq!(advance, LatestAdvance::Unchecked);
            assert_eq!(latest(&remote).await?, OURS);
            Ok(())
        }
    }
}
