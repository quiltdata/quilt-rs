use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::path::Path;
use std::sync::RwLock;

use async_stream::try_stream;
use aws_config::BehaviorVersion;
use aws_credential_types::Credentials;
use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::error::SdkError;
use aws_sdk_s3::operation::get_object_attributes::GetObjectAttributesOutput;
use aws_sdk_s3::primitives::ByteStream;
use aws_sdk_s3::types::ChecksumAlgorithm;
use aws_sdk_s3::types::CompletedMultipartUpload;
use aws_sdk_s3::types::CompletedPart;
use aws_sdk_s3::types::Object;
use aws_smithy_types::byte_stream::Length;
use aws_types::region::Region;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;
use multihash::Multihash;
use parquet::data_type::AsBytes;
use tokio::io::AsyncRead;
use tracing::debug;
use tracing::info;
use tracing::warn;

use crate::auth;
use crate::checksum;
use crate::error::AuthError;
use crate::error::S3Error;
use crate::io::remote::HttpClient;
use crate::io::remote::ObjectsStream;
use crate::io::remote::Remote;
use crate::io::storage::auth::AuthIo;
use crate::io::storage::LocalStorage;
use crate::paths::DomainPaths;
use crate::uri::Host;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

const LIST_OBJECTS_V2_MAX_KEYS: i32 = 1_00;

use crate::io::remote::RemoteObjectStream;
use crate::io::remote::S3Attributes;

struct S3AttributesWrapper {
    pub hash: Multihash<256>,
    pub size: u64,
    pub version: String,
}

impl TryFrom<GetObjectAttributesOutput> for S3AttributesWrapper {
    type Error = Error;
    fn try_from(attrs: GetObjectAttributesOutput) -> Result<Self, Self::Error> {
        if attrs.delete_marker.is_some() {
            // Can happen if object is removed after it was listed but before attributes retrieved.
            return Err(Error::S3Raw("Object is a delete marker".to_string()));
        }

        let checksum = match checksum::get_compliant_chunked_checksum(&attrs) {
            Some(c) => c,
            None => return Err(Error::Checksum("missing checksum".to_string())),
        };
        let hash = Multihash::wrap(checksum::MULTIHASH_SHA256_CHUNKED, checksum.as_bytes())?;
        let size = attrs.object_size.expect("ObjectSize must be requested") as u64;
        Ok(S3AttributesWrapper {
            version: attrs.version_id.expect("VersionId must be requested"),
            hash,
            size,
        })
    }
}

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

async fn get_object(client: &aws_sdk_s3::Client, s3_uri: &S3Uri) -> Res<impl AsyncRead> {
    Ok(get_object_stream(client, s3_uri)
        .await?
        .body
        .into_async_read())
}

async fn put_object_and_checksum(
    client: aws_sdk_s3::Client,
    source_path: impl AsRef<Path>,
    dest_uri: &S3Uri,
    size: u64,
) -> Res<(S3Uri, Multihash<256>)> {
    let response = client
        .put_object()
        .bucket(&dest_uri.bucket)
        .key(&dest_uri.key)
        .body(ByteStream::from_path(source_path).await?)
        .checksum_algorithm(ChecksumAlgorithm::Sha256)
        .send()
        .await
        .map_err(|err| Error::S3Raw(DisplayErrorContext(err).to_string()))?;
    let s3_checksum_b64 = response
        .checksum_sha256
        .ok_or(Error::Checksum("missing checksum".to_string()))?;
    // let s3_checksum = BASE64_STANDARD.decode(s3_checksum_b64)?;
    let hash: Multihash<256> =
        checksum::ContentHash::SHA256Chunked(s3_checksum_b64.to_string()).try_into()?;
    let checksum = if size == 0 {
        // Edge case: a 0-byte upload is treated as an empty list of chunks, rather than
        // a list of a 0-byte chunk. Its checksum is sha256(''), NOT sha256(sha256('')).
        hash
    } else {
        // NOTE: we're calculating checksum of checksums here,
        //       not a checksum of the file
        // NOTE: in the current design, we're not using this checksum
        checksum::sha256(hash.digest()).await?.into()
    };

    Ok((
        S3Uri {
            version: response.version_id,
            ..dest_uri.clone()
        },
        checksum,
    ))
}

