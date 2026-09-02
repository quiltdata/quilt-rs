use std::collections::HashMap;
use std::collections::hash_map::Entry;
use std::path::Path;
use std::sync::Arc;
use std::sync::RwLock;

use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_credential_types::provider::ProvideCredentials;
use aws_credential_types::provider::error::CredentialsError;
use aws_credential_types::provider::future;
use aws_sdk_s3::error::ProvideErrorMetadata;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::primitives::ByteStream;
use aws_types::region::Region;
use tokio::sync::Mutex as AsyncMutex;
use tracing::debug;
use tracing::info;
use tracing::trace;
use tracing::warn;

use crate::Error;
use crate::Res;
use crate::auth;
use crate::auth::OAuthParams;
use crate::auth::RoleInfo;
use crate::error::LoginError;
use crate::error::RemoteCatalogError;
use crate::error::S3Error;
use crate::error::S3ErrorKind;
use crate::io::remote::HostChecksums;
use crate::io::remote::HostConfig;
use crate::io::remote::HttpClient;
use crate::io::remote::Remote;
use crate::io::remote::describe_sdk_error;
use crate::io::remote::host::fetch_host_config;
use crate::io::remote::object::multipart_upload_and_sha256_chunksum;
use crate::io::remote::object::put_and_request_checksum;
use crate::io::storage::LocalStorage;
use crate::io::storage::auth::OAuthClient;
use crate::object_hash::ObjectHash;
use crate::paths::DomainPaths;
use quilt_uri::Host;
use quilt_uri::S3Uri;

use crate::io::remote::RemoteObjectStream;

/// S3's HEAD-bucket endpoint returns `x-amz-bucket-region` for any bucket
/// that exists and is addressable, regardless of permissions. A missing
/// header means the bucket name didn't resolve — typo, malformed name,
/// or the bucket genuinely doesn't exist. We surface a single domain
/// error instead of the raw HTTP artifact, since from the caller's
/// perspective the distinction doesn't matter: the bucket is unusable.
async fn find_bucket_region(client: &impl HttpClient, bucket: &str) -> Res<String> {
    let headers = client
        .head(&format!("https://s3.amazonaws.com/{bucket}"))
        .await?;
    let region = headers
        .get("x-amz-bucket-region")
        .ok_or_else(|| RemoteCatalogError::BucketUnreachable(bucket.to_string()))?;
    Ok(region.to_str()?.into())
}

/// Map an AWS error onto a typed kind. Keyed on the *code* wherever there is
/// one, never on the HTTP status alone: `ExpiredToken` and
/// `InvalidAccessKeyId` are also 403 but call for a credential re-vend, not a
/// role switch.
///
/// The one exception is a 403 that names no code at all. `HeadObject` is why
/// that case exists: a HEAD response carries no body, so S3 has nowhere to
/// put the `<Code>AccessDenied</Code>` document every other operation
/// returns, and the SDK has no code to hand us. Reading the status is safe
/// *because* the code is absent — a 403 that did name itself never reaches
/// that arm.
///
/// Only a genuine denial changes the kind. Everything else is handed to
/// `fallback`, so a call site keeps the operation-specific kind it would
/// have produced anyway — a failed put stays [`S3ErrorKind::PutObject`]
/// rather than collapsing into an undiagnosable [`S3ErrorKind::Raw`].
pub(super) fn classify_s3_error(
    code: Option<&str>,
    status: Option<u16>,
    described: &str,
    fallback: fn(String) -> S3ErrorKind,
) -> S3ErrorKind {
    match code {
        Some("AccessDenied") => S3ErrorKind::AccessDenied(described.to_string()),
        None if status == Some(403) => S3ErrorKind::AccessDenied(described.to_string()),
        Some("InvalidAccessKeyId" | "ExpiredToken" | "InvalidToken" | "InvalidClientTokenId") => {
            S3ErrorKind::InvalidCredentials(described.to_string())
        }
        // Only codes meaning a missing *object* belong here. `push` reads
        // `is_not_found` on a package's `latest` tag as "no revisions yet, so
        // this is a first push" (flow/push.rs), so a code that means a missing
        // bucket would send it to write into one that does not exist.
        Some("NoSuchKey" | "NotFound") => S3ErrorKind::NotFound(described.to_string()),
        _ => fallback(described.to_string()),
    }
}

/// Classify an `SdkError` straight off an AWS call.
///
/// [`describe_sdk_error`] consumes the error, so the code and status have to
/// be lifted out first; doing it here keeps every call site from repeating
/// that ordering constraint (and from getting it wrong).
pub(super) fn classify_sdk_error<E>(
    err: SdkError<E>,
    fallback: fn(String) -> S3ErrorKind,
) -> S3ErrorKind
where
    E: ProvideErrorMetadata + std::error::Error + Send + Sync + 'static,
{
    let code = err
        .as_service_error()
        .and_then(ProvideErrorMetadata::code)
        .map(str::to_owned);
    let status = err.raw_response().map(|raw| raw.status().as_u16());
    classify_s3_error(code.as_deref(), status, &describe_sdk_error(err), fallback)
}

