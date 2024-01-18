use std::sync::Arc;

use arrow_array::{types::Utf8Type, GenericByteArray, Int64Array};
use aws_credential_types::provider::ProvideCredentials;
use futures_util::stream::TryStreamExt;
use object_store::{
    aws::{resolve_bucket_region, AmazonS3Builder},
    path::Path,
    ClientOptions, GetOptions, ObjectStore,
};
use parquet::arrow::{async_reader::ParquetObjectReader, ParquetRecordBatchStreamBuilder};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ManifestRow {
    name: String,
    place: String,
    size: i64,
    multihash: String,
    // workflow: ???,
    info: String,
    meta: String,
}

#[tokio::main]
async fn main() {
    // Object store doesn't read credentials from AWS config,
    // so we have to pass them explicitly.
    // Probably it's not so hard to replicate AWS SDK behavior which tries
    // to get credentials from multiple sources.
    let sdk_config = aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
    let cp = sdk_config.credentials_provider().expect("cp");
    let creds = cp.provide_credentials().await.expect("creds");

    let bucket_name = "udp-spec";
    let s3 = AmazonS3Builder::new()
        .with_bucket_name(bucket_name)
        .with_region(
            resolve_bucket_region(bucket_name, &ClientOptions::new())
                .await
                .unwrap(),
        )
        .with_access_key_id(creds.access_key_id())
        .with_secret_access_key(creds.secret_access_key())
        // .with_token(creds.session_token().expect("no token").to_string())
        .build()
        .expect("msg");

    // Get object version
    let get_result = s3
        .get_opts(
            &Path::from("spec/parquet/READ ME.md"),
            GetOptions {
                version: Some("r5iu2bKoYyeUUt5veiBM8yfLRgu06fe7".into()),
                ..GetOptions::default()
            },
        )
        .await
        .unwrap();

    dbg!(&get_result.meta);
    dbg!(String::from_utf8(get_result.bytes().await.unwrap().to_vec()).unwrap());

    // Read parquet manifest
    let manifest_location = ".quilt/packages/122045ee6d96fd1cd8d1555e2d86e7e4a1699c05eeba9a555a4ea789004816abb592.parquet";
    let obj_meta = s3.head(&Path::from(manifest_location)).await.unwrap();
    let reader = ParquetObjectReader::new(Arc::new(s3), obj_meta);
    let mut reader_stream = ParquetRecordBatchStreamBuilder::new(reader)
        .await
        .unwrap()
        .build()
        .unwrap();

    dbg!(reader_stream.schema());
    while let Some(item) = reader_stream.try_next().await.unwrap() {
        for i in 0..item.num_rows() {
            let x = ManifestRow {
                name: item
                    .column_by_name("name")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<GenericByteArray<Utf8Type>>()
                    .unwrap()
                    .value(i)
                    .into(),
                place: item
                    .column_by_name("place")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<GenericByteArray<Utf8Type>>()
                    .unwrap()
                    .value(i)
                    .into(),
                size: item
                    .column_by_name("size")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<Int64Array>()
                    .unwrap()
                    .value(i)
                    .into(),
                multihash: item
                    .column_by_name("multihash")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<GenericByteArray<Utf8Type>>()
                    .unwrap()
                    .value(i)
                    .into(),
                info: item
                    .column_by_name("info.json")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<GenericByteArray<Utf8Type>>()
                    .unwrap()
                    .value(i)
                    .into(),
                meta: item
                    .column_by_name("meta.json")
                    .unwrap()
                    .as_any()
                    .downcast_ref::<GenericByteArray<Utf8Type>>()
                    .unwrap()
                    .value(i)
                    .into(),
            };
            dbg!(&x);
        }
    }
}
