use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::sync::Arc;
use std::sync::Mutex;

use aws_sdk_s3::primitives::ByteStream;
use tracing::log;

use crate::Error;
use crate::error::FsError;
use crate::error::S3Error;
use crate::error::S3ErrorKind;
use crate::flow::list_dirs;
use crate::io::remote::ConditionalPut;
use crate::io::remote::HostConfig;
use crate::io::remote::RemoteObjectStream;
use crate::io::remote::WriteCondition;
use crate::io::storage::Storage;
use crate::io::storage::mocks::MockStorage;
use crate::object_hash::ObjectHash;
use quilt_uri::Host;
use quilt_uri::Namespace;
use quilt_uri::S3Uri;
use quilt_uri::is_top_hash;
use quilt_uri::paths::TAGS_PREFIX;
use quilt_uri::paths::get_manifest_key;
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
    /// Objects this remote's role may write but not read; see
    /// [`MockRemote::deny_reads`].
    unreadable: Arc<Mutex<HashSet<String>>>,
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
    /// A remote whose objects are kept in `root`, which every `MockRemote` at
    /// the same `root` shares, in this process or another.
    #[cfg(test)]
    pub(crate) fn at(root: std::path::PathBuf) -> Self {
        Self {
            storage: MockStorage {
                temp_dir: crate::io::storage::mocks::Root::Shared(root),
            },
            get_object_calls: Arc::default(),
            parked: Arc::default(),
            unreadable: Arc::default(),
        }
    }

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

    /// Holds the next `get_object_stream`, `put_object` or `put_object_if` of
    /// `uri` until the returned gate is released, so a test can land another
    /// operation inside this one. Only the next call parks; later ones pass.
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

    /// Refuses reads of `uri` as S3 refuses a role without read access to
    /// it: `exists`, `get_object_stream` and the conditional `put_object_if`,
    /// which S3 evaluates against the object, are denied, and a plain
    /// `put_object` still writes.
    ///
    /// # Panics
    ///
    /// Panics if the internal mutex is poisoned.
    pub fn deny_reads(&self, uri: &str) {
        self.unreadable.lock().unwrap().insert(uri.to_string());
    }

    fn denied(&self, key: &str) -> Option<Error> {
        self.unreadable
            .lock()
            .unwrap()
            .contains(key)
            .then(|| S3Error::new(S3ErrorKind::AccessDenied(key.to_string())).into())
    }

    /// The object's contents, or `None` when there is none, whatever
    /// [`MockRemote::deny_reads`] refuses.
    ///
    /// # Errors
    ///
    /// Returns an error if the backing storage cannot be read.
    pub async fn current(&self, key: &str) -> Res<Option<Vec<u8>>> {
        let path = storage_path(key);
        if !self.storage.exists(&path).await {
            return Ok(None);
        }
        let stream = self.storage.read_byte_stream(&path).await?;
        Ok(Some(stream.collect().await?.to_vec()))
    }
}

/// The mock's `ETag`: derived from the contents, as S3's is for an object
/// written in one part, so a rewrite with other contents changes it.
fn etag_of(contents: &[u8]) -> String {
    use sha2::Digest;
    format!("\"{}\"", hex::encode(sha2::Sha256::digest(contents)))
}

/// Where the backing storage keeps the object at `uri`.
///
/// The URI itself can't be a path on Windows: `s3:` is not a valid folder
/// name and `?` is not allowed in file names.
fn storage_path(uri: &str) -> String {
    uri.strip_prefix("s3://").unwrap_or(uri).replace('?', "#")
}

