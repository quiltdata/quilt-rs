use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::RwLock;

use aws_config::BehaviorVersion;
use aws_credential_types::provider::error::CredentialsError;
use aws_credential_types::provider::future;
use aws_credential_types::provider::ProvideCredentials;
use aws_credential_types::Credentials;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::primitives::ByteStream;
use aws_types::region::Region;
use tracing::debug;
use tracing::info;
use tracing::warn;

use crate::auth;
use crate::auth::OAuthParams;
use crate::checksum::ObjectHash;
use crate::error::LoginError;
use crate::error::RemoteCatalogError;
use crate::error::S3Error;
use crate::error::S3ErrorKind;
use crate::io::remote::describe_sdk_error;
use crate::io::remote::host::fetch_host_config;
use crate::io::remote::object::multipart_upload_and_sha256_chunksum;
use crate::io::remote::object::put_and_request_checksum;
use crate::io::remote::HostChecksums;
use crate::io::remote::HostConfig;
use crate::io::remote::HttpClient;
use crate::io::remote::Remote;
use crate::io::storage::auth::OAuthClient;
use crate::io::storage::LocalStorage;
use crate::paths::DomainPaths;
use crate::uri::Host;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

use crate::io::remote::RemoteObjectStream;

async fn find_bucket_region(client: &impl HttpClient, bucket: &str) -> Res<String> {
    match client
        .head(&format!("https://s3.amazonaws.com/{bucket}"))
        .await?
        .get("x-amz-bucket-region")
    {
        Some(location) => Ok(location.to_str()?.into()),
        // TODO: make a better error for invalid `.head()`
        None => Err(Error::RemoteCatalog(RemoteCatalogError::MissingHeader(
            "x-amz-bucket-region".to_string(),
        ))),
    }
}

