use std::pin::Pin;
use std::task::Context;
use std::task::Poll;

use aws_sdk_s3::primitives::ByteStream;
use bytes::Bytes;
use serde::de::DeserializeOwned;
use tokio::io::AsyncWriteExt;
use tokio::sync::mpsc;
use tokio_stream::Stream;
use tokio_stream::StreamExt;
use tracing::debug;

use crate::Error;
use crate::Res;
use crate::checksum::hash_reader;
use crate::error::ChecksumError;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::ManifestRow;
use crate::object_hash::ObjectHash;
use crate::paths::DomainPaths;
use quilt_uri::Host;
use quilt_uri::S3Uri;

/// How [`read_object`] treats the object's hash.
///
/// Build one with [`default`](Default::default) or
/// [`unverified`](Self::unverified); more options may be added.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ReadOptions {
    /// Check the bytes against the row's hash as they stream, and end the
    /// stream with an error if they do not match. On by default; turn it off
    /// to read what the remote returns as it is.
    pub verify_hash: bool,
}

impl Default for ReadOptions {
    fn default() -> Self {
        Self { verify_hash: true }
    }
}

impl ReadOptions {
    /// Read without checking the hash.
    #[must_use]
    pub const fn unverified() -> Self {
        Self { verify_hash: false }
    }
}

/// The bytes of one object, as a stream of chunks.
///
/// When the read verifies the hash, the chunks arrive as they are fetched and
/// the check finishes at the end: a stream that does not hash to its row
/// yields every chunk, then one final `Err`. So an item is trustworthy only
/// once the stream has ended without an error; use [`bytes`](Self::bytes),
/// [`string`](Self::string) or [`json`](Self::json) when you want the whole
/// object and would rather get nothing than bytes that failed the check.
///
/// **Bytes read from the object store are not hash-checked**; see
/// [`read_object`].
///
/// Dropping the stream early stops the fetch, and the check never completes.
pub struct ObjectBody {
    chunks: mpsc::Receiver<Res<Bytes>>,
}

impl Stream for ObjectBody {
    type Item = Res<Bytes>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        self.chunks.poll_recv(cx)
    }
}

impl ObjectBody {
    /// The whole object, or the error that ended its stream.
    pub async fn bytes(mut self) -> Res<Vec<u8>> {
        let mut all = Vec::new();
        while let Some(chunk) = self.next().await {
            all.extend_from_slice(&chunk?);
        }
        Ok(all)
    }

    /// The whole object as UTF-8 text.
    pub async fn string(self) -> Res<String> {
        String::from_utf8(self.bytes().await?).map_err(|err| Error::Utf8(err.utf8_error()))
    }

    /// The whole object parsed as JSON.
    pub async fn json<T: DeserializeOwned>(self) -> Res<T> {
        Ok(serde_json::from_slice(&self.bytes().await?)?)
    }
}

/// Opens the object `row` describes for reading, checking its hash as the
/// bytes stream.
///
/// A file in `store` for the row's hash is read from there **without a hash
/// check**: install filed it after verifying it, but a commit also files a copy
/// of a working file it hashed earlier and does not hash again. Only the file's
/// length is compared to the row's `size`; if it differs the store is ignored
/// and the remote is read and verified. The right length with wrong bytes is
/// not caught.
///
/// Otherwise the bytes come from the row's `s3://` `physical_key` and are
/// verified unless `options` turns that off. Nothing is written to the store.
/// The hash uses the row's algorithm, as
/// [`refresh_hash`](crate::checksum::refresh_hash) does for a file, fed from the
/// stream in a second task, so the object is read once.
///
/// What the stream's last item can be:
///
/// - [`ChecksumError::HashMismatch`]: the bytes do not hash to the row. With
///   no `versionId` on the `physical_key` the object may have been replaced by a
///   later revision, and the message says so, as `install_paths` does; with a
///   pinned version the recorded hash is wrong.
/// - [`ChecksumError::SizeMismatch`]: the stream is not the row's size. With no
///   `versionId` it also says the object may have been replaced.
/// - Any other error, such as a failed read or a hasher that could not run, is
///   not a verdict on the bytes.
pub async fn read_object(
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    host: Option<&Host>,
    store: Option<&DomainPaths>,
    row: &ManifestRow,
    options: ReadOptions,
) -> Res<ObjectBody> {
    if let Some(paths) = store {
        let held = paths.object(row.hash.digest());
        if storage.exists(&held).await {
            match stored_size(storage, &held).await {
                Ok(size) if size == row.size => {
                    debug!("✔️ Reading {} from the object store", row.logical_key);
                    let body = storage.read_byte_stream(&held).await?;
                    return Ok(pump(body, None));
                }
                other => debug!(
                    "Stored file for {} is not usable ({other:?} bytes, row says {}): reading the remote",
                    row.logical_key, row.size
                ),
            }
        }
    }
    let uri: S3Uri = row.physical_key.parse()?;
    let body = remote.get_object_stream(host, &uri).await?.body;
    let check = options.verify_hash.then(|| Check {
        expected: row.hash.clone(),
        size: row.size,
        key: row.logical_key.to_string(),
        replaced: matches!(uri.version.as_deref(), None | Some("null")),
    });
    Ok(pump(body, check))
}

