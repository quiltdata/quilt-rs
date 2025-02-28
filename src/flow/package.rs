use aws_sdk_s3::types::Object;
use chrono;
use futures::future::try_join_all;
use tokio_stream::StreamExt;
use tracing::{debug, info, warn};

use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::manifest::tag_latest;
use crate::io::manifest::tag_timestamp;
use crate::io::manifest::upload_manifest;
use crate::io::manifest::RowsStream;
use crate::io::remote::Remote;
use crate::io::remote::S3Attributes;
use crate::io::remote::StreamItem;
use crate::io::storage::Storage;
use crate::manifest::Header;
use crate::manifest::JsonObject;
use crate::manifest::Row;
use crate::paths::DomainPaths;
use crate::perf::Measure;
use crate::uri::Host;
use crate::uri::ManifestUri;
use crate::uri::S3PackageUri;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

async fn get_object_attributes_inner(
    storage: &impl Storage,
    remote: &impl Remote,
    host: &Option<Host>,
    listing_uri: &S3Uri,
    object: Res<Object>,
) -> Res<S3Attributes> {
    let obj = object?;
    let key = obj.key.clone().ok_or(Error::ObjectKey)?;
    match remote.get_object_attributes(host, listing_uri, &obj).await {
        Ok(attrs) => Ok(attrs),
        Err(Error::Checksum(msg)) => {
            debug!("{}", msg);
            debug!(
                "⏳ Calculating checksum for bucket {} key {}",
                &listing_uri.bucket, &key
            );
            let stream = remote
                .get_object_stream(
                    host,
                    &S3Uri {
                        bucket: listing_uri.bucket.clone(),
                        key,
                        version: None,
                    },
                )
                .await?;
            storage
                .get_object_attributes(stream, listing_uri, &obj)
                .await
        }
        Err(err) => {
            warn!("❌ Error getting attributes: {}", err);
            Err(err)
        }
    }
}

async fn get_object_attributes(
    storage: &impl Storage,
    remote: &impl Remote,
    host: &Option<Host>,
    listing_uri: &S3Uri,
    objects: StreamItem,
) -> Res<Vec<S3Attributes>> {
    try_join_all(
        objects?
            .into_iter()
            .map(|object| get_object_attributes_inner(storage, remote, host, listing_uri, object))
            .collect::<Vec<_>>(),
    )
    .await
}

async fn stream_objects<'a>(
    storage: &'a impl Storage,
    remote: &'a impl Remote,
    host: &'a Option<Host>,
    listing_uri: &'a S3Uri,
) -> impl RowsStream + 'a {
    let stream = remote.list_objects(host, listing_uri).await;
    stream
        .then(move |objs| get_object_attributes(storage, remote, host, listing_uri, objs))
        .map(|result| {
            result.map(move |objs| objs.into_iter().map(|obj| Ok(Row::from(obj))).collect())
        })
}

/// Lists the objects from S3 prefix as a stream and creates a package (manifest) from it.
/// It creates manifest in temporary directory then uploads it to the remote.
pub async fn package_s3_prefix(
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    source_uri: &S3Uri,
    dest_uri: S3PackageUri,
    message: Option<String>,
    user_meta: Option<JsonObject>,
) -> Res<ManifestUri> {
    info!(
        "⏳ Creating package from {} S3 prefix at {}",
        source_uri, dest_uri
    );
    // TODO: make get_object_attributes() calls concurrently across list_objects() pages
    // TODO: increase concurrency, to do that we need to figure out how to deal
    //       with fd limits on Mac by default it's 256
    // TODO: s3 uri key ends with / and has no version
    // FIXME: filter or fail on keys with `.` or `..` in path segments as quilt3 do

    let perf = Measure::start();
    let stream = Box::pin(stream_objects(storage, remote, &dest_uri.catalog, source_uri).await);
    let dest_dir = paths.manifest_cache_dir(&source_uri.bucket);
    let header = Header::new(message, user_meta, None);
    let (cache_path, top_hash) =
        build_manifest_from_rows_stream(storage, dest_dir, header, stream).await?;

    let S3PackageUri {
        bucket, namespace, ..
    } = dest_uri;

    let manifest_uri = ManifestUri {
        bucket,
        namespace,
        hash: top_hash,
        catalog: dest_uri.catalog,
    };
    info!(
        "✔️ Created manifest {} for {}",
        manifest_uri.display(),
        perf.elapsed()
    );

    debug!("⏳ Uploading manifest to remote storage");
    upload_manifest(storage, remote, &manifest_uri, &cache_path).await?;
    debug!("✔️ Manifest uploaded ({})", perf.elapsed());

    debug!("⏳ Adding timestamp tag");
    tag_timestamp(remote, &manifest_uri, chrono::Utc::now()).await?;
    debug!("✔️ Timestamp tag uploaded");

    debug!("⏳ Setting as latest version");
    tag_latest(remote, &manifest_uri).await?;
    debug!("✔️ Latest tag uploaded");

    info!(
        "✔️ Successfully created and uploaded package for {}",
        perf.elapsed()
    );

    Ok(manifest_uri)
}

#[cfg(test)]
mod tests {
    use super::*;

    use aws_sdk_s3::types::Object;

    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;

    #[tokio::test]
    async fn test_get_object_attributes_inner_success() -> Res {
        let remote = MockRemote::default();

        // Create a mock object in S3
        let listing_uri = S3Uri::try_from("s3://test-bucket/directory/")?;
        let object_uri = S3Uri::try_from("s3://test-bucket/directory/test-key")?;
        remote
            .put_object(&None, &object_uri, b"test content".to_vec())
            .await?;

        // Create mock S3 Object
        let object = Object::builder()
            .key("directory/test-key".to_string())
            .size(12)
            .build();

        let result =
            get_object_attributes_inner(&remote.storage, &remote, &None, &listing_uri, Ok(object))
                .await;

        let attrs = result.unwrap();
        assert_eq!(attrs.size, 12);
        assert_eq!(attrs.listing_uri.key, "directory/");
        assert_eq!(attrs.object_uri.key, "directory/test-key");
        Ok(())
    }

    #[tokio::test]
    async fn test_get_object_attributes_inner_not_found() -> Res {
        let storage = MockStorage::default();
        let remote = MockRemote::default();

        // Create mock S3 Object pointing to non-existent key
        let s3_uri = S3Uri::try_from("s3://test-bucket/nonexistent-key")?;
        let object = Object::builder()
            .key("nonexistent-key".to_string())
            .size(12)
            .build();

        let result =
            get_object_attributes_inner(&storage, &remote, &None, &s3_uri, Ok(object)).await;
        println!("RESULT {:?}", result);
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("NoSuchKey"));
        Ok(())
    }
}