async fn get_object_stream(client: &aws_sdk_s3::Client, s3_uri: &S3Uri) -> Res<RemoteObjectStream> {
    let result = client.get_object().bucket(&s3_uri.bucket).key(&s3_uri.key);
    let result = match &s3_uri.version {
        Some(version) => result.version_id(version),
        None => result,
    };

    let result = result.send().await.map_err(|err| match &err {
        SdkError::ServiceError(svc) if svc.err().is_no_such_key() => {
            Error::S3(S3Error::new(S3ErrorKind::NotFound(s3_uri.to_string())))
        }
        _ => Error::S3(S3Error::new(S3ErrorKind::Raw(describe_sdk_error(err)))),
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
#[derive(Clone, Debug)]
struct QuiltCredentialsProvider {
    auth: auth::Auth,
    http: crate::io::remote::client::ReqwestClient,
    host: Host,
}

impl ProvideCredentials for QuiltCredentialsProvider {
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
    s3: RwLock<HashMap<CredsRef, aws_sdk_s3::Client>>,
    regions: RwLock<HashMap<String, Region>>,
}

impl RemoteS3 {
    pub fn new(paths: DomainPaths, storage: LocalStorage) -> Self {
        RemoteS3 {
            http: crate::io::remote::client::ReqwestClient::new(),
            s3: RwLock::new(HashMap::new()),
            regions: RwLock::new(HashMap::new()),
            auth: auth::Auth::new(paths, Arc::new(storage)),
        }
    }

    pub fn try_clone(&self) -> Res<Self> {
        let s3 = match self.s3.read() {
            Ok(s3) => s3.clone(),
            Err(_) => return Err(Error::S3(S3Error::new(S3ErrorKind::RemoteInit))),
        };
        let regions = match self.regions.read() {
            Ok(regions) => regions.clone(),
            Err(_) => return Err(Error::S3(S3Error::new(S3ErrorKind::RemoteInit))),
        };
        Ok(RemoteS3 {
            http: self.http.clone(),
            s3: RwLock::new(s3),
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
        host: &Option<Host>,
        region: aws_types::region::Region,
    ) -> Res<aws_sdk_s3::Client> {
        let creds_ref = CredsRef {
            region: region.clone(),
            host: host.clone(),
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

        info!("⏳ Creating new S3 client for region {:?}", region);
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
            Some(ref host) => {
                // Smoke-test eagerly so `Login required` surfaces now rather
                // than inside a later S3 call. The provider below handles
                // subsequent refreshes per-request.
                self.auth
                    .get_credentials_or_refresh(&self.http, host)
                    .await?;
                debug!("✔️ Got credentials for host {:?}", host);
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
        debug!("✔️ created new S3 client for region {:?}", region);

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

    async fn get_client_for_bucket(
        &self,
        host: &Option<Host>,
        bucket: &str,
    ) -> Res<aws_sdk_s3::Client> {
        let region = self.get_region_for_bucket(bucket).await?.clone();
        self.get_client_for_region(host, region)
            .await
            .map_err(|e| match e {
                Error::Login(LoginError::Required(_)) | Error::S3(_) => e,
                _ => Error::S3(S3Error {
                    host: host.to_owned(),
                    kind: S3ErrorKind::Client(e.to_string()),
                }),
            })
    }
}

impl Remote for RemoteS3 {
    async fn exists(&self, host: &Option<Host>, s3_uri: &S3Uri) -> Res<bool> {
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
            Err(err) => {
                warn!("❌ Failed to check object existence at {}: {}", s3_uri, err);
                Err(Error::S3(S3Error {
                    host: host.to_owned(),
                    kind: S3ErrorKind::Exists(describe_sdk_error(err)),
                }))
            }
        }
    }

    async fn get_object_stream(
        &self,
        host: &Option<Host>,
        s3_uri: &S3Uri,
    ) -> Res<RemoteObjectStream> {
        debug!(
            "⏳ Getting object stream - host: {:?}, uri: {}",
            host, s3_uri
        );
        let client = self.get_client_for_bucket(host, &s3_uri.bucket).await?;
        match get_object_stream(&client, s3_uri).await {
            Ok(stream) => {
                info!("✔️ Created stream for object {}", s3_uri);
                Ok(stream)
            }
            Err(e) if e.is_not_found() => {
                info!("ℹ️ Object not found: {}", s3_uri);
                Err(e)
            }
            Err(e) => {
                warn!("❌ Failed to create stream for {}: {}", s3_uri, e);
                Err(Error::S3(S3Error {
                    host: host.to_owned(),
                    kind: S3ErrorKind::GetObjectStream(e.to_string()),
                }))
            }
        }
    }

    async fn put_object(
        &self,
        host: &Option<Host>,
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
            .map_err(|err| {
                Error::S3(S3Error {
                    host: host.to_owned(),
                    kind: S3ErrorKind::PutObject(describe_sdk_error(err)),
                })
            })?;

        Ok(())
    }

    async fn resolve_url(&self, host: &Option<Host>, s3_uri: &S3Uri) -> Res<S3Uri> {
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
            Err(err) => Err(Error::S3(S3Error {
                host: host.to_owned(),
                kind: S3ErrorKind::ResolveUrl(describe_sdk_error(err)),
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
            .get_client_for_bucket(&host_config.host, &dest_uri.bucket)
            .await?;

        if host_config.checksums == HostChecksums::Sha256Chunked && size != 0 {
            multipart_upload_and_sha256_chunksum(client, source_path, dest_uri, size).await
        } else {
            put_and_request_checksum(client, source_path, dest_uri, host_config).await
        }
    }

    async fn host_config(&self, host: &Option<Host>) -> Res<HostConfig> {
        fetch_host_config(&self.http, host).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use std::io::Write;
    use tempfile::NamedTempFile;

    use crate::fixtures::objects::less_than_8mb;
    use crate::fixtures::objects::zero_bytes;
    use crate::fixtures::objects::LESS_THAN_8MB_HASH_B64;
    use crate::fixtures::objects::ZERO_HASH_B64;
    use crate::io::storage::LocalStorage;
    use crate::paths::DomainPaths;

    #[test(tokio::test)]
    async fn test_multipart_upload() -> Res<()> {
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
    async fn test_zero_bytes_upload() -> Res<()> {
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
    async fn test_crc64_upload() -> Res<()> {
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
}