#[allow(
    clippy::unused_async_trait_impl,
    reason = "seven of this impl's methods do await; only `host_config` and `verify_bucket` do not. Rewriting just those two would leave one impl split between `async fn` and `fn -> impl Future`, which reads worse than either consistent choice."
)]
impl Remote for MockRemote {
    async fn exists(&self, _host: Option<&Host>, s3_uri: &S3Uri) -> Res<bool> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {key} exists request");
        if let Some(err) = self.denied(&key) {
            return Err(err);
        }
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
        if let Some(denied) = self.denied(&key) {
            return Err(denied);
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
        let contents = body?.collect().await?.to_vec();
        Ok(RemoteObjectStream {
            etag: Some(etag_of(&contents)),
            body: ByteStream::from(contents),
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

    async fn put_object_if(
        &self,
        _host: Option<&Host>,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
        condition: &WriteCondition,
    ) -> Res<ConditionalPut> {
        let key = s3_uri.to_string();
        log::debug!("Mocking {key} conditional put request");
        // Parked before the condition is evaluated, so a test can move the
        // object between the read the condition came from and this write.
        let parked = self.parked.lock().unwrap().remove(&key);
        if let Some(gate) = parked {
            gate.hold().await;
        }
        if let Some(denied) = self.denied(&key) {
            return Err(denied);
        }
        let current = self.current(&key).await?;
        let holds = match (condition, &current) {
            (WriteCondition::Absent, None) => true,
            (WriteCondition::Matches(etag), Some(contents)) => *etag == etag_of(contents),
            _ => false,
        };
        if !holds {
            return Ok(ConditionalPut::Moved);
        }
        self.storage
            .write_byte_stream(storage_path(&key), contents.into())
            .await?;
        Ok(ConditionalPut::Written)
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
        // S3 answers with the checksum type the upload asked for.
        let (hash, _) = crate::checksum::hash_file(
            &self.storage,
            source_path.as_ref(),
            host_config.checksums.algorithm_code(),
        )
        .await?;
        Ok((
            S3Uri {
                version: Some("version".to_string()),
                ..dest_uri.clone()
            },
            hash,
        ))
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

    /// The names under `<bucket>/.quilt/packages/` that start with `prefix`
    /// and are a whole top hash, as `put_object` stored them.
    async fn manifest_hashes_with_prefix(
        &self,
        _host: Option<&Host>,
        bucket: &str,
        prefix: &str,
    ) -> Res<Vec<String>> {
        let dir = format!("s3://{bucket}/{}", get_manifest_key(""));
        log::debug!("Mocking {dir}{prefix}* manifest listing");
        if !self.storage.exists(storage_path(&dir)).await {
            return Ok(Vec::new());
        }
        let mut entries = self.storage.read_dir(storage_path(&dir)).await?;
        let mut hashes = Vec::new();
        while let Some(entry) = entries.next_entry().await? {
            let name = entry.file_name().to_string_lossy().into_owned();
            // A key nested under a hash-named name makes that name a directory,
            // which S3 would not list as an object of its own.
            if is_top_hash(&name) && name.starts_with(prefix) && entry.file_type().await?.is_file()
            {
                hashes.push(name);
            }
        }
        Ok(hashes)
    }

    /// Each `<bucket>/.quilt/named_packages/<owner>/<name>/latest` that
    /// `put_object` stored, with the top hash it holds. One that
    /// [`MockRemote::deny_reads`] refuses fails the listing.
    async fn latest_hashes(
        &self,
        _host: Option<&Host>,
        bucket: &str,
    ) -> Res<Vec<(Namespace, String)>> {
        let root = format!("s3://{bucket}/{TAGS_PREFIX}");
        log::debug!("Mocking {root} latest listing");
        let mut latest = Vec::new();
        for owner in list_dirs(&self.storage, Path::new(&storage_path(&root))).await? {
            for package in list_dirs(&self.storage, &owner).await? {
                let name = |dir: &Path| {
                    dir.file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned()
                };
                let Ok(namespace) =
                    Namespace::try_from(format!("{}/{}", name(&owner), name(&package)))
                else {
                    continue;
                };
                let pointer = format!("s3://{bucket}/{}", tag_key(&namespace, "latest"));
                if let Some(err) = self.denied(&pointer) {
                    return Err(err);
                }
                let Some(bytes) = self.current(&pointer).await? else {
                    continue;
                };
                let hash = String::from_utf8_lossy(&bytes).into_owned();
                if is_top_hash(&hash) {
                    latest.push((namespace, hash));
                }
            }
        }
        Ok(latest)
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
