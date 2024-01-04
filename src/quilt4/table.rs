//!
//! Table4 is a wrapper for arrow-rs's Table, the native Manifest format for quilt4.
//! It uses UPath to transparently read and write to/from local and remote filesystems,
//! and provides methods to read/write (decode/encode) quilt3's JSONL format
//!

use std::{
    collections::BTreeMap,
    io::{Error, ErrorKind},
    sync::Arc,
};

use arrow::{
    array::{GenericByteArray, UInt64Array},
    datatypes::{BinaryType, Utf8Type},
    error::ArrowError,
};
use aws_sdk_s3::config::ProvideCredentials;
use multihash::Multihash;
use object_store::{aws::AmazonS3Builder, ObjectStore};
use parquet::arrow::{
    async_reader::{AsyncFileReader, ParquetObjectReader},
    ParquetRecordBatchStreamBuilder,
};
use tokio_stream::StreamExt;

use crate::s3_utils::get_region_for_bucket;

use super::{row4::Row4, upath::UPath};

const HEADER_ROW: &str = ".";

#[derive(Clone, Debug)]
pub struct Table {
    records: BTreeMap<String, Row4>,
    path3: Option<UPath>,
    path4: Option<UPath>,
}

impl Table {
    pub fn new(path: Option<UPath>) -> Self {
        Table {
            records: BTreeMap::new(),
            path3: None,
            path4: path.clone(),
        }
    }
    pub fn to_string(&self) -> String {
        format!("Table({:?})", self.path4)
            + &format!("({:?})\n", self.path3)
            + &format!("[\n{:?}\n]", self.records)
    }
    // Read quilt3's JSONL format
    pub fn read3(&self) -> Result<Self, ArrowError> {
        // Implementation goes here
        unimplemented!()
    }

    // Write quilt3's JSONL format
    pub fn write3(&self) -> Result<(), ArrowError> {
        // Implementation goes here
        unimplemented!()
    }

    async fn read_rows_impl<T>(reader: T) -> Result<BTreeMap<String, Row4>, ArrowError>
    where
        T: AsyncFileReader + Unpin + Send + 'static,
    {
        let mut stream = ParquetRecordBatchStreamBuilder::new(reader)
            .await?
            .build()?;

        let mut records = BTreeMap::new();
        while let Some(item) = stream.try_next().await? {
            let name_column = item
                .column_by_name("name")
                .ok_or(ArrowError::SchemaError("missing 'name'".into()))?
                .as_any()
                .downcast_ref::<GenericByteArray<Utf8Type>>()
                .ok_or(ArrowError::SchemaError("invalid 'name'".into()))?;
            let place_column = item
                .column_by_name("place")
                .ok_or(ArrowError::SchemaError("missing 'place'".into()))?
                .as_any()
                .downcast_ref::<GenericByteArray<Utf8Type>>()
                .ok_or(ArrowError::SchemaError("invalid 'place'".into()))?;
            let size_column = item
                .column_by_name("size")
                .ok_or(ArrowError::SchemaError("missing 'size'".into()))?
                .as_any()
                .downcast_ref::<UInt64Array>()
                .ok_or(ArrowError::SchemaError("invalid 'size'".into()))?;
            let multihash_column = item
                .column_by_name("multihash")
                .ok_or(ArrowError::SchemaError("missing 'multihash'".into()))?
                .as_any()
                .downcast_ref::<GenericByteArray<BinaryType>>()
                .ok_or(ArrowError::SchemaError("invalid 'multihash'".into()))?;
            let info_column = item
                .column_by_name("info.json")
                .ok_or(ArrowError::SchemaError("missing 'info.json'".into()))?
                .as_any()
                .downcast_ref::<GenericByteArray<Utf8Type>>()
                .ok_or(ArrowError::SchemaError("invalid 'info.json'".into()))?;
            let meta_column = item
                .column_by_name("meta.json")
                .ok_or(ArrowError::SchemaError("missing 'meta.json'".into()))?
                .as_any()
                .downcast_ref::<GenericByteArray<Utf8Type>>()
                .ok_or(ArrowError::SchemaError("invalid 'meta.json'".into()))?;

            for idx in 0..item.num_rows() {
                let name = name_column.value(idx);
                let hash = if name == HEADER_ROW {
                    Multihash::default()
                } else {
                    Multihash::from_bytes(multihash_column.value(idx))
                        .map_err(|err| ArrowError::SchemaError(err.to_string()))?
                };

                records.insert(
                    name.into(),
                    Row4 {
                        name: name.into(),
                        place: place_column.value(idx).into(),
                        path: None,
                        size: size_column.value(idx),
                        hash,
                        info: serde_json::from_str(info_column.value(idx))
                            .map_err(|err| ArrowError::SchemaError(err.to_string()))?,
                        meta: serde_json::from_str(meta_column.value(idx))
                            .map_err(|err| ArrowError::SchemaError(err.to_string()))?,
                    },
                );
            }
        }

        Ok(records)
    }

