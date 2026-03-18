use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::sync::RwLock;

use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::primitives::ByteStream;
use aws_types::region::Region;
use tracing::debug;
use tracing::info;
use tracing::warn;

use crate::auth;
use crate::auth::OAuthParams;
use crate::checksum::ObjectHash;
use crate::error::AuthError;
use crate::error::S3Error;
use crate::io::remote::host::fetch_host_config;
use crate::io::remote::object::multipart_upload_and_sha256_chunksum;
use crate::io::remote::object::put_and_request_checksum;
use crate::io::remote::HostChecksums;
use crate::io::remote::HostConfig;
use crate::io::remote::HttpClient;
use crate::io::remote::Remote;
use crate::io::storage::auth::AuthIo;
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
        None => Err(Error::MissingHTTPHeader("x-amz-bucket-region".to_string())),
    }
}

async fn get_object_stream(client: &aws_sdk_s3::Client, s3_uri: &S3Uri) -> Res<RemoteObjectStream> {
    let result = client.get_object().bucket(&s3_uri.bucket).key(&s3_uri.key);
    let result = match &s3_uri.version {
        Some(version) => result.version_id(version),
        None => result,
    };

    let result = result
        .send()
        .await
        .map_err(|err| Error::S3Raw(DisplayErrorContext(err).to_string()))?;
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
            Err(_) => return Err(Error::RemoteInit),
        };
        let regions = match self.regions.read() {
            Ok(regions) => regions.clone(),
            Err(_) => return Err(Error::RemoteInit),
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
                .map_err(|e| Error::PoisonLock(e.to_string()))?
                .get(bucket)
            {
                return Ok(region.clone());
            }
        }

        let region = find_bucket_region(&self.http, bucket).await?;

        let mut map = self
            .regions
            .write()
            .map_err(|e| Error::PoisonLock(e.to_string()))?;
        match map.entry(bucket.to_owned()) {
            Entry::Occupied(entry) => Ok(entry.get().clone()),
            Entry::Vacant(entry) => Ok(entry.insert(Region::new(region)).clone()),
        }
    }

    async fn get_client_for_region(
        &self,
        host: &Option<Host>,
        region: aws_types::region::Region,
    ) -> Res<aws_sdk_s3::Client> {
        let creds_ref = CredsRef {
            region: region.clone(),
            host: host.clone(),
        };

        // Try to get existing client from cache and check if credentials are valid
        {
            // Check if we have a valid cached client
            let cached_client = {
                let map = self
                    .s3
                    .read()
                    .map_err(|e| Error::PoisonLock(e.to_string()))?;
                map.get(&creds_ref).cloned()
            };

            if let Some(client) = cached_client {
                if let Some(host) = &host {
                    // If credentials saved, check if they are valid
                    let auth_io =
                        AuthIo::new(self.auth.storage.clone(), self.auth.paths.auth_host(host));
                    match auth_io.read_credentials().await {
                        // We ensured credentials are not expired inside `read_credentials`
                        Ok(Some(_)) => {
                            info!(
                                "✔️ Using cached S3 client with valid credentials for {}",
                                host
                            );
                            return Ok(client);
                        }
                        Ok(None) => {
                            info!(
                                "❌ No credentials found for {}, will create new client",
                                host
                            );
                        }
                        Err(e) => {
                            warn!("❌ Failed to read credentials for {}: {}", host, e);
                            return Err(Error::Auth(
                                host.to_owned(),
                                AuthError::CredentialsRead(e.to_string()),
                            ));
                        }
                    }
                    // Credentials expired or missing, will create new client with refreshed credentials
                } else {
                    // For clients with inferred credentials from ~/.aws, reuse existing client
                    info!("✔️ Using cached S3 client with AWS credentials");
                    return Ok(client);
                }
            }
        }

        info!("⏳ Creating new S3 client for region {:?}", region);
        // Create new client
        let config = match host {
            None => {
                info!("⏳ No `&catalog=`, so we use credentials in ~/.aws");
                let config = aws_config::defaults(BehaviorVersion::latest())
                    .region(region.clone())
                    .load()
                    .await;

                // Check if we have valid credentials
                if config.credentials_provider().is_none() {
                    return Err(Error::LoginRequired(None));
                }
                config
            }
            Some(ref host) => {
                let creds = self
                    .auth
                    .get_credentials_or_refresh(&self.http, host)
                    .await?;
                debug!("✔️ Got credentials for host {:?}", host);
                aws_config::defaults(BehaviorVersion::latest())
                    .region(region.clone())
                    .credentials_provider(Credentials::new(
                        creds.access_key,
                        &creds.secret_key,
                        Some(creds.token),
                        Some(creds.expires_at.into()),
                        "quilt-registry",
                    ))
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
            .map_err(|e| Error::PoisonLock(e.to_string()))?;

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
                Error::LoginRequired(_) | Error::S3(_, _) => e,
                _ => Error::S3(
                    host.to_owned(),
                    S3Error::Client(DisplayErrorContext(e).to_string()),
                ),
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
                Err(Error::S3(
                    host.to_owned(),
                    S3Error::Exists(DisplayErrorContext(err).to_string()),
                ))
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
            Err(e) => {
                warn!("❌ Failed to create stream for {}: {}", s3_uri, e);
                Err(Error::S3(
                    host.to_owned(),
                    S3Error::GetObjectStream(DisplayErrorContext(e).to_string()),
                ))
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
                Error::S3(
                    host.to_owned(),
                    S3Error::PutObject(DisplayErrorContext(err).to_string()),
                )
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
            Err(err) => Err(Error::S3(
                host.to_owned(),
                S3Error::ResolveUrl(DisplayErrorContext(err).to_string()),
            )),
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
}
