//!
//! Table4 is a wrapper for arrow-rs's Table, the native Manifest format for quilt4.
//! It uses UPath to transparently read and write to/from local and remote filesystems,
//! and provides methods to read/write (decode/encode) quilt3's JSONL format
//!

use std::{
    io::{Error, ErrorKind},
    sync::Arc,
};

use arrow::error::ArrowError;
use arrow::record_batch::RecordBatch;
use aws_sdk_s3::config::ProvideCredentials;
use object_store::{aws::AmazonS3Builder, ObjectStore};
use parquet::arrow::{async_reader::ParquetObjectReader, ParquetRecordBatchStreamBuilder};
use tokio_stream::StreamExt;

use crate::s3_utils::get_region_for_bucket;

use super::{row4::Row4, upath::UPath};
use serde::{Deserialize, Serialize};

const HEADER_ROW: &str = ".";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Table {
    #[serde(skip)]
    records: Vec<RecordBatch>, // Vec<RecordBatch>? DataFusion?
    path3: Option<UPath>,
    path4: Option<UPath>,
}

impl Table {
    pub fn new(path: Option<UPath>) -> Self {
        Table {
            records: vec![],
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

    // Read quilt4's Parquet format
    pub async fn read4(&self) -> Result<Self, ArrowError> {
        let upath = self
            .path4
            .as_ref()
            .ok_or(ArrowError::NotYetImplemented("only path4 supported".into()))?;

        let mut records = vec![];

        match upath {
            UPath::Local(path) => {
                let file = tokio::fs::File::open(&path).await?;
                let mut stream = ParquetRecordBatchStreamBuilder::new(file).await?.build()?;
                while let Some(item) = stream.next().await {
                    records.push(item?);
                }
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
                let mut stream = ParquetRecordBatchStreamBuilder::new(reader)
                    .await?
                    .build()?;
                while let Some(item) = stream.next().await {
                    records.push(item?);
                }
            }
        };

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
    pub fn get_row(&self, _name: &str) -> Option<Row4> {
        // Implementation goes here
        unimplemented!()
    }

    pub fn get_header(&self) -> Option<Row4> {
        self.get_row(&HEADER_ROW.to_string())
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
        assert_eq!(new_table.records.len(), 1);
        assert_eq!(new_table.records[0].num_rows(), 2);
    }

    #[tokio::test]
    async fn read_existing_s3() {
        let url = "s3://udp-spec/.quilt/packages/122045ee6d96fd1cd8d1555e2d86e7e4a1699c05eeba9a555a4ea789004816abb592.parquet";
        let table = Table::new(Some(UPath::parse(url).unwrap()));
        let new_table = table.read4().await.unwrap();
        assert_eq!(new_table.records.len(), 1);
        assert_eq!(new_table.records[0].num_rows(), 3);
    }
}
