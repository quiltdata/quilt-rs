use std::collections::BTreeMap;
use std::path::PathBuf;

use aws_sdk_s3::error::DisplayErrorContext;
use tracing::log;

use crate::io::manifest::tag_latest;
use crate::io::manifest::tag_timestamp;
use crate::io::manifest::upload_manifest;
use crate::io::remote::get_client_for_bucket;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::manifest::Row;
use crate::manifest::Table;
use crate::paths::DomainPaths;
use crate::uri::ManifestUri;
use crate::uri::S3PackageUri;
use crate::uri::S3Uri;
use crate::Error;

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
    let client = get_client_for_bucket(&source_uri.bucket).await?;

    // XXX: we need real API to build manifests
    let header = Row::default();
    let mut records: BTreeMap<PathBuf, Row> = BTreeMap::new();

    // FIXME: let mut table = Table?
    // FIXME: table.insert_record?

    let mut p = client
        .list_objects_v2()
        .bucket(&source_uri.bucket)
        .prefix(&source_uri.key)
        .into_paginator()
        .page_size(100) // XXX: this is to limit concurrency
        .send();
    while let Some(page) = p.next().await {
        let page = page.map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?;
        let page_contents_iter = page.contents.iter().flatten();

        for obj in page_contents_iter {
            let object_key = obj.key.clone().expect("object key expected to be present");
            let row: Row = match remote.get_object_attributes(source_uri, &object_key).await {
                Ok(attrs) => attrs,
                Err(err) => {
                    log::warn!("Error getting attributes: {}", err);
                    storage
                        .get_object_attributes(source_uri, &object_key)
                        .await?
                }
            }
            .into();
            records.insert(row.name.clone(), row);
        }
    }

    let table = Table { header, records };
    let manifest_uri = upload_manifest(storage, remote, paths, dest_uri.into(), table).await?;
    tag_timestamp(remote, &manifest_uri, chrono::Utc::now()).await?;
    tag_latest(remote, &manifest_uri).await?;

    Ok(manifest_uri)
}