async fn stored_size(storage: &(impl Storage + Sync), path: &std::path::Path) -> Res<u64> {
    Ok(storage.open_file(path).await?.metadata().await?.len())
}

/// What the stream must hash to.
struct Check {
    expected: ObjectHash,
    size: u64,
    key: String,
    /// The `physical_key` names no exact version.
    replaced: bool,
}

/// Moves `body` to the caller a chunk at a time, feeding the hasher the same
/// chunks, and ends with the verdict.
///
/// A task of its own, so the hasher reads from a pipe while the chunks flow on.
/// The pipe is bounded: a hasher slower than the network slows the fetch rather
/// than buffering the object.
fn pump(mut body: ByteStream, check: Option<Check>) -> ObjectBody {
    let (tx, chunks) = mpsc::channel(4);
    tokio::spawn(async move {
        let hashing = check.map(|check| {
            let (pipe, read_end) = tokio::io::duplex(64 * 1024);
            let algorithm = check.expected.algorithm();
            let hasher = tokio::spawn(hash_reader(read_end, check.size, algorithm));
            (pipe, hasher, check)
        });
        let (mut pipe, mut hashing) = match hashing {
            Some((pipe, hasher, check)) => (Some(pipe), Some((hasher, check))),
            None => (None, None),
        };
        let mut streamed: u64 = 0;
        loop {
            // A dropped reader stops the fetch at once, not at the next chunk.
            let next = tokio::select! {
                () = tx.closed() => return,
                next = body.try_next() => next,
            };
            match next {
                Ok(Some(chunk)) => {
                    streamed += chunk.len() as u64;
                    if let Some(open) = pipe.as_mut()
                        && open.write_all(&chunk).await.is_err()
                    {
                        // The hasher stopped early; its verdict follows below.
                        pipe = None;
                    }
                    if tx.send(Ok(chunk)).await.is_err() {
                        return;
                    }
                }
                Ok(None) => break,
                Err(err) => {
                    let _ = tx.send(Err(err.into())).await;
                    return;
                }
            }
        }
        drop(pipe);
        if let Some((hasher, check)) = hashing.take() {
            // The chunked hash reads only as many bytes as the size it is told,
            // so bytes past the end would not change it: the length is judged
            // first, and a hasher fed too few bytes fails for the same reason.
            let hashed = hasher.await;
            let verdict = if streamed == check.size {
                match hashed {
                    Ok(Ok(computed)) if computed == check.expected => None,
                    Ok(Ok(computed)) => Some(Error::Checksum(ChecksumError::HashMismatch {
                        key: check.key,
                        expected: check.expected.to_string(),
                        computed: computed.to_string(),
                        replaced: check.replaced,
                    })),
                    // Not a verdict on the bytes: the hasher could not finish.
                    Ok(Err(err)) => Some(err),
                    Err(err) => Some(Error::Io(std::io::Error::other(err))),
                }
            } else {
                Some(Error::Checksum(ChecksumError::SizeMismatch {
                    key: check.key,
                    expected: check.size,
                    actual: streamed,
                    replaced: check.replaced,
                }))
            };
            if let Some(verdict) = verdict {
                let _ = tx.send(Err(verdict)).await;
            }
        }
    });
    ObjectBody { chunks }
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use aws_smithy_types::body::SdkBody;
    use std::collections::VecDeque;
    use std::path::Path;
    use std::sync::Arc;
    use std::sync::atomic::AtomicBool;
    use std::sync::atomic::Ordering;

    use crate::checksum::hash_file;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::object_hash::MULTIHASH_CRC64_NVME;
    use crate::object_hash::MULTIHASH_SHA256;
    use crate::object_hash::MULTIHASH_SHA256_CHUNKED;

    const KEY: &str = "s3://b/p/k.bin?versionId=v1";

    /// Bytes spread over several 8 MiB parts, so the chunked hash has more than
    /// one part to get right.
    fn body(len: usize) -> Vec<u8> {
        (0..len)
            .map(|i| u8::try_from(i.wrapping_mul(31) % 251).unwrap())
            .collect()
    }

    /// The row `body` has under `algorithm`, as the manifest of a package that
    /// holds it would record.
    async fn row_of(storage: &MockStorage, body: &[u8], algorithm: u64) -> Res<ManifestRow> {
        let path = Path::new("/hash-me");
        storage
            .write_byte_stream(path, ByteStream::from(body.to_vec()))
            .await?;
        let (hash, size) = hash_file(storage, path, algorithm).await?;
        Ok(ManifestRow {
            logical_key: crate::LogicalKey::new("k.bin")?,
            physical_key: KEY.to_string(),
            hash,
            size,
            meta: None,
            meta_numbers: crate::manifest::RawNumbers::default(),
        })
    }

    async fn put(remote: &MockRemote, body: &[u8]) -> Res {
        remote
            .put_object(None, &S3Uri::try_from(KEY)?, body.to_vec())
            .await
    }

    async fn read(
        remote: &MockRemote,
        store: Option<&DomainPaths>,
        row: &ManifestRow,
        options: ReadOptions,
    ) -> Res<ObjectBody> {
        read_object(&remote.storage, remote, None, store, row, options).await
    }

    const ALGORITHMS: [u64; 3] = [
        MULTIHASH_SHA256_CHUNKED,
        MULTIHASH_SHA256,
        MULTIHASH_CRC64_NVME,
    ];

    #[test(tokio::test)]
    async fn an_object_that_matches_its_row_reads_back_whole() -> Res {
        let contents = body(9 * 1024 * 1024 + 17);
        for algorithm in ALGORITHMS {
            let remote = MockRemote::default();
            let row = row_of(&remote.storage, &contents, algorithm).await?;
            put(&remote, &contents).await?;

            let read = read(&remote, None, &row, ReadOptions::default()).await?;

            assert!(read.bytes().await? == contents, "algorithm {algorithm:#x}");
        }
        Ok(())
    }

    #[test(tokio::test)]
    async fn an_empty_object_reads_back_empty() -> Res {
        for algorithm in ALGORITHMS {
            let remote = MockRemote::default();
            let row = row_of(&remote.storage, b"", algorithm).await?;
            put(&remote, b"").await?;

            let read = read(&remote, None, &row, ReadOptions::default()).await?;

            assert_eq!(read.bytes().await?, Vec::<u8>::new(), "{algorithm:#x}");
        }
        Ok(())
    }

    /// The check is at the end: every chunk reaches the caller first, then one
    /// error closes the stream.
    #[test(tokio::test)]
    async fn bytes_that_do_not_match_arrive_and_then_the_stream_errors() -> Res {
        let remote = MockRemote::default();
        let row = row_of(&remote.storage, b"what the row says", MULTIHASH_SHA256).await?;
        put(&remote, b"what the box says").await?;

        let mut stream = read(&remote, None, &row, ReadOptions::default()).await?;

        let mut received = Vec::new();
        let mut verdict = None;
        while let Some(item) = stream.next().await {
            match item {
                Ok(chunk) => received.extend_from_slice(&chunk),
                Err(err) => verdict = Some(err),
            }
        }
        assert_eq!(received, b"what the box says");
        let Some(Error::Checksum(ChecksumError::HashMismatch {
            key,
            replaced,
            expected,
            computed,
        })) = verdict
        else {
            panic!("expected a hash mismatch at the end, got {verdict:?}");
        };
        assert_eq!(key, "k.bin");
        assert_ne!(expected, computed);
        // A pinned version cannot have been replaced: the recorded hash is wrong.
        assert!(!replaced);
        Ok(())
    }

    /// The words are `install_paths`': a key that names no version may have been
    /// replaced since, one that names a version may not.
    #[test(tokio::test)]
    async fn a_hash_mismatch_is_worded_by_whether_the_key_names_a_version() -> Res {
        for (physical_key, replaced, wording) in [
            (KEY, false, "the package may be corrupt"),
            ("s3://b/p/k.bin", true, "has been replaced"),
            ("s3://b/p/k.bin?versionId=null", true, "has been replaced"),
        ] {
            let remote = MockRemote::default();
            let mut row = row_of(&remote.storage, b"what the row says", MULTIHASH_SHA256).await?;
            row.physical_key = physical_key.to_string();
            remote
                .put_object(
                    None,
                    &S3Uri::try_from(physical_key)?,
                    b"what the box says".to_vec(),
                )
                .await?;

            let outcome = read(&remote, None, &row, ReadOptions::default())
                .await?
                .bytes()
                .await;

            let Err(Error::Checksum(error @ ChecksumError::HashMismatch { .. })) = outcome else {
                panic!("{physical_key}: expected a hash mismatch, got {outcome:?}");
            };
            let ChecksumError::HashMismatch { replaced: got, .. } = &error else {
                unreachable!()
            };
            assert_eq!(*got, replaced, "{physical_key}");
            let message = error.to_string();
            assert!(message.contains(wording), "{message}");
            assert!(message.contains("k.bin"), "{message}");
        }
        Ok(())
    }

    #[test(tokio::test)]
    async fn the_whole_object_helpers_return_the_error_not_the_bytes() -> Res {
        let remote = MockRemote::default();
        let row = row_of(&remote.storage, b"what the row says", MULTIHASH_SHA256).await?;
        put(&remote, b"what the box says").await?;

        for helper in 0..3 {
            let body = read(&remote, None, &row, ReadOptions::default()).await?;
            let outcome = match helper {
                0 => body.bytes().await.map(|_| ()),
                1 => body.string().await.map(|_| ()),
                _ => body.json::<serde_json::Value>().await.map(|_| ()),
            };
            assert!(
                matches!(outcome, Err(Error::Checksum(_))),
                "helper {helper}: {outcome:?}"
            );
        }
        Ok(())
    }

    /// Extra or missing bytes are a size mismatch, for any algorithm, and say
    /// how long the object really was.
    #[test(tokio::test)]
    async fn an_object_longer_or_shorter_than_its_row_is_a_size_mismatch() -> Res {
        let contents = body(9 * 1024 * 1024);
        let mut longer = contents.clone();
        longer.extend_from_slice(b"trailing");
        let shorter = contents[..contents.len() - 1].to_vec();
        for algorithm in ALGORITHMS {
            for served in [&longer, &shorter] {
                let remote = MockRemote::default();
                let row = row_of(&remote.storage, &contents, algorithm).await?;
                put(&remote, served).await?;

                let read = read(&remote, None, &row, ReadOptions::default()).await?;

                let outcome = read.bytes().await;
                match outcome {
                    Err(Error::Checksum(ChecksumError::SizeMismatch {
                        key,
                        expected,
                        actual,
                        replaced,
                    })) => {
                        assert_eq!(key, "k.bin");
                        assert!(!replaced, "KEY pins a version");
                        assert_eq!(expected, contents.len() as u64);
                        assert_eq!(actual, served.len() as u64, "algorithm {algorithm:#x}");
                    }
                    other => panic!(
                        "algorithm {algorithm:#x}, {} bytes: {:?}",
                        served.len(),
                        other.map(|bytes| bytes.len())
                    ),
                }
            }
        }
        Ok(())
    }

    /// As for a hash mismatch, a key that names no version says the object may
    /// have been replaced since, and one that names a version does not.
    #[test(tokio::test)]
    async fn a_size_mismatch_is_worded_by_whether_the_key_names_a_version() -> Res {
        for (physical_key, replaced) in [
            (KEY, false),
            ("s3://b/p/k.bin", true),
            ("s3://b/p/k.bin?versionId=null", true),
        ] {
            let remote = MockRemote::default();
            let mut row = row_of(&remote.storage, b"what the row says", MULTIHASH_SHA256).await?;
            row.physical_key = physical_key.to_string();
            remote
                .put_object(None, &S3Uri::try_from(physical_key)?, b"too short".to_vec())
                .await?;

            let outcome = read(&remote, None, &row, ReadOptions::default())
                .await?
                .bytes()
                .await;

            let Err(Error::Checksum(error @ ChecksumError::SizeMismatch { .. })) = outcome else {
                panic!("{physical_key}: expected a size mismatch, got {outcome:?}");
            };
            let ChecksumError::SizeMismatch { replaced: got, .. } = &error else {
                unreachable!()
            };
            assert_eq!(*got, replaced, "{physical_key}");
            let message = error.to_string();
            assert!(message.contains("k.bin is 9 bytes"), "{message}");
            assert_eq!(
                message.contains("may have been replaced"),
                replaced,
                "{message}"
            );
        }
        Ok(())
    }

    #[test(tokio::test)]
    async fn the_check_can_be_turned_off() -> Res {
        let remote = MockRemote::default();
        let row = row_of(&remote.storage, b"what the row says", MULTIHASH_SHA256).await?;
        put(&remote, b"what the box says").await?;

        let read = read(&remote, None, &row, ReadOptions::unverified()).await?;

        assert_eq!(read.bytes().await?, b"what the box says");
        Ok(())
    }

    #[test]
    fn the_check_is_on_by_default() {
        assert!(ReadOptions::default().verify_hash);
        assert!(!ReadOptions::unverified().verify_hash);
    }

    /// An object the store already holds, of the row's size, is read from it
    /// unchecked: install verified it on the way in, and a commit filed a copy of
    /// a file it had hashed. The remote is not asked.
    #[test(tokio::test)]
    async fn an_object_the_store_holds_is_read_from_it_without_the_check() -> Res {
        let remote = MockRemote::default();
        let row = row_of(&remote.storage, b"what the row says", MULTIHASH_SHA256).await?;
        let (paths, _dir) = DomainPaths::from_temp_dir()?;
        // Not the row's bytes, so only a read that skips the check returns them.
        remote
            .storage
            .write_byte_stream(
                paths.object(row.hash.digest()),
                ByteStream::from_static(b"kept in the store"),
            )
            .await?;

        let read = read(&remote, Some(&paths), &row, ReadOptions::default()).await?;

        assert_eq!(read.bytes().await?, b"kept in the store");
        assert_eq!(remote.get_object_count(KEY), 0);
        Ok(())
    }

    #[test(tokio::test)]
    async fn an_object_the_store_lacks_is_fetched_and_checked() -> Res {
        let remote = MockRemote::default();
        let row = row_of(&remote.storage, b"what the row says", MULTIHASH_SHA256).await?;
        put(&remote, b"what the box says").await?;
        let (paths, _dir) = DomainPaths::from_temp_dir()?;

        let read = read(&remote, Some(&paths), &row, ReadOptions::default()).await?;

        assert!(matches!(read.bytes().await, Err(Error::Checksum(_))));
        assert_eq!(remote.get_object_count(KEY), 1);
        // Reading files nothing in the store.
        assert!(!remote.storage.exists(paths.object(row.hash.digest())).await);
        Ok(())
    }

    /// A stored file that is not the row's size is not trusted: the read goes to
    /// the remote and is verified, as if the store had nothing.
    #[test(tokio::test)]
    async fn a_stored_file_of_the_wrong_size_is_read_from_the_remote_instead() -> Res {
        let remote = MockRemote::default();
        let contents = b"what the row says";
        let row = row_of(&remote.storage, contents, MULTIHASH_SHA256).await?;
        put(&remote, contents).await?;
        let (paths, _dir) = DomainPaths::from_temp_dir()?;
        for stored in [&b"cut short"[..], b"a stored file that is far too long"] {
            remote
                .storage
                .write_byte_stream(
                    paths.object(row.hash.digest()),
                    ByteStream::from(stored.to_vec()),
                )
                .await?;
            let before = remote.get_object_count(KEY);

            let read = read(&remote, Some(&paths), &row, ReadOptions::default()).await?;

            assert_eq!(read.bytes().await?, contents);
            assert_eq!(remote.get_object_count(KEY), before + 1);
        }
        Ok(())
    }

    /// A remote object delivered in the pieces a test chooses, and then, if it
    /// is told to, a failure instead of the end.
    struct Pieces {
        inner: MockRemote,
        parts: Vec<bytes::Bytes>,
        then_fail: bool,
        /// Never ends, and sets this when the body is dropped.
        stall_until_dropped: Option<Arc<AtomicBool>>,
    }

    struct PiecesBody {
        pieces: VecDeque<bytes::Bytes>,
        then_fail: bool,
        stalled: Option<Arc<AtomicBool>>,
    }

    impl Drop for PiecesBody {
        fn drop(&mut self) {
            if let Some(dropped) = &self.stalled {
                dropped.store(true, Ordering::SeqCst);
            }
        }
    }

    impl http_body::Body for PiecesBody {
        type Data = bytes::Bytes;
        type Error = Box<dyn std::error::Error + Send + Sync>;

        fn poll_frame(
            mut self: Pin<&mut Self>,
            _cx: &mut Context<'_>,
        ) -> Poll<Option<Result<http_body::Frame<Self::Data>, Self::Error>>> {
            if let Some(piece) = self.pieces.pop_front() {
                return Poll::Ready(Some(Ok(http_body::Frame::data(piece))));
            }
            if self.stalled.is_some() {
                return Poll::Pending;
            }
            if std::mem::take(&mut self.then_fail) {
                let broken = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "reset");
                return Poll::Ready(Some(Err(Box::new(broken))));
            }
            Poll::Ready(None)
        }
    }

    impl Remote for Pieces {
        async fn exists(&self, host: Option<&Host>, uri: &S3Uri) -> Res<bool> {
            self.inner.exists(host, uri).await
        }

        #[allow(clippy::unused_async_trait_impl)]
        async fn get_object_stream(
            &self,
            _host: Option<&Host>,
            uri: &S3Uri,
        ) -> Res<crate::io::remote::RemoteObjectStream> {
            let body = PiecesBody {
                pieces: self.parts.iter().cloned().collect(),
                then_fail: self.then_fail,
                stalled: self.stall_until_dropped.clone(),
            };
            Ok(crate::io::remote::RemoteObjectStream {
                body: ByteStream::new(SdkBody::from_body_1_x(body)),
                uri: uri.clone(),
            })
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
            self.inner.put_object(host, uri, contents).await
        }

        async fn upload_file(
            &self,
            host_config: &crate::io::remote::HostConfig,
            source_path: impl AsRef<Path>,
            dest_uri: &S3Uri,
            size: u64,
        ) -> Res<(S3Uri, ObjectHash)> {
            self.inner
                .upload_file(host_config, source_path, dest_uri, size)
                .await
        }

        async fn host_config(&self, host: Option<&Host>) -> Res<crate::io::remote::HostConfig> {
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
    }

    fn in_pieces(contents: &[u8], size: usize) -> Vec<bytes::Bytes> {
        contents
            .chunks(size)
            .map(bytes::Bytes::copy_from_slice)
            .collect()
    }

    /// Many chunks reach the caller one by one, in order, and the check
    /// still passes on the whole.
    #[test(tokio::test)]
    async fn a_stream_of_many_chunks_is_checked_as_a_whole() -> Res {
        let contents = body(300_000);
        for algorithm in ALGORITHMS {
            let source = MockRemote::default();
            let row = row_of(&source.storage, &contents, algorithm).await?;
            let remote = Pieces {
                inner: source,
                parts: in_pieces(&contents, 7_001),
                then_fail: false,
                stall_until_dropped: None,
            };

            let mut stream = read_object(
                &remote.inner.storage,
                &remote,
                None,
                None,
                &row,
                ReadOptions::default(),
            )
            .await?;

            let mut received = Vec::new();
            let mut chunks = 0;
            while let Some(item) = stream.next().await {
                received.extend_from_slice(&item?);
                chunks += 1;
            }
            assert_eq!(chunks, remote.parts.len(), "algorithm {algorithm:#x}");
            assert!(received == contents, "algorithm {algorithm:#x}");
        }
        Ok(())
    }

    /// A failure partway through is passed on as the error it is, after the
    /// chunks that did arrive. It is not a verdict on the hash, and nothing
    /// follows it.
    #[test(tokio::test)]
    async fn a_remote_that_fails_mid_stream_ends_the_stream_with_that_error() -> Res {
        let contents = body(60_000);
        for algorithm in ALGORITHMS {
            let source = MockRemote::default();
            let row = row_of(&source.storage, &contents, algorithm).await?;
            let mut pieces = in_pieces(&contents, 10_000);
            pieces.truncate(2);
            let remote = Pieces {
                inner: source,
                parts: pieces,
                then_fail: true,
                stall_until_dropped: None,
            };

            let mut stream = read_object(
                &remote.inner.storage,
                &remote,
                None,
                None,
                &row,
                ReadOptions::default(),
            )
            .await?;

            let mut received = Vec::new();
            let mut failure = None;
            while let Some(item) = stream.next().await {
                assert!(failure.is_none(), "an item followed the error");
                match item {
                    Ok(chunk) => received.extend_from_slice(&chunk),
                    Err(err) => failure = Some(err),
                }
            }
            assert!(received == contents[..20_000], "algorithm {algorithm:#x}");
            let failure = failure.expect("the stream ends with the error");
            assert!(
                !matches!(failure, Error::Checksum(_)),
                "algorithm {algorithm:#x}: {failure:?}"
            );
        }
        Ok(())
    }

    /// Dropping the stream while the remote has nothing more to send releases
    /// the fetch then, without waiting for a chunk that may never come.
    #[test(tokio::test)]
    async fn dropping_the_stream_stops_a_stalled_fetch() -> Res {
        let contents = body(20_000);
        for algorithm in ALGORITHMS {
            let source = MockRemote::default();
            let row = row_of(&source.storage, &contents, algorithm).await?;
            let released = Arc::new(AtomicBool::new(false));
            let remote = Pieces {
                inner: source,
                parts: in_pieces(&contents, 10_000),
                then_fail: false,
                stall_until_dropped: Some(Arc::clone(&released)),
            };
            let mut stream = read_object(
                &remote.inner.storage,
                &remote,
                None,
                None,
                &row,
                ReadOptions::default(),
            )
            .await?;
            // The two chunks arrive, then the remote stalls.
            for _ in 0..2 {
                assert!(stream.next().await.is_some());
            }
            assert!(!released.load(Ordering::SeqCst));

            drop(stream);

            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while !released.load(Ordering::SeqCst) {
                assert!(
                    std::time::Instant::now() < deadline,
                    "algorithm {algorithm:#x}: the fetch outlived its reader"
                );
                tokio::time::sleep(std::time::Duration::from_millis(10)).await;
            }
        }
        Ok(())
    }

    #[test(tokio::test)]
    async fn text_and_json_helpers_read_a_matching_object() -> Res {
        let remote = MockRemote::default();
        let document = br#"{"a": [1, 2]}"#;
        let row = row_of(&remote.storage, document, MULTIHASH_CRC64_NVME).await?;
        put(&remote, document).await?;

        let text = read(&remote, None, &row, ReadOptions::default()).await?;
        assert_eq!(text.string().await?, r#"{"a": [1, 2]}"#);
        let json = read(&remote, None, &row, ReadOptions::default()).await?;
        assert_eq!(
            json.json::<serde_json::Value>().await?,
            serde_json::json!({"a": [1, 2]})
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn an_object_the_remote_does_not_have_is_not_found() -> Res {
        let remote = MockRemote::default();
        let row = row_of(&remote.storage, b"x", MULTIHASH_SHA256).await?;

        let read = read(&remote, None, &row, ReadOptions::default()).await;

        assert!(read.is_err_and(|err| err.is_not_found()));
        Ok(())
    }

    /// Prints what verifying costs on a large object, which depends on the
    /// machine and on a release build, so it is not run by default:
    /// `just test no-aws --release --run-ignored only --no-capture -E 'test(cost_of_verifying)'`
    #[test(tokio::test)]
    #[ignore = "a benchmark: it reports timings, and asserts nothing"]
    async fn cost_of_verifying_a_large_object() -> Res {
        let mebibytes: u32 = std::env::var("QUILT_READ_BENCH_MIB")
            .ok()
            .and_then(|value| value.parse().ok())
            .unwrap_or(512);
        let contents = body(mebibytes as usize * 1024 * 1024);
        for algorithm in ALGORITHMS {
            let remote = MockRemote::default();
            let row = row_of(&remote.storage, &contents, algorithm).await?;
            put(&remote, &contents).await?;
            let mut seconds = Vec::new();
            for options in [ReadOptions::unverified(), ReadOptions::default()] {
                let started = std::time::Instant::now();
                let read = read(&remote, None, &row, options).await?;
                assert_eq!(read.bytes().await?.len(), contents.len());
                seconds.push(started.elapsed().as_secs_f64());
            }
            println!(
                "{mebibytes} MiB, algorithm {algorithm:#x}: unverified {:.2}s, verified {:.2}s ({:.0} MiB/s verified)",
                seconds[0],
                seconds[1],
                f64::from(mebibytes) / seconds[1]
            );
        }
        Ok(())
    }
}
