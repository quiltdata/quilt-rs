use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use aws_sdk_s3::primitives::ByteStream;
use tracing::log;

use crate::Error;
use crate::error::FsError;
use crate::error::S3Error;
use crate::error::S3ErrorKind;
use crate::io::remote::HostConfig;
use crate::io::remote::RemoteObjectStream;
use crate::io::storage::LocalStorage;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;
use crate::io::storage::mocks::MockStorage;
use crate::object_hash::ObjectHash;
use quilt_uri::Host;
use quilt_uri::Namespace;
use quilt_uri::S3Uri;
use quilt_uri::paths::tag_key;

use crate::Res;

use super::Remote;

/// A mock implementation of the `Remote` trait.
#[derive(Default)]
pub struct MockRemote {
    pub(crate) storage: MockStorage,
    /// Per-URI count of `get_object_stream` calls, so tests can assert that a
    /// config or schema document is fetched exactly once across an operation.
    get_object_calls: Arc<Mutex<HashMap<String, usize>>>,
    /// Fetches held until a test lets them go; see [`MockRemote::park`].
    parked: Arc<Mutex<HashMap<String, Arc<Gate>>>>,
    /// Uploads so far, so each one gets its own S3 version, as on a
    /// versioned bucket.
    uploads: std::sync::atomic::AtomicUsize,
}

/// One parked operation: it says when it arrives, then waits to be let go.
///
/// Both sides are [`tokio::sync::Notify`] permits, so neither order of arrival
/// and release loses a wake-up.
#[derive(Default)]
pub struct Gate {
    arrived: tokio::sync::Notify,
    released: tokio::sync::Notify,
}

impl Gate {
    /// Waits until the parked fetch has arrived and is holding.
    pub async fn arrived(&self) {
        self.arrived.notified().await;
    }

    /// Lets the parked fetch go on.
    pub fn release(&self) {
        self.released.notify_one();
    }

    /// The parked side: says it has arrived, then waits to be released.
    pub async fn hold(&self) {
        self.arrived.notify_one();
        self.released.notified().await;
    }
}

impl MockRemote {
    /// How many times `get_object_stream` was called for `uri`.
    ///
    /// # Panics
    ///
    /// Panics if the internal call-count mutex is poisoned.
    #[must_use]
    pub fn get_object_count(&self, uri: &str) -> usize {
        self.get_object_calls
            .lock()
            .unwrap()
            .get(uri)
            .copied()
            .unwrap_or(0)
    }

    /// Holds the next `get_object_stream` or `put_object` of `uri` until the
    /// returned gate is released, so a test can land another operation inside
    /// this one. Only the next call parks; later ones pass.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    #[must_use]
    pub fn park(&self, uri: &str) -> Arc<Gate> {
        let gate = Arc::new(Gate::default());
        self.parked
            .lock()
            .unwrap()
            .insert(uri.to_string(), Arc::clone(&gate));
        gate
    }
}

/// Where the backing storage keeps the object at `uri`.
///
/// The URI itself can't be a path on Windows: `s3:` is not a valid folder
/// name and `?` is not allowed in file names.
fn storage_path(uri: &str) -> String {
    uri.strip_prefix("s3://").unwrap_or(uri).replace('?', "#")
}

