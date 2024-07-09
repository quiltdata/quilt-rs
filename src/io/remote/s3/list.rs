use async_stream::try_stream;
use aws_sdk_s3::error::DisplayErrorContext;
use aws_sdk_s3::types::Object;
use futures::future::try_join_all;
use tokio_stream::Stream;
use tokio_stream::StreamExt;

use crate::io::remote::get_client_for_bucket;
use crate::io::remote::EntriesStream;
use crate::io::remote::Remote;
use crate::io::Entry;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

use crate::io::remote::s3::attributes::get_object_attributes;

const LIST_OBJECTS_V2_MAX_KEYS: i32 = 1_00;

async fn get_objects_chunk_attributes(
    remote: &impl Remote,
    listing_uri: S3Uri,
    objects: StreamItem,
) -> Res<Vec<Entry>> {
    try_join_all(
        objects?
            .into_iter()
            .map(|object| get_object_attributes(remote, &listing_uri, object))
            .collect::<Vec<_>>(),
    )
    .await
}

type StreamItem = Res<Vec<Res<Object>>>;
trait ObjectsStream: Stream<Item = StreamItem> {}
impl<T: Stream<Item = StreamItem>> ObjectsStream for T {}

async fn list_objects(listing_uri: S3Uri) -> impl ObjectsStream {
    try_stream! {
        let client = get_client_for_bucket(&listing_uri.bucket).await?;
        let mut paginated_stream = client
            .list_objects_v2()
            .bucket(&listing_uri.bucket)
            .prefix(&listing_uri.key)
            .into_paginator()
            .page_size(LIST_OBJECTS_V2_MAX_KEYS) // XXX: this is to limit concurrency
            .send();
        while let Some(page) = paginated_stream.next().await {
            yield page
                .map_err(|err| Error::S3(DisplayErrorContext(err).to_string()))?
                .contents
                .into_iter()
                .flatten()
                .map(Ok)
                .collect::<Vec<_>>();
        }
    }
}

pub async fn stream(remote: &impl Remote, listing_uri: S3Uri) -> impl EntriesStream + '_ {
    list_objects(listing_uri.clone())
        .await
        .then(move |objs| get_objects_chunk_attributes(remote, listing_uri.clone(), objs))
        .map(|result| result.map(move |objs| objs.into_iter().map(Ok).collect()))
}