    // Read quilt4's Parquet format
    pub async fn read4(&self) -> Result<Self, ArrowError> {
        let upath = self
            .path4
            .as_ref()
            .ok_or(ArrowError::NotYetImplemented("only path4 supported".into()))?;

        let records = match upath {
            UPath::Local(path) => {
                let file = tokio::fs::File::open(&path).await?;
                Table::read_rows_impl(file).await
            }
            UPath::S3 { bucket, path } => {
                let region = get_region_for_bucket(bucket)
                    .await
                    .map_err(|err| Error::new(ErrorKind::Other, err))?;

                // TODO: Cache the credentials in s3_util or use s3_util's clients
                // TODO: Return custom errors instead of abusing io::Error.
                let sdk_config =
                    aws_config::load_defaults(aws_config::BehaviorVersion::latest()).await;
                let cp = sdk_config
                    .credentials_provider()
                    .ok_or(Error::new(ErrorKind::Other, "missing credentials"))?;
                let creds = cp
                    .provide_credentials()
                    .await
                    .map_err(|err| Error::new(ErrorKind::Other, err))?;

                let s3 = AmazonS3Builder::new()
                    .with_bucket_name(bucket)
                    .with_region(region.to_string())
                    .with_access_key_id(creds.access_key_id())
                    .with_secret_access_key(creds.secret_access_key())
                    .with_token(creds.session_token().unwrap_or_default())
                    .build()
                    .map_err(|err| Error::new(ErrorKind::Other, err))?;

                let obj_meta = s3
                    .head(path)
                    .await
                    .map_err(|err| Error::new(ErrorKind::Other, err))?;
                let reader = ParquetObjectReader::new(Arc::new(s3), obj_meta);
                Table::read_rows_impl(reader).await
            }
        }?;

        Ok(Self {
            records,
            path3: self.path3.clone(),
            path4: self.path4.clone(),
        })
    }

    // Write quilt4's Parquet format
    pub fn write4(&self) -> Result<(), ArrowError> {
        // Implementation goes here
        unimplemented!()
    }

    // Get a row from the table
    pub fn get_row(&self, name: &str) -> Option<&Row4> {
        self.records.get(name)
    }

    pub fn get_header(&self) -> Option<&Row4> {
        self.get_row(&HEADER_ROW)
    }
    // TBD: Store header metadata as PARQUET Metadata?

    pub fn list_names(&self) -> Vec<Row4> {
        // Implementation goes here
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use utils::local_uri_parquet;

    use super::*;

    #[tokio::test]
    async fn read_existing_local() {
        let table = Table::new(Some(UPath::parse(&local_uri_parquet()).unwrap()));
        let new_table = table.read4().await.unwrap();
        assert_eq!(new_table.records.len(), 3);

        let header = new_table.get_header().unwrap();
        assert_eq!(header.size, 0);

        let readme = new_table.get_row("READ ME.md").unwrap();
        assert_eq!(readme.size, 33);
    }
}
