use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::ChecksumAlgorithm;
use aws_sdk_s3::types::CompletedMultipartUpload;
use aws_sdk_s3::types::CompletedPart;
use aws_smithy_types::byte_stream::Length;
use aws_types::region::Region;
use tracing::debug;
use tracing::info;
use tracing::warn;

use crate::auth;
use crate::checksum::get_checksum_chunksize_and_parts;
use crate::checksum::hash_sha256_checksum;
use crate::checksum::ObjectHash;
use crate::checksum::Sha256ChunkedHash;
use crate::error::AuthError;
use crate::error::S3Error;
use crate::io::remote::host::fetch_host_config;
use crate::io::remote::{HostConfig, HttpClient, Remote};
use crate::io::storage::auth::AuthIo;
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

async fn put_object_and_checksum(
    client: aws_sdk_s3::Client,
    source_path: impl AsRef<Path>,
    dest_uri: &S3Uri,
    size: u64,
) -> Res<(S3Uri, ObjectHash)> {
    let response = client
        .put_object()
        .bucket(&dest_uri.bucket)
        .key(&dest_uri.key)
        .body(ByteStream::from_path(source_path).await?)
        .checksum_algorithm(ChecksumAlgorithm::Sha256)
        .send()
        .await
        .map_err(|err| Error::S3Raw(DisplayErrorContext(err).to_string()))?;
    let s3_checksum_sha256_b64 = response
        .checksum_sha256
        .ok_or(Error::Checksum("missing checksum".to_string()))?;

    let uri = S3Uri {
        version: response.version_id,
        ..dest_uri.clone()
    };

    let hash = match size {
        // Edge case: a 0-byte upload is treated as an empty list of chunks, rather than
        // a list of a 0-byte chunk. Its checksum is sha256(''), NOT sha256(sha256('')).
        0 => s3_checksum_sha256_b64,
        // NOTE: we're calculating checksum of checksums here,
        //       not a checksum of the file
        // NOTE: in the current design, we're not using this checksum
        _ => hash_sha256_checksum(s3_checksum_sha256_b64.as_str())
            .ok_or(Error::InvalidMultihash(s3_checksum_sha256_b64))?,
    };
    Ok((uri, Sha256ChunkedHash::try_from(hash.as_str())?.into()))
}

async fn multipart_upload_and_checksum(
    client: aws_sdk_s3::Client,
    source_path: impl AsRef<Path>,
    dest_uri: &S3Uri,
    size: u64,
) -> Res<(S3Uri, ObjectHash)> {
    let (chunksize, num_chunks) = get_checksum_chunksize_and_parts(size);
    let upload_id = client
        .create_multipart_upload()
        .bucket(&dest_uri.bucket)
        .key(&dest_uri.key)
        .checksum_algorithm(ChecksumAlgorithm::Sha256)
        .send()
        .await
        .map_err(|err| Error::S3Raw(DisplayErrorContext(err).to_string()))?
        .upload_id
        .ok_or(Error::UploadId("failed to get an UploadId".to_string()))?;

    let mut parts: Vec<CompletedPart> = Vec::new();
    for chunk_idx in 0..num_chunks {
        let part_number = chunk_idx as i32 + 1;
        let offset = chunk_idx * chunksize;
        let length = chunksize.min(size - offset);
        let chunk_body = ByteStream::read_from()
            .path(source_path.as_ref())
            .offset(offset)
            .length(Length::Exact(length)) // https://github.com/awslabs/aws-sdk-rust/issues/821
            .build()
            .await?;
        let part_response = client
            .upload_part()
            .bucket(&dest_uri.bucket)
            .key(&dest_uri.key)
            .upload_id(&upload_id)
            .part_number(part_number)
            .checksum_algorithm(ChecksumAlgorithm::Sha256)
            .body(chunk_body)
            .send()
            .await
            .map_err(|err| Error::S3Raw(DisplayErrorContext(err).to_string()))?;
        parts.push(
            CompletedPart::builder()
                .part_number(part_number)
                .e_tag(part_response.e_tag.unwrap_or_default())
                .checksum_sha256(part_response.checksum_sha256.unwrap_or_default())
                .build(),
        );
    }

    let response = client
        .complete_multipart_upload()
        .bucket(&dest_uri.bucket)
        .key(&dest_uri.key)
        .upload_id(&upload_id)
        .multipart_upload(
            CompletedMultipartUpload::builder()
                .set_parts(Some(parts))
                .build(),
        )
        .send()
        .await
        .map_err(|err| Error::S3Raw(DisplayErrorContext(err).to_string()))?;

    let s3_checksum = response
        .checksum_sha256
        .ok_or(Error::Checksum("missing checksum".to_string()))?;
    let (checksum_b64, _) = s3_checksum
        .split_once('-')
        .ok_or(Error::Checksum("unexpected checksum".to_string()))?;

    Ok((
        S3Uri {
            version: response.version_id,
            ..dest_uri.clone()
        },
        Sha256ChunkedHash::try_from(checksum_b64)?.into(),
    ))
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
            auth: auth::Auth::new(paths, storage),
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
        let client = self.get_client_for_bucket(host, &s3_uri.bucket).await?;
        client
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

    async fn upload_file(
        &self,
        host: &Option<Host>,
        source_path: impl AsRef<Path>,
        dest_uri: &S3Uri,
        size: u64,
    ) -> Res<(S3Uri, ObjectHash)> {
        let client = self.get_client_for_bucket(host, &dest_uri.bucket).await?;
        (if size == 0 {
            put_object_and_checksum(client, source_path, dest_uri, size).await
        } else {
            multipart_upload_and_checksum(client, source_path, dest_uri, size).await
        })
        .map(|(uri, hash)| (uri, hash.into()))
        .map_err(|err| {
            Error::S3(
                host.to_owned(),
                S3Error::UploadFile(DisplayErrorContext(err).to_string()),
            )
        })
    }

    async fn host_config(&self, host: &Option<Host>) -> Res<HostConfig> {
        match host {
            Some(host) => fetch_host_config(&self.http, &host.to_string()).await,
            None => Ok(HostConfig::default()),
        }
    }
}