/// The file's hash, of the checksum type S3 would answer with, and its bytes.
async fn hash_and_read(
    storage: &(impl Storage + Sync),
    path: &Path,
    algorithm: u64,
) -> Res<(ObjectHash, Vec<u8>)> {
    let (hash, _) = crate::checksum::hash_file(storage, path, algorithm).await?;
    Ok((hash, storage.read_bytes(path).await?))
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "seven of this impl's methods do await; only `host_config` and `verify_bucket` do not. Rewriting just those two would leave one impl split between `async fn` and `fn -> impl Future`, which reads worse than either consistent choice."
)]
impl Remote for MockRemote {
    async fn exists(&self, _host: Option<&Host>, s3_uri: &S3Uri) -> Res<bool> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {key} exists request");
        Ok(self.storage.exists(storage_path(&key)).await)
    }

    async fn get_object_stream(
        &self,
        _host: Option<&Host>,
        s3_uri: &S3Uri,
    ) -> Res<RemoteObjectStream> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {key} get request");
        *self
            .get_object_calls
            .lock()
            .unwrap()
            .entry(key.clone())
            .or_insert(0) += 1;
        let parked = self.parked.lock().unwrap().remove(&key);
        if let Some(gate) = parked {
            gate.hold().await;
        }

        let body = self
            .storage
            .read_byte_stream(storage_path(&key))
            .await
            .map_err(|err| match err {
                Error::Fs(FsError::ByteStream(_)) => {
                    S3Error::new(S3ErrorKind::NotFound(key.clone())).into()
                }
                Error::Io(inner_err) if inner_err.kind() == std::io::ErrorKind::NotFound => {
                    S3Error::new(S3ErrorKind::NotFound(key.clone())).into()
                }
                other => other,
            });
        Ok(RemoteObjectStream {
            body: body?,
            uri: s3_uri.clone(),
        })
    }

    async fn put_object(
        &self,
        _host: Option<&Host>,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
    ) -> Res {
        let key = s3_uri.to_string();
        log::debug!("Mocking {key} put request");
        let parked = self.parked.lock().unwrap().remove(&key);
        if let Some(gate) = parked {
            gate.hold().await;
        }
        self.storage
            .write_byte_stream(storage_path(&key), contents.into())
            .await
    }

    async fn resolve_url(&self, _host: Option<&Host>, s3_uri: &S3Uri) -> Res<S3Uri> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {key} HEAD request");
        if self.storage.exists(storage_path(&key)).await {
            Ok(s3_uri.clone())
        } else {
            Err(Error::S3(S3Error::new(S3ErrorKind::NotFound(key))))
        }
    }

    async fn upload_file(
        &self,
        host_config: &HostConfig,
        source_path: impl AsRef<Path>,
        dest_uri: &S3Uri,
        _size: u64,
    ) -> Res<(S3Uri, ObjectHash)> {
        // A domain on the real file system hands over a path this mock's
        // storage would re-root, so read such a file where it is.
        let source = source_path.as_ref();
        let algorithm = host_config.checksums.algorithm_code();
        let (hash, body) = if tokio::fs::try_exists(source).await? {
            hash_and_read(&LocalStorage::new(), source, algorithm).await?
        } else {
            hash_and_read(&self.storage, source, algorithm).await?
        };
        let uploaded = S3Uri {
            version: Some(format!(
                "version-{}",
                self.uploads
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed)
                    .saturating_add(1)
            )),
            ..dest_uri.clone()
        };
        // Kept, so another domain on this remote can download what was pushed.
        self.storage
            .write_byte_stream(storage_path(&uploaded.to_string()), body.into())
            .await?;
        Ok((uploaded, hash))
    }

    async fn host_config(&self, _host: Option<&Host>) -> Res<HostConfig> {
        Ok(HostConfig::default())
    }

    async fn verify_bucket(&self, _bucket: &str) -> Res {
        Ok(())
    }

    /// What the registry, or the bucket, would list: the hash in each timestamped pointer
    /// `put_object` stored under `.quilt/named_packages/<namespace>/`, so a
    /// test seeds publication the way a push makes it. `latest` is a moving
    /// alias, not a revision.
    async fn published_revisions(
        &self,
        _host: Option<&Host>,
        bucket: &str,
        namespace: &Namespace,
    ) -> Res<Vec<String>> {
        let dir = format!("s3://{bucket}/{}", tag_key(namespace, ""));
        log::debug!("Mocking {dir} revision listing");
        if !self.storage.exists(storage_path(&dir)).await {
            return Ok(Vec::new());
        }
        let mut entries = self.storage.read_dir(storage_path(&dir)).await?;
        let mut hashes = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            if entry.file_name() == "latest" {
                continue;
            }
            let pointer = tokio::fs::read_to_string(entry.path()).await?;
            hashes.push(pointer.trim().to_string());
        }
        Ok(hashes)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    #[test(tokio::test)]
    async fn test_get_object_stream() -> Res {
        let remote = MockRemote::default();
        remote
            .put_object(
                None,
                &S3Uri::try_from("s3://found/n?versionId=v")?,
                b"Hello".to_vec(),
            )
            .await?;
        let s3_uri_not_found = S3Uri::try_from("s3://b/n?versionId=v")?;
        let Err(err) = remote.get_object_stream(None, &s3_uri_not_found).await else {
            panic!("expected S3NotFound error");
        };
        assert!(err.is_not_found(), "expected S3NotFound, got: {err}");
        let s3_uri_found = S3Uri::try_from("s3://found/n?versionId=v")?;
        let found = remote.get_object_stream(None, &s3_uri_found).await?;
        assert_eq!(found.body.collect().await?.to_vec(), b"Hello");
        Ok(())
    }
}