async fn get_object_stream(
    client: &aws_sdk_s3::Client,
    s3_uri: &S3Uri,
    host: Option<&Host>,
) -> Res<RemoteObjectStream> {
    let result = client.get_object().bucket(&s3_uri.bucket).key(&s3_uri.key);
    let result = match &s3_uri.version {
        Some(version) => result.version_id(version),
        None => result,
    };

    let result = result.send().await.map_err(|err| match &err {
        SdkError::ServiceError(svc) if svc.err().is_no_such_key() => Error::S3(S3Error {
            host: host.cloned(),
            kind: S3ErrorKind::NotFound(s3_uri.to_string()),
        }),
        // The host separates a deployment session, whose remedy is signing in,
        // from ambient `~/.aws` credentials, whose remedy is the file. A `None`
        // here is read downstream as the latter.
        _ => Error::S3(S3Error {
            host: host.cloned(),
            kind: classify_sdk_error(err, S3ErrorKind::Raw),
        }),
    })?;
    let uri_versioned = S3Uri {
        version: result.version_id,
        ..s3_uri.clone()
    };
    Ok(RemoteObjectStream {
        body: result.body,
        uri: uri_versioned,
    })
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct CredsRef {
    region: Region,
    host: Option<Host>,
}

/// Adapter that lets the AWS SDK pull fresh credentials from our `Auth`
/// layer on every request, instead of holding a static
/// `aws_credential_types::Credentials` that ages out.
///
/// The SDK wraps this in its own caching layer with TTL and async
/// prefetch, so we just need to return the *current* credentials on
/// each call — `get_credentials_or_refresh` already handles the
/// "token expired → refresh → new STS creds" flow.
///
/// Generic over `HttpClient` so tests can inject a mock; production
/// code instantiates it with [`ReqwestClient`].
#[derive(Clone, Debug)]
struct QuiltCredentialsProvider<H> {
    auth: auth::Auth,
    http: H,
    host: Host,
}

impl<H> ProvideCredentials for QuiltCredentialsProvider<H>
where
    H: HttpClient + Clone + std::fmt::Debug + Send + Sync + 'static,
{
    fn provide_credentials<'a>(&'a self) -> future::ProvideCredentials<'a>
    where
        Self: 'a,
    {
        future::ProvideCredentials::new(async move {
            let c = self
                .auth
                .get_credentials_or_refresh(&self.http, &self.host)
                .await
                .map_err(CredentialsError::provider_error)?;
            Ok(Credentials::new(
                c.access_key,
                c.secret_key,
                Some(c.token),
                Some(c.expires_at.into()),
                "quilt-registry",
            ))
        })
    }
}

/// Implementation of the `Remote` trait for S3
#[derive(Debug)]
pub struct RemoteS3 {
    auth: auth::Auth,
    http: crate::io::remote::client::ReqwestClient,
    s3: Arc<RwLock<HashMap<CredsRef, aws_sdk_s3::Client>>>,
    client_locks: Arc<RwLock<HashMap<CredsRef, Arc<AsyncMutex<()>>>>>,
    regions: RwLock<HashMap<String, Region>>,
}

impl RemoteS3 {
    #[must_use]
    pub fn new(paths: DomainPaths, storage: LocalStorage) -> Self {
        RemoteS3 {
            http: crate::io::remote::client::ReqwestClient::new(),
            s3: Arc::new(RwLock::new(HashMap::new())),
            client_locks: Arc::new(RwLock::new(HashMap::new())),
            regions: RwLock::new(HashMap::new()),
            auth: auth::Auth::new(paths, Arc::new(storage)),
        }
    }

    pub fn try_clone(&self) -> Res<Self> {
        let regions = match self.regions.read() {
            Ok(regions) => regions.clone(),
            Err(_) => return Err(Error::S3(S3Error::new(S3ErrorKind::RemoteInit))),
        };
        Ok(RemoteS3 {
            http: self.http.clone(),
            s3: Arc::clone(&self.s3),
            client_locks: Arc::clone(&self.client_locks),
            regions: RwLock::new(regions),
            auth: self.auth.clone(),
        })
    }

    pub async fn login(&self, host: &Host, refresh_token: String) -> Res {
        self.auth.login(&self.http, host, refresh_token).await
    }

    pub async fn login_oauth(&self, host: &Host, params: OAuthParams) -> Res {
        self.auth.login_oauth(&self.http, host, params).await
    }

    pub async fn get_or_register_client(
        &self,
        host: &Host,
        redirect_uri: &str,
    ) -> Res<OAuthClient> {
        self.auth
            .get_or_register_client(&self.http, host, redirect_uri)
            .await
    }

    /// Read the active role and reconcile the credential cache.
    ///
    /// See [`Auth::refresh_roles`] for the flush contract — in particular
    /// that expiring credentials does not invalidate this remote's cached
    /// S3 clients. Callers that hold a client cache must clear it too.
    ///
    /// [`Auth::refresh_roles`]: crate::auth::Auth::refresh_roles
    pub async fn refresh_roles(&self, host: &Host) -> Res<RoleInfo> {
        self.auth.refresh_roles(&self.http, host).await
    }

    /// Make `role_name` the user's primary role.
    ///
    /// The change is server-side and global. Locally this expires the
    /// host's cached credentials, but **not** this remote's cached S3
    /// clients — call [`RemoteS3::clear_client_cache`] for `host`
    /// afterwards or the old role keeps signing until its own expiry.
    pub async fn switch_role(&self, host: &Host, role_name: &str) -> Res<RoleInfo> {
        self.auth.switch_role(&self.http, host, role_name).await
    }

    /// The buckets the active role can read. An optimistic hint: it
    /// over-reports for unmanaged roles and anonymous-access stacks, and
    /// never distinguishes read from write.
    pub async fn readable_buckets(&self, host: &Host) -> Res<Vec<String>> {
        self.auth.readable_buckets(&self.http, host).await
    }

    /// Expire the host's cached STS credentials, keeping the login token.
    ///
    /// **Not sufficient on its own.** An already-built `aws_sdk_s3::Client`
    /// holds its own resolved credentials in the SDK's lazy identity cache,
    /// so the old role keeps signing until they expire (~1h) — call
    /// [`RemoteS3::clear_client_cache`] for the same host afterwards.
    pub async fn expire_credentials(&self, host: &Host) -> Res {
        self.auth.expire_credentials(host).await
    }

    async fn get_region_for_bucket(&self, bucket: &str) -> Res<Region> {
        {
            if let Some(region) = self
                .regions
                .read()
                .map_err(|e| S3Error::new(S3ErrorKind::PoisonLock(e.to_string())))?
                .get(bucket)
            {
                return Ok(region.clone());
            }
        }

        let region = find_bucket_region(&self.http, bucket).await?;

        let mut map = self
            .regions
            .write()
            .map_err(|e| S3Error::new(S3ErrorKind::PoisonLock(e.to_string())))?;
        match map.entry(bucket.to_owned()) {
            Entry::Occupied(entry) => Ok(entry.get().clone()),
            Entry::Vacant(entry) => Ok(entry.insert(Region::new(region)).clone()),
        }
    }