async fn multipart_upload_and_checksum(
    client: aws_sdk_s3::Client,
    source_path: impl AsRef<Path>,
    dest_uri: &S3Uri,
    size: u64,
) -> Res<(S3Uri, Multihash<256>)> {
    let (chunksize, num_chunks) = checksum::get_checksum_chunksize_and_parts(size);
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
        checksum::ContentHash::SHA256Chunked(checksum_b64.to_string()).try_into()?,
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

    async fn get_object(
        &self,
        host: &Option<Host>,
        s3_uri: &S3Uri,
    ) -> Res<impl AsyncRead + Send + Unpin> {
        debug!("⏳ Getting object - host: {:?}, uri: {}", host, s3_uri);
        let client = self.get_client_for_bucket(host, &s3_uri.bucket).await?;
        match get_object(&client, s3_uri).await {
            Ok(reader) => {
                info!("✔️ Successfully retrieved object from {}", s3_uri);
                Ok(reader)
            }
            Err(e) => {
                warn!("❌ Failed to get object from {}: {}", s3_uri, e);
                Err(Error::S3(
                    host.to_owned(),
                    S3Error::GetObject(DisplayErrorContext(e).to_string()),
                ))
            }
        }
    }

    async fn get_object_attributes(
        &self,
        host: &Option<Host>,
        listing_uri: &S3Uri,
        object: &Object,
    ) -> Res<S3Attributes> {
        let client = self
            .get_client_for_bucket(host, &listing_uri.bucket)
            .await?;
        let key = object.key.clone().ok_or(Error::ObjectKey)?;
        debug!(
            "⏳ Getting object attributes - host: {:?}, bucket: {}, key: {}",
            host, &listing_uri.bucket, key
        );
        match client
            .get_object_attributes()
            .bucket(&listing_uri.bucket)
            .key(key.clone())
            .object_attributes(aws_sdk_s3::types::ObjectAttributes::Checksum)
            .object_attributes(aws_sdk_s3::types::ObjectAttributes::ObjectParts)
            .object_attributes(aws_sdk_s3::types::ObjectAttributes::ObjectSize)
            .max_parts(checksum::MPU_MAX_PARTS as i32)
            .send()
            .await
        {
            Ok(attrs) => {
                let S3AttributesWrapper {
                    size,
                    hash,
                    version,
                } = attrs.try_into()?;
                let attributes = S3Attributes {
                    listing_uri: listing_uri.clone(),
                    object_uri: S3Uri {
                        bucket: listing_uri.bucket.clone(),
                        key: key.to_string(),
                        version: Some(version),
                    },
                    hash,
                    size,
                };
                info!(
                    "✔️ Retrieved attributes for {}/{} - size: {}, hash: {}",
                    listing_uri.bucket,
                    key,
                    size,
                    BASE64_STANDARD.encode(hash.digest())
                );
                Ok(attributes)
            }
            Err(err) => {
                warn!(
                    "❌ Failed to get attributes for {}/{}: {}",
                    listing_uri.bucket, key, err
                );
                Err(Error::S3(
                    host.to_owned(),
                    S3Error::GetObjectAttributes(DisplayErrorContext(err).to_string()),
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

    async fn list_objects(&self, host: &Option<Host>, listing_uri: &S3Uri) -> impl ObjectsStream {
        try_stream! {
            let client = self.get_client_for_bucket(host, &listing_uri.bucket).await?;
            let mut paginated_stream = client
                .list_objects_v2()
                .bucket(&listing_uri.bucket)
                .prefix(&listing_uri.key)
                .into_paginator()
                .page_size(LIST_OBJECTS_V2_MAX_KEYS) // XXX: this is to limit concurrency
                .send();
            while let Some(page) = paginated_stream.next().await {
                yield page
                    .map_err(|err| Error::S3(host.to_owned(), S3Error::ListObjects(DisplayErrorContext(err).to_string())))?
                    .contents
                    .into_iter()
                    .flatten()
                    .map(Ok)
                    .collect::<Vec<_>>();
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
    ) -> Res<(S3Uri, Multihash<256>)> {
        let client = self.get_client_for_bucket(host, &dest_uri.bucket).await?;
        {
            if size == 0 {
                put_object_and_checksum(client, source_path, dest_uri, size).await
            } else {
                multipart_upload_and_checksum(client, source_path, dest_uri, size).await
            }
        }
        .map_err(|err| {
            Error::S3(
                host.to_owned(),
                S3Error::UploadFile(DisplayErrorContext(err).to_string()),
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_get_object_attributes() -> Res {
        let listing_uri = S3Uri {
            bucket: "data-yaml-spec-tests".to_string(),
            ..S3Uri::default()
        };

        let object = Object::builder()
            .key("scale/10u/e0-0.txt")
            .size(1024)
            .build();

        let remote = RemoteS3::new(DomainPaths::default(), LocalStorage::default());
        let result = remote
            .get_object_attributes(&None, &listing_uri, &object)
            .await?;

        assert_eq!(
            result.object_uri,
            S3Uri {
                key: object.key().unwrap().to_string(),
                version: Some("jHb6DGN43Ex7EhbxZc2G9JnAkWSeTfEY".to_string()),
                ..listing_uri
            }
        );
        assert_eq!(
            result.hash,
            checksum::ContentHash::SHA256Chunked(
                "/UMjH1bsbrMLBKdd9cqGGvtjhWzawhz1BfrxgngUhVI=".to_string()
            )
            .try_into()?
        );
        assert_eq!(result.size, 29);
        Ok(())
    }

    #[tokio::test]
    async fn test_get_object_attributes_missing_checksum() {
        let listing_uri = S3Uri {
            bucket: "allencell".to_string(),
            key: "".to_string(),
            version: None,
        };
        let object = Object::builder().key("README.md").size(1024).build();

        let remote = RemoteS3::new(DomainPaths::default(), LocalStorage::default());
        let result = remote
            .get_object_attributes(&None, &listing_uri, &object)
            .await;

        match result {
            Err(Error::Checksum(msg)) => {
                assert_eq!(msg, "missing checksum");
            }
            _ => panic!("Expected Error::Checksum(\"missing checksum\"), got {result:?}"),
        }
    }
}
