use aws_sdk_s3::types::Object;
use chrono;
use futures::future::try_join_all;
use tokio_stream::StreamExt;
use tracing::log;

use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::manifest::tag_latest;
use crate::io::manifest::tag_timestamp;
use crate::io::manifest::upload_manifest;
use crate::io::manifest::RowsStream;
use crate::io::remote::Remote;
use crate::io::remote::S3Attributes;
use crate::io::storage::Storage;
use crate::manifest::Row;
use crate::paths::DomainPaths;
use crate::perf::Measure;
use crate::uri::ManifestUri;
use crate::uri::S3PackageUri;
use crate::uri::S3Uri;
use crate::Error;

async fn get_object_attributes_inner(
    storage: &impl Storage,
    remote: &impl Remote,
    listing_uri: &S3Uri,
    object: Result<Object, Error>,
) -> Result<S3Attributes, Error> {
    let object_key = object?
        .key
        .clone()
        .expect("object key expected to be present");
    match remote.get_object_attributes(listing_uri, &object_key).await {
        Ok(attrs) => Ok(attrs),
        Err(err) => {
            log::warn!("Error getting attributes: {}", err);
            storage
                .get_object_attributes(listing_uri, &object_key)
                .await
        }
    }
}

async fn get_object_attributes(
    storage: &impl Storage,
    remote: &impl Remote,
    listing_uri: S3Uri,
    objects: Result<Vec<Result<Object, Error>>, Error>,
) -> Result<Vec<S3Attributes>, Error> {
    try_join_all(
        objects?
            .into_iter()
            .map(|object| get_object_attributes_inner(storage, remote, &listing_uri, object))
            .collect::<Vec<_>>(),
    )
    .await
}

async fn stream_objects<'a>(
    storage: &'a impl Storage,
    remote: &'a impl Remote,
    listing_uri: S3Uri,
) -> impl RowsStream + 'a {
    let stream = remote.list_objects(listing_uri.clone()).await;
    stream
        .then(move |objs| get_object_attributes(storage, remote, listing_uri.clone(), objs))
        .map(|result| {
            result.map(move |objs| {
                objs.into_iter()
                    .map(|obj| Ok(Row::from(obj)))
                    .collect::<Vec<Result<Row, Error>>>()
            })
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
) -> Result<ManifestUri, Error> {
    log::debug!("Source URI: {:?}, target URI: {:?}", source_uri, dest_uri);
    // TODO: make get_object_attributes() calls concurrently across list_objects() pages
    // TODO: increase concurrency, to do that we need to figure out how to deal
    //       with fd limits on Mac by default it's 256
    // TODO: s3 uri key ends with / and has no version
    // FIXME: filter or fail on keys with `.` or `..` in path segments as quilt3 do

    let perf = Measure::start();
    let stream = Box::pin(stream_objects(storage, remote, source_uri.clone()).await);
    let manifest_path = |t: &str| paths.manifest_cache(&source_uri.bucket, t);
    let (cache_path, top_hash) =
        build_manifest_from_rows_stream(storage, manifest_path, Row::default_header(), stream)
            .await?;

    let S3PackageUri {
        bucket, namespace, ..
    } = dest_uri;

    let manifest_uri = ManifestUri {
        bucket,
        namespace,
        hash: top_hash,
    };
    let perf = perf.elapsed();
    log::info!("Created manifest {:?} for {}", manifest_uri, perf);
    upload_manifest(storage, remote, &manifest_uri, &cache_path).await?;
    log::debug!("Manifest uploaded for {}", perf.elapsed());
    tag_timestamp(remote, &manifest_uri, chrono::Utc::now()).await?;
    log::debug!("Timestamp tag uploaded");
    tag_latest(remote, &manifest_uri).await?;
    log::debug!("Latest uploaded");

    Ok(manifest_uri)
}