    /// `aws_config::defaults` already applies 3-attempt standard retry
    /// (exponential backoff + jitter) and a 3.1 s connect timeout; no
    /// read/operation timeout so slow multipart uploads aren't cut off.
    ///
    /// For the `Some(host)` branch, credential freshness is handled by
    /// [`QuiltCredentialsProvider`] on every S3 request — the cached
    /// client itself holds the provider, not a frozen access key, so
    /// it stays usable across STS rotations.
    async fn get_client_for_region(
        &self,
        host: Option<&Host>,
        region: aws_types::region::Region,
    ) -> Res<aws_sdk_s3::Client> {
        let creds_ref = CredsRef {
            region: region.clone(),
            host: host.cloned(),
        };

        let cached_client = {
            let map = self
                .s3
                .read()
                .map_err(|e| S3Error::new(S3ErrorKind::PoisonLock(e.to_string())))?;
            map.get(&creds_ref).cloned()
        };
        if let Some(client) = cached_client {
            info!("✔️ Using cached S3 client for region {:?}", region);
            return Ok(client);
        }

        // Cache misses for the same principal can arrive together when a package
        // opens. Share one construction lock across every cloned RemoteS3 handle,
        // then recheck the shared cache after waiting.
        let client_lock = {
            let mut locks = self
                .client_locks
                .write()
                .map_err(|e| S3Error::new(S3ErrorKind::PoisonLock(e.to_string())))?;
            locks
                .entry(creds_ref.clone())
                .or_insert_with(|| Arc::new(AsyncMutex::new(())))
                .clone()
        };
        let _client_guard = client_lock.lock().await;
        if let Some(client) = self
            .s3
            .read()
            .map_err(|e| S3Error::new(S3ErrorKind::PoisonLock(e.to_string())))?
            .get(&creds_ref)
            .cloned()
        {
            info!("✔️ Using cached S3 client for region {:?}", region);
            return Ok(client);
        }

        // `debug`, and worth reading as a signal rather than noise: a *new* client
        // per fetch means the client cache is not being hit, which is TLS and config
        // work repeated for nothing. The line stays so that stays visible.
        debug!("⏳ Creating new S3 client for region {:?}", region);
        let config = match host {
            None => {
                info!("⏳ No `&catalog=`, so we use credentials in ~/.aws");
                let config = aws_config::defaults(BehaviorVersion::latest())
                    .region(region.clone())
                    .load()
                    .await;

                // Check if we have valid credentials
                if config.credentials_provider().is_none() {
                    return Err(Error::Login(LoginError::Required(None)));
                }
                config
            }
            Some(host) => {
                // Smoke-test eagerly so `Login required` surfaces now rather
                // than inside a later S3 call. The provider below handles
                // subsequent refreshes per-request.
                self.auth
                    .get_credentials_or_refresh(&self.http, host)
                    .await?;
                trace!("✔️ Got credentials for host {:?}", host);
                aws_config::defaults(BehaviorVersion::latest())
                    .region(region.clone())
                    .credentials_provider(QuiltCredentialsProvider {
                        auth: self.auth.clone(),
                        http: self.http.clone(),
                        host: host.clone(),
                    })
                    .load()
                    .await
            }
        };
        let client = aws_sdk_s3::Client::new(&config);
        // The construction is already announced above; this only says it finished.
        trace!("✔️ created new S3 client for region {:?}", region);

        // Cache the new client
        let mut map = self
            .s3
            .write()
            .map_err(|e| S3Error::new(S3ErrorKind::PoisonLock(e.to_string())))?;

        match map.entry(creds_ref) {
            Entry::Occupied(mut entry) => {
                // Replace existing client with new one
                entry.insert(client.clone());
                Ok(client)
            }
            Entry::Vacant(entry) => Ok(entry.insert(client).clone()),
        }
    }

    /// Drop cached S3 clients so the next request rebuilds them with a
    /// fresh credential provider. Called on logout: a cached client still
    /// holds the STS credentials minted before logout (valid ~1h), so
    /// without this the running app keeps serving reads/writes after the
    /// on-disk auth token is erased.
    ///
    /// `Some(host)` clears only the clients for that catalog host;
    /// `None` (global logout) clears every cached client.
    ///
    /// The `regions` cache is left intact: it is keyed by bucket name and
    /// holds only public HEAD-bucket region lookups — no credentials — so
    /// it never goes stale on logout.
    pub fn clear_client_cache(&self, host: Option<&Host>) {
        // On a poisoned lock, recover the guard and clear anyway: dropping
        // stale clients on logout is the safe outcome, never leaving
        // credentials cached.
        let mut map = match self.s3.write() {
            Ok(map) => map,
            Err(poisoned) => poisoned.into_inner(),
        };
        match host {
            Some(host) => map.retain(|creds_ref, _| creds_ref.host.as_ref() != Some(host)),
            None => map.clear(),
        }
        drop(map);

        let mut locks = match self.client_locks.write() {
            Ok(locks) => locks,
            Err(poisoned) => poisoned.into_inner(),
        };
        match host {
            Some(host) => locks.retain(|creds_ref, _| creds_ref.host.as_ref() != Some(host)),
            None => locks.clear(),
        }
    }

    async fn get_client_for_bucket(
        &self,
        host: Option<&Host>,
        bucket: &str,
    ) -> Res<aws_sdk_s3::Client> {
        let region = self.get_region_for_bucket(bucket).await?.clone();
        self.get_client_for_region(host, region)
            .await
            .map_err(|e| match e {
                Error::Login(LoginError::Required(_)) | Error::S3(_) => e,
                _ => Error::S3(S3Error {
                    host: host.cloned(),
                    kind: S3ErrorKind::Client(e.to_string()),
                }),
            })
    }
}

impl Remote for RemoteS3 {
    async fn exists(&self, host: Option<&Host>, s3_uri: &S3Uri) -> Res<bool> {
        debug!(
            "⏳ Checking if object exists - host: {:?}, uri: {}",
            host, s3_uri
        );
        let client = self.get_client_for_bucket(host, &s3_uri.bucket).await?;
        let result = client.head_object().bucket(&s3_uri.bucket).key(&s3_uri.key);
        let result = match &s3_uri.version {
            Some(version) => result.version_id(version),
            None => result,
        };
        match result.send().await {
            Ok(_) => {
                info!("✔️ Object exists at {}", s3_uri);
                Ok(true)
            }
            Err(SdkError::ServiceError(err)) if err.err().is_not_found() => {
                info!("ℹ️ Object does not exist at {}", s3_uri);
                Ok(false)
            }
            // A denial is typed distinctly — this is the push gate's first S3
            // call (`.quilt/workflows/config.yml`), so collapsing it into a
            // generic existence failure is what makes a no-read role's Push
            // read as an unexplained S3 error instead of a role problem.
            // Anything else stays a plain existence failure.
            Err(err) => {
                warn!("❌ Failed to check object existence at {}: {}", s3_uri, err);
                Err(Error::S3(S3Error {
                    host: host.cloned(),
                    kind: classify_sdk_error(err, S3ErrorKind::Exists),
                }))
            }
        }
    }

