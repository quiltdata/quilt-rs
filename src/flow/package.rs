use chrono;
use tokio_stream::StreamExt;
use tracing::log;

use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::manifest::tag_latest;
use crate::io::manifest::tag_timestamp;
use crate::io::manifest::upload_manifest;
use crate::io::manifest::RowsStream;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Header;
use crate::paths::DomainPaths;
use crate::perf::Measure;
use crate::uri::ManifestUri;
use crate::uri::S3PackageUri;
use crate::uri::S3Uri;
use crate::Res;

async fn stream_objects(remote: &impl Remote, listing_uri: S3Uri) -> impl RowsStream + '_ {
    remote
        .list_objects(listing_uri.clone())
        .await
        .map(|result| {
            result.map(move |objs| objs.into_iter().map(|obj| obj.map(|o| o.into())).collect())
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
) -> Res<ManifestUri> {
    log::debug!("Source URI: {:?}, target URI: {:?}", source_uri, dest_uri);
    // TODO: make get_object_attributes() calls concurrently across list_objects() pages
    // TODO: increase concurrency, to do that we need to figure out how to deal
    //       with fd limits on Mac by default it's 256
    // TODO: s3 uri key ends with / and has no version
    // FIXME: filter or fail on keys with `.` or `..` in path segments as quilt3 do

    let perf = Measure::start();
    let stream = Box::pin(stream_objects(remote, source_uri.clone()).await);
    let manifest_path = |t: &str| paths.manifest_cache(&source_uri.bucket, t);
    let (cache_path, top_hash) =
        build_manifest_from_rows_stream(storage, manifest_path, Header::default(), stream).await?;

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