    async fn get_object_stream(
        &self,
        host: Option<&Host>,
        s3_uri: &S3Uri,
    ) -> Res<RemoteObjectStream> {
        debug!(
            "⏳ Getting object stream - host: {:?}, uri: {}",
            host, s3_uri
        );
        let client = self.get_client_for_bucket(host, &s3_uri.bucket).await?;
        match get_object_stream(&client, s3_uri, host).await {
            Ok(stream) => {
                debug!("✔️ Created stream for object {}", s3_uri);
                Ok(stream)
            }
            Err(e) if e.is_not_found() => {
                info!("ℹ️ Object not found: {}", s3_uri);
                Err(e)
            }
            // Pass the typed denial straight through: re-wrapping it as a
            // generic stream failure would erase the one bit callers branch
            // on — that the active role, not the session, is the problem.
            Err(e) if e.is_access_denied() => {
                warn!("❌ Access denied reading {}: {}", s3_uri, e);
                Err(e)
            }
            // A rejected credential says the session is dead, not that this
            // read failed: autosync raises the login affordance on it rather
            // than backing off. Re-wrapped, it is indistinguishable from a
            // network fault.
            Err(e) if e.is_invalid_credentials() => {
                warn!("❌ Credentials rejected reading {}: {}", s3_uri, e);
                Err(e)
            }
            Err(e) => {
                warn!("❌ Failed to create stream for {}: {}", s3_uri, e);
                Err(Error::S3(S3Error {
                    host: host.cloned(),
                    kind: S3ErrorKind::GetObjectStream(e.to_string()),
                }))
            }
        }
    }

    async fn put_object(
        &self,
        host: Option<&Host>,
        s3_uri: &S3Uri,
        contents: impl Into<ByteStream>,
    ) -> Res {
        self.get_client_for_bucket(host, &s3_uri.bucket)
            .await?
            .put_object()
            .bucket(&s3_uri.bucket)
            .key(&s3_uri.key)
            .body(contents.into())
            .send()
            .await
            // A denial is typed distinctly so the push path can say "this role
            // cannot write here"; anything else stays a plain put failure.
            .map_err(|err| {
                Error::S3(S3Error {
                    host: host.cloned(),
                    kind: classify_sdk_error(err, S3ErrorKind::PutObject),
                })
            })?;

        Ok(())
    }

    async fn resolve_url(&self, host: Option<&Host>, s3_uri: &S3Uri) -> Res<S3Uri> {
        let client = self.get_client_for_bucket(host, &s3_uri.bucket).await?;
        let result = client.head_object().bucket(&s3_uri.bucket).key(&s3_uri.key);
        let result = match &s3_uri.version {
            Some(version) => result.version_id(version),
            None => result,
        };
        match result.send().await {
            Ok(head) => Ok(S3Uri {
                version: head.version_id,
                ..s3_uri.clone()
            }),
            // Same reasoning as [`Remote::exists`]: only a genuine denial
            // changes the kind, everything else stays a resolve failure.
            Err(err) => Err(Error::S3(S3Error {
                host: host.cloned(),
                kind: classify_sdk_error(err, S3ErrorKind::ResolveUrl),
            })),
        }
    }

    // NOTE: For 0-byte Chunked uploads, the checksum is sha256(''), NOT sha256(sha256(''))
    //       So we use the S3 checksum directly without hashing it again
    async fn upload_file(
        &self,
        host_config: &HostConfig,
        source_path: impl AsRef<Path>,
        dest_uri: &S3Uri,
        size: u64,
    ) -> Res<(S3Uri, ObjectHash)> {
        let client = self
            .get_client_for_bucket(host_config.host.as_ref(), &dest_uri.bucket)
            .await?;

        if host_config.checksums == HostChecksums::Sha256Chunked && size != 0 {
            multipart_upload_and_sha256_chunksum(
                client,
                source_path,
                dest_uri,
                size,
                host_config.host.as_ref(),
            )
            .await
        } else {
            put_and_request_checksum(client, source_path, dest_uri, host_config).await
        }
    }

    async fn host_config(&self, host: Option<&Host>) -> Res<HostConfig> {
        fetch_host_config(&self.http, host).await
    }

    async fn verify_bucket(&self, bucket: &str) -> Res {
        self.get_region_for_bucket(bucket).await?;
        Ok(())
    }

    fn clear_client_cache(&self, host: Option<&Host>) {
        RemoteS3::clear_client_cache(self, host);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use std::io::Write;

    use async_trait::async_trait;
    use reqwest::header::HeaderMap;
    use tempfile::NamedTempFile;

    use crate::fixtures::objects::LESS_THAN_8MB_HASH_B64;
    use crate::fixtures::objects::ZERO_HASH_B64;
    use crate::fixtures::objects::less_than_8mb;
    use crate::fixtures::objects::zero_bytes;
    use crate::io::storage::LocalStorage;
    use crate::paths::DomainPaths;

    #[test(tokio::test)]
    async fn live_multipart_upload() -> Res<()> {
        // Create a temporary file with the test content
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(less_than_8mb())?;
        let temp_path = temp_file.path();

        // Set up the S3 client
        let paths = DomainPaths::default();
        let storage = LocalStorage::new();
        let remote = RemoteS3::new(paths, storage);

        // Create host config for SHA256 chunked checksums
        let host_config = HostConfig {
            checksums: HostChecksums::Sha256Chunked,
            host: None,
        };

        // Parse the S3 URI
        let s3_uri =
            S3Uri::try_from("s3://data-yaml-spec-tests/test_quilt_rs/multipart-upload.txt")?;

        // Get the file size
        let size = less_than_8mb().len() as u64;

        // Test the upload
        let result = remote
            .upload_file(&host_config, temp_path, &s3_uri, size)
            .await;

        // Verify the upload succeeded
        assert!(result.is_ok());
        let (uploaded_uri, object_hash) = result?;

        // Verify we got a versioned URI back
        assert!(uploaded_uri.version.is_some());

        // Verify we got a hash back
        assert_eq!(object_hash.to_string(), LESS_THAN_8MB_HASH_B64);

        Ok(())
    }

    #[test(tokio::test)]
    async fn live_zero_bytes_upload() -> Res<()> {
        // Create a temporary file with zero bytes
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(zero_bytes())?;
        let temp_path = temp_file.path();

        // Set up the S3 client
        let paths = DomainPaths::default();
        let storage = LocalStorage::new();
        let remote = RemoteS3::new(paths, storage);

        // Create host config for SHA256 chunked checksums
        let host_config = HostConfig {
            checksums: HostChecksums::Sha256Chunked,
            host: None,
        };

        // Parse the S3 URI
        let s3_uri =
            S3Uri::try_from("s3://data-yaml-spec-tests/test_quilt_rs/zero-bytes-file.txt")?;

        // Get the file size (should be 0)
        let size = zero_bytes().len() as u64;
        assert_eq!(size, 0);

        // Test the upload
        let result = remote
            .upload_file(&host_config, temp_path, &s3_uri, size)
            .await;

        // Verify the upload succeeded
        assert!(result.is_ok());
        let (uploaded_uri, object_hash) = result?;

        // Verify we got a versioned URI back
        assert!(uploaded_uri.version.is_some());

        // Verify we got the correct hash for zero bytes
        assert_eq!(object_hash.to_string(), ZERO_HASH_B64);

        Ok(())
    }

    #[test(tokio::test)]
    async fn live_crc64_upload() -> Res<()> {
        // Read the fixture file content
        let fixture_path = std::path::Path::new("fixtures/user-settings.mkfg");
        let file_content = std::fs::read(fixture_path)?;

        // Create a temporary file with the fixture content
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(&file_content)?;
        let temp_path = temp_file.path();

        // Set up the S3 client
        let paths = DomainPaths::default();
        let storage = LocalStorage::new();
        let remote = RemoteS3::new(paths, storage);

        // Create host config for CRC64 checksums
        let host_config = HostConfig {
            checksums: HostChecksums::Crc64,
            host: None,
        };

        // Parse the S3 URI
        let s3_uri = S3Uri::try_from("s3://data-yaml-spec-tests/test_quilt_rs/crc64.txt")?;

        // Get the file size
        let size = file_content.len() as u64;

        // Test the upload
        let result = remote
            .upload_file(&host_config, temp_path, &s3_uri, size)
            .await;

        // Verify the upload succeeded
        assert!(result.is_ok());
        let (uploaded_uri, object_hash) = result?;

        // Verify we got a versioned URI back
        assert!(uploaded_uri.version.is_some());

        // Verify we got the correct CRC64 hash
        assert_eq!(object_hash.to_string(), "LZmmpqbBItw=");

        Ok(())
    }

    /// `AccessDenied` must be typed distinctly so the roster and autosync can
    /// branch on "this role can't reach the bucket" instead of collapsing it
    /// into a generic error that reads as "sign in again".
    #[test]
    fn access_denied_code_classifies_as_access_denied() {
        let err = classify_s3_error(
            Some("AccessDenied"),
            Some(403),
            "AccessDenied: forbidden",
            S3ErrorKind::Raw,
        );
        assert_eq!(
            err,
            S3ErrorKind::AccessDenied("AccessDenied: forbidden".to_string())
        );
        assert!(S3Error::new(err).is_access_denied());
    }

    /// A denial outranks the caller's fallback: the upload sites ask for
    /// `PutObject`/`UploadFile`, but the one code the push path branches on
    /// still has to come through typed.
    #[test]
    fn upload_denial_classifies_as_access_denied() {
        let err = classify_s3_error(
            Some("AccessDenied"),
            Some(403),
            "AccessDenied: forbidden",
            S3ErrorKind::PutObject,
        );
        assert_eq!(
            err,
            S3ErrorKind::AccessDenied("AccessDenied: forbidden".to_string())
        );
    }

    /// The mirror image: everything that is *not* a denial keeps the kind the
    /// call site would have produced on its own. Flattening these to `Raw`
    /// would cost every upload failure its "which operation broke" detail.
    #[test]
    fn non_denial_codes_keep_the_callers_fallback_kind() {
        assert_eq!(
            classify_s3_error(
                Some("SlowDown"),
                Some(503),
                "SlowDown: throttled",
                S3ErrorKind::PutObject
            ),
            S3ErrorKind::PutObject("SlowDown: throttled".to_string())
        );
        assert_eq!(
            classify_s3_error(
                None,
                Some(500),
                "HTTP 500 (no error body)",
                S3ErrorKind::UploadFile
            ),
            S3ErrorKind::UploadFile("HTTP 500 (no error body)".to_string())
        );
    }

    /// Credential failures are also HTTP 403, which is exactly why we key on
    /// the code: they mean "re-vend credentials", not "wrong role".
    #[test]
    fn credential_codes_classify_as_invalid_credentials() {
        for code in [
            "ExpiredToken",
            "InvalidAccessKeyId",
            "InvalidToken",
            "InvalidClientTokenId",
        ] {
            let err = classify_s3_error(
                Some(code),
                Some(403),
                &format!("{code}: nope"),
                S3ErrorKind::Raw,
            );
            assert!(
                !S3Error::new(err).is_access_denied(),
                "{code} must not be read as a role denial"
            );
            let err = classify_s3_error(
                Some(code),
                Some(403),
                &format!("{code}: nope"),
                S3ErrorKind::Raw,
            );
            assert!(
                S3Error::new(err).is_invalid_credentials(),
                "{code} must be shown as a credential failure"
            );
        }
    }

    /// A bucket that is not there is not "the object is not there".
    ///
    /// `push` reads `is_not_found` on the `latest` tag as "first push for this
    /// package"; classifying `NoSuchBucket` as `NotFound` would make a
    /// misspelled bucket take that branch instead of failing.
    #[test]
    fn missing_bucket_does_not_classify_as_not_found() {
        let err = classify_s3_error(
            Some("NoSuchBucket"),
            Some(404),
            "NoSuchBucket: no-such-bucket",
            S3ErrorKind::Raw,
        );
        assert!(
            !S3Error::new(err).is_not_found(),
            "a missing bucket must not read as a missing object"
        );
    }

    #[test]
    fn missing_objects_classify_as_not_found() {
        let err = classify_s3_error(
            Some("NoSuchKey"),
            Some(404),
            "NoSuchKey: missing",
            S3ErrorKind::GetObject,
        );
        assert_eq!(err, S3ErrorKind::NotFound("NoSuchKey: missing".to_string()));
    }

    #[test]
    fn unknown_codes_stay_raw() {
        let err = classify_s3_error(
            Some("SlowDown"),
            Some(503),
            "SlowDown: throttled",
            S3ErrorKind::Raw,
        );
        assert_eq!(err, S3ErrorKind::Raw("SlowDown: throttled".to_string()));
    }

    #[test]
    fn missing_code_stays_raw() {
        let err = classify_s3_error(
            None,
            Some(500),
            "HTTP 500 (no error body)",
            S3ErrorKind::Raw,
        );
        assert_eq!(
            err,
            S3ErrorKind::Raw("HTTP 500 (no error body)".to_string())
        );
    }

    /// A `HeadObject` refusal is a bodyless 403: HEAD responses carry no
    /// payload, so S3 cannot send the `<Code>AccessDenied</Code>` document.
    /// Without the status arm the `exists` gate on the push path — the first
    /// S3 call a push makes — reports an untyped error and the user is told
    /// to sign in again for a bucket their role simply cannot read.
    #[test]
    fn a_bodyless_403_classifies_as_access_denied() {
        let err = classify_s3_error(
            None,
            Some(403),
            "HTTP 403 (no error body)",
            S3ErrorKind::Exists,
        );
        assert_eq!(
            err,
            S3ErrorKind::AccessDenied("HTTP 403 (no error body)".to_string())
        );
    }

    /// The status arm is narrow on purpose: only 403, and only when nothing
    /// named itself. A bodyless 404 or 500 keeps the call site's own kind.
    #[test]
    fn other_bodyless_statuses_keep_the_callers_fallback_kind() {
        for status in [404u16, 500, 503] {
            let described = format!("HTTP {status} (no error body)");
            assert_eq!(
                classify_s3_error(None, Some(status), &described, S3ErrorKind::ResolveUrl),
                S3ErrorKind::ResolveUrl(described.clone()),
                "HTTP {status} is not a denial"
            );
        }
    }

    /// Answer every connection with the same canned HTTP response, so an
    /// `aws_sdk_s3::Client` pointed at the returned address gets a real S3
    /// error document off the wire instead of a hand-built `SdkError`.
    async fn spawn_canned_s3_endpoint(response: Vec<u8>) -> std::net::SocketAddr {
        use tokio::io::AsyncReadExt;
        use tokio::io::AsyncWriteExt;

        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        tokio::spawn(async move {
            while let Ok((mut stream, _)) = listener.accept().await {
                let response = response.clone();
                tokio::spawn(async move {
                    let mut buf = [0u8; 8192];
                    let _ = stream.read(&mut buf).await;
                    let _ = stream.write_all(&response).await;
                    let _ = stream.shutdown().await;
                });
            }
        });
        addr
    }

    /// The bucket every access-denied test addresses; the stub endpoint
    /// refuses each request regardless of operation, bucket or key.
    const DENIED_BUCKET: &str = "locked";

    /// A [`RemoteS3`] whose region and client caches are pre-seeded with a
    /// client aimed at a stub endpoint that answers every request with S3's
    /// 403 `AccessDenied` document — so calls resolve without a live region
    /// lookup or a credential vend, and every operation comes back denied.
    async fn access_denied_remote() -> RemoteS3 {
        let body = "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\
             <Error><Code>AccessDenied</Code><Message>Access Denied</Message>\
             <RequestId>REQ</RequestId><HostId>HID</HostId></Error>";
        let addr = spawn_canned_s3_endpoint(
            format!(
                "HTTP/1.1 403 Forbidden\r\n\
                 Content-Type: application/xml\r\n\
                 Content-Length: {}\r\n\
                 Connection: close\r\n\r\n{body}",
                body.len()
            )
            .into_bytes(),
        )
        .await;

        let region = Region::new("us-east-1");
        let denied_client = aws_sdk_s3::Client::from_conf(
            aws_sdk_s3::Config::builder()
                .behavior_version(BehaviorVersion::latest())
                .region(region.clone())
                .credentials_provider(Credentials::new("AK", "SK", None, None, "test"))
                .endpoint_url(format!("http://{addr}"))
                .force_path_style(true)
                .build(),
        );

        let remote = RemoteS3::new(DomainPaths::default(), LocalStorage::new());
        remote
            .regions
            .write()
            .unwrap()
            .insert(DENIED_BUCKET.to_string(), region.clone());
        remote
            .s3
            .write()
            .unwrap()
            .insert(CredsRef { region, host: None }, denied_client);
        remote
    }

    /// The typed denial has to survive the `Remote` **method**, not just the
    /// free helper underneath it: the method re-wraps anything it does not
    /// recognise, so without the passthrough arm a locked bucket surfaces as
    /// a generic stream failure and `is_access_denied()` answers `false`.
    #[test(tokio::test)]
    async fn get_object_stream_method_preserves_access_denied() -> Res<()> {
        let remote = access_denied_remote().await;

        let Err(err) =
            Remote::get_object_stream(&remote, None, &S3Uri::try_from("s3://locked/x")?).await
        else {
            panic!("the stub endpoint always refuses");
        };

        assert!(
            err.is_access_denied(),
            "the method must not re-wrap the denial, got: {err}"
        );
        Ok(())
    }

    /// `exists` is the push gate's **first** S3 call: `fetch_workflows_config`
    /// heads `.quilt/workflows/config.yml` before a single byte is uploaded.
    /// A role that cannot read the bucket is refused right there, so if this
    /// method swallows the denial the user is shown a raw S3 error on a row
    /// the roster has already marked as out of reach.
    #[test(tokio::test)]
    async fn exists_method_preserves_access_denied() -> Res<()> {
        let remote = access_denied_remote().await;

        let Err(err) = Remote::exists(&remote, None, &S3Uri::try_from("s3://locked/x")?).await
        else {
            panic!("the stub endpoint always refuses");
        };

        assert!(
            err.is_access_denied(),
            "the existence check must not re-wrap the denial, got: {err}"
        );
        Ok(())
    }

    /// `resolve_url` heads the same way `exists` does, and is reached while
    /// resolving a manifest entry, so it has the same obligation.
    #[test(tokio::test)]
    async fn resolve_url_method_preserves_access_denied() -> Res<()> {
        let remote = access_denied_remote().await;

        let Err(err) = Remote::resolve_url(&remote, None, &S3Uri::try_from("s3://locked/x")?).await
        else {
            panic!("the stub endpoint always refuses");
        };

        assert!(
            err.is_access_denied(),
            "the resolve path must not re-wrap the denial, got: {err}"
        );
        Ok(())
    }

    /// Drives the real `put_object` method against a canned 403, so the
    /// wiring is covered and not just the classifier: a write denial has to
    /// reach the push path typed, or the user is told to sign in again when
    /// the real problem is that their role cannot write here.
    #[test(tokio::test)]
    async fn put_object_method_preserves_access_denied() -> Res<()> {
        let remote = access_denied_remote().await;

        let Err(err) = Remote::put_object(
            &remote,
            None,
            &S3Uri::try_from("s3://locked/x")?,
            ByteStream::from_static(b"payload"),
        )
        .await
        else {
            panic!("the stub endpoint always refuses");
        };

        assert!(
            err.is_access_denied(),
            "the upload path must not re-wrap the denial, got: {err}"
        );
        Ok(())
    }

    /// Same guarantee for the single-shot upload path, which builds its own
    /// error with an `UploadFile` kind rather than going through `put_object`.
    #[test(tokio::test)]
    async fn upload_file_method_preserves_access_denied() -> Res<()> {
        let remote = access_denied_remote().await;
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(b"payload")?;

        let host_config = HostConfig {
            checksums: HostChecksums::Crc64,
            host: None,
        };
        let Err(err) = remote
            .upload_file(
                &host_config,
                temp_file.path(),
                &S3Uri::try_from("s3://locked/x")?,
                7,
            )
            .await
        else {
            panic!("the stub endpoint always refuses");
        };

        assert!(
            err.is_access_denied(),
            "the upload path must not re-wrap the denial, got: {err}"
        );
        Ok(())
    }

    /// And for the multipart path, whose first call — `CreateMultipartUpload`
    /// — is where a write denial shows up.
    #[test(tokio::test)]
    async fn multipart_upload_preserves_access_denied() -> Res<()> {
        let remote = access_denied_remote().await;
        let mut temp_file = NamedTempFile::new()?;
        temp_file.write_all(b"payload")?;

        let host_config = HostConfig {
            checksums: HostChecksums::Sha256Chunked,
            host: None,
        };
        let Err(err) = remote
            .upload_file(
                &host_config,
                temp_file.path(),
                &S3Uri::try_from("s3://locked/x")?,
                7,
            )
            .await
        else {
            panic!("the stub endpoint always refuses");
        };

        assert!(
            err.is_access_denied(),
            "the multipart path must not re-wrap the denial, got: {err}"
        );
        Ok(())
    }

    /// Building a real, offline `aws_sdk_s3::Client` for a region so the
    /// cache-clearing test exercises the actual `s3` map, not a stand-in.
    fn dummy_client(region: &str) -> aws_sdk_s3::Client {
        let conf = aws_sdk_s3::Config::builder()
            .behavior_version(BehaviorVersion::latest())
            .region(Region::new(region.to_string()))
            .build();
        aws_sdk_s3::Client::from_conf(conf)
    }

    /// `clear_client_cache(Some(host))` drops only that host's clients and
    /// keeps the rest; `clear_client_cache(None)` empties the whole cache.
    #[test]
    fn test_clear_client_cache_filters_by_host() {
        use std::str::FromStr;

        let host_a = Host::from_str("a.example.com").unwrap();
        let host_b = Host::from_str("b.example.com").unwrap();

        let remote = RemoteS3::new(DomainPaths::default(), LocalStorage::new());

        {
            let mut map = remote.s3.write().unwrap();
            map.insert(
                CredsRef {
                    region: Region::new("us-east-1"),
                    host: Some(host_a.clone()),
                },
                dummy_client("us-east-1"),
            );
            map.insert(
                CredsRef {
                    region: Region::new("us-west-2"),
                    host: Some(host_b.clone()),
                },
                dummy_client("us-west-2"),
            );
            map.insert(
                CredsRef {
                    region: Region::new("eu-west-1"),
                    host: None,
                },
                dummy_client("eu-west-1"),
            );
        }

        // Clearing host_a leaves host_b and the host-less entry.
        remote.clear_client_cache(Some(&host_a));
        {
            let map = remote.s3.read().unwrap();
            assert_eq!(map.len(), 2);
            assert!(!map.keys().any(|k| k.host.as_ref() == Some(&host_a)));
            assert!(map.keys().any(|k| k.host.as_ref() == Some(&host_b)));
        }

        // Clearing None empties everything.
        remote.clear_client_cache(None);
        assert!(remote.s3.read().unwrap().is_empty());
    }

    #[test]
    fn cloned_remotes_share_client_cache_and_single_flight_locks() {
        let remote = RemoteS3::new(DomainPaths::default(), LocalStorage::new());
        let cloned = remote.try_clone().unwrap();

        assert!(Arc::ptr_eq(&remote.s3, &cloned.s3));
        assert!(Arc::ptr_eq(&remote.client_locks, &cloned.client_locks));

        let key = CredsRef {
            region: Region::new("us-east-1"),
            host: None,
        };
        remote
            .s3
            .write()
            .unwrap()
            .insert(key.clone(), dummy_client("us-east-1"));
        remote
            .client_locks
            .write()
            .unwrap()
            .insert(key.clone(), Arc::new(AsyncMutex::new(())));

        assert!(cloned.s3.read().unwrap().contains_key(&key));
        assert!(cloned.client_locks.read().unwrap().contains_key(&key));

        cloned.clear_client_cache(None);
        assert!(remote.s3.read().unwrap().is_empty());
        assert!(remote.client_locks.read().unwrap().is_empty());
    }

    /// Never called: compiling these calls is the proof that the three
    /// networked role delegations exist on `RemoteS3` with these shapes.
    /// They cannot be driven here — each one talks to a registry.
    async fn role_api_shapes(remote: &RemoteS3, host: &Host) -> Res<()> {
        let _: RoleInfo = remote.refresh_roles(host).await?;
        let _: RoleInfo = remote.switch_role(host, "ReadOnly").await?;
        let _: Vec<String> = remote.readable_buckets(host).await?;
        Ok(())
    }

    /// The delegations must pass the remote's own http client through to
    /// `Auth`, the same way `login` does — otherwise the role calls cannot
    /// reach the registry at all. `expire_credentials` needs no network, so
    /// it is the one we can actually drive.
    #[test(tokio::test)]
    async fn remote_exposes_the_role_api() -> Res<()> {
        use std::str::FromStr;

        use tempfile::TempDir;

        let _ = role_api_shapes;

        let temp = TempDir::new()?;
        let remote = RemoteS3::new(
            DomainPaths::new(temp.path().to_path_buf()),
            LocalStorage::new(),
        );
        let host = Host::from_str("catalog.example.com").unwrap();

        remote.expire_credentials(&host).await?;
        Ok(())
    }

    /// When storage holds valid credentials, the provider must surface them
    /// as `aws_credential_types::Credentials` on every call. This proves
    /// the async plumbing compiles and runs, and that the quilt-side
    /// credential fields map correctly to the SDK ones.
    #[test(tokio::test)]
    async fn test_quilt_credentials_provider_returns_stored_creds() -> Res<()> {
        use std::str::FromStr;

        use tempfile::TempDir;

        use crate::io::storage::auth::AuthIo;
        use crate::io::storage::auth::Credentials as QuiltCreds;

        let temp = TempDir::new()?;
        let paths = DomainPaths::new(temp.path().to_path_buf());
        let storage = Arc::new(LocalStorage::new());
        let host = Host::from_str("catalog.example.com").unwrap();

        let stored = QuiltCreds {
            access_key: "AKIAEXAMPLE".to_string(),
            secret_key: "secret".to_string(),
            token: "session-token".to_string(),
            expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
        };
        let auth_io = AuthIo::new(Arc::clone(&storage), paths.auth_host(&host));
        auth_io.write_credentials(&stored).await?;

        let provider = QuiltCredentialsProvider {
            auth: auth::Auth::new(paths, storage),
            http: crate::io::remote::client::ReqwestClient::new(),
            host,
        };

        let sdk_creds = provider.provide_credentials().await.unwrap();
        assert_eq!(sdk_creds.access_key_id(), stored.access_key);
        assert_eq!(sdk_creds.secret_access_key(), stored.secret_key);
        assert_eq!(sdk_creds.session_token(), Some(stored.token.as_str()));
        Ok(())
    }

    /// Serves a stack config plus freshly minted STS credentials, so the
    /// credentials-refresh path can be exercised without a live registry.
    #[derive(Clone, Debug)]
    struct RefreshMock {
        refreshed_access_key: String,
    }

    #[async_trait]
    impl HttpClient for RefreshMock {
        async fn get<T: serde::de::DeserializeOwned>(
            &self,
            url: &str,
            auth_token: Option<&str>,
        ) -> Res<T> {
            if url.ends_with("/config.json") {
                let body = serde_json::json!({
                    "registryUrl": "https://registry.example.com",
                });
                return Ok(serde_json::from_value(body)?);
            }
            if url.contains("/api/auth/get_credentials") {
                assert_eq!(auth_token, Some("fresh-access-token"));
                let body = serde_json::json!({
                    "AccessKeyId": self.refreshed_access_key,
                    "SecretAccessKey": "refreshed-secret",
                    "SessionToken": "refreshed-session",
                    "Expiration": (chrono::Utc::now() + chrono::Duration::hours(1))
                        .to_rfc3339(),
                });
                return Ok(serde_json::from_value(body)?);
            }
            panic!("unexpected GET: {url}");
        }
        async fn head(&self, _url: &str) -> Res<HeaderMap> {
            unimplemented!("head not used")
        }
        async fn post<T: serde::de::DeserializeOwned>(
            &self,
            _url: &str,
            _form_data: &HashMap<String, String>,
        ) -> Res<T> {
            unimplemented!("fresh tokens → no token refresh")
        }
        async fn post_json<T: serde::de::DeserializeOwned, B: serde::Serialize + Send + Sync>(
            &self,
            _url: &str,
            _body: &B,
        ) -> Res<T> {
            unimplemented!("post_json not used")
        }
        async fn post_json_auth<
            T: serde::de::DeserializeOwned,
            B: serde::Serialize + Send + Sync,
        >(
            &self,
            _url: &str,
            _body: &B,
            _auth_token: &str,
        ) -> Res<T> {
            unimplemented!("post_json_auth not used")
        }
    }

    /// The core of the `ExpiredToken` fix: when on-disk credentials are
    /// expired but the access token is still fresh, `provide_credentials`
    /// must call the registry to mint new STS creds and return *those*,
    /// not the stale on-disk ones.
    #[test(tokio::test)]
    async fn test_quilt_credentials_provider_refreshes_when_expired() -> Res<()> {
        use std::str::FromStr;

        use tempfile::TempDir;

        use crate::io::storage::auth::AuthIo;
        use crate::io::storage::auth::Credentials as QuiltCreds;
        use crate::io::storage::auth::Tokens;

        let temp = TempDir::new()?;
        let paths = DomainPaths::new(temp.path().to_path_buf());
        let storage = Arc::new(LocalStorage::new());
        let host = Host::from_str("catalog.example.com").unwrap();

        let auth_io = AuthIo::new(Arc::clone(&storage), paths.auth_host(&host));
        // Expired credentials — force the refresh path.
        auth_io
            .write_credentials(&QuiltCreds {
                access_key: "STALE".to_string(),
                secret_key: "stale-secret".to_string(),
                token: "stale-session".to_string(),
                expires_at: chrono::Utc::now() - chrono::Duration::hours(1),
            })
            .await?;
        // Fresh access token — skip the OAuth refresh leg.
        auth_io
            .write_tokens(&Tokens {
                access_token: "fresh-access-token".to_string(),
                refresh_token: "refresh-token".to_string(),
                expires_at: chrono::Utc::now() + chrono::Duration::hours(1),
            })
            .await?;

        let provider = QuiltCredentialsProvider {
            auth: auth::Auth::new(paths, storage),
            http: RefreshMock {
                refreshed_access_key: "REFRESHED".to_string(),
            },
            host,
        };

        let sdk_creds = provider.provide_credentials().await.unwrap();
        assert_eq!(sdk_creds.access_key_id(), "REFRESHED");
        assert_eq!(sdk_creds.secret_access_key(), "refreshed-secret");
        assert_eq!(sdk_creds.session_token(), Some("refreshed-session"));
        Ok(())
    }
}
