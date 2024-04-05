//!
//! Table4 is a wrapper for arrow-rs's Table, the native Manifest format for quilt4.
//! It uses UPath to transparently read and write to/from local and remote filesystems,
//! and provides methods to read/write (decode/encode) quilt3's JSONL format
//!

use std::collections::BTreeMap;
use std::fmt;
// use std::io;
use std::path::PathBuf;
use std::sync::Arc;

use arrow::array::GenericByteArray;
use arrow::array::UInt64Array;
use arrow::datatypes::BinaryType;
use arrow::datatypes::DataType;
use arrow::datatypes::Field;
use arrow::datatypes::Schema;
use arrow::datatypes::Utf8Type;
use arrow::error::ArrowError;
use arrow::record_batch::RecordBatch;
use multihash::Multihash;
use parquet::arrow::async_reader::AsyncFileReader;
use parquet::arrow::AsyncArrowWriter;
use parquet::arrow::ParquetRecordBatchStreamBuilder;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use sha2::Digest;
use sha2::Sha256;
use tokio::fs;
use tokio::io::AsyncWrite;
use tokio_stream::StreamExt;

use crate::quilt::manifest::Manifest;
use crate::quilt::ContentHash;

use super::row4::Row4;
use crate::Error;

pub const HEADER_ROW: &str = ".";

#[derive(Clone, Debug, Default, PartialEq)]
pub struct Table {
    pub header: Row4,
    pub records: BTreeMap<String, Row4>,
}

impl Table {
    async fn read_rows_impl<T>(reader: T) -> Result<Self, ArrowError>
    where
        T: AsyncFileReader + Unpin + Send + 'static,
    {
        let mut stream = ParquetRecordBatchStreamBuilder::new(reader)
            .await?
            .build()?;

        let mut header: Option<Row4> = None;
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

                let row = Row4 {
                    name: name.into(),
                    place: place_column.value(idx).into(),
                    size: size_column.value(idx),
                    hash,
                    info: serde_json::from_str(info_column.value(idx))
                        .map_err(|err| ArrowError::SchemaError(err.to_string()))?,
                    meta: serde_json::from_str(meta_column.value(idx))
                        .map_err(|err| ArrowError::SchemaError(err.to_string()))?,
                };

                if name == HEADER_ROW {
                    header = Some(row);
                } else {
                    records.insert(name.into(), row);
                }
            }
        }

        Ok(Table {
            header: header.ok_or(ArrowError::SchemaError("missing header row".into()))?,
            records,
        })
    }

    // Read quilt4's Parquet format
    pub async fn read_from_path(path: &PathBuf) -> Result<Self, ArrowError> {
        let file = tokio::fs::File::open(&path).await?;
        Table::read_rows_impl(file).await
    }

    async fn write_row_impl<T>(
        writer: &mut AsyncArrowWriter<T>,
        schema: Arc<Schema>,
        row: &Row4,
    ) -> Result<(), ArrowError>
    where
        T: AsyncWrite + Unpin + Send,
    {
        let hash: &[u8] = &row.hash.to_bytes();
        let batch = RecordBatch::try_new(
            schema,
            vec![
                Arc::new(GenericByteArray::<Utf8Type>::from(vec![row.name.as_str()])),
                Arc::new(GenericByteArray::<Utf8Type>::from(vec![row.place.as_str()])),
                Arc::new(UInt64Array::from(vec![row.size])),
                Arc::new(GenericByteArray::<BinaryType>::from(vec![hash])),
                Arc::new(GenericByteArray::<Utf8Type>::from(vec![
                    serde_json::to_string(&row.meta).unwrap(),
                ])),
                Arc::new(GenericByteArray::<Utf8Type>::from(vec![
                    serde_json::to_string(&row.info).unwrap(),
                ])),
            ],
        )?;
        writer.write(&batch).await?;
        Ok(())
    }

    // Write quilt4's Parquet format
    pub async fn write_to_path(&self, path: &PathBuf) -> Result<(), ArrowError> {
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", DataType::Utf8, false),
            Field::new("place", DataType::Utf8, false),
            Field::new("size", DataType::UInt64, false),
            Field::new("multihash", DataType::Binary, false),
            Field::new("meta.json", DataType::Utf8, false),
            Field::new("info.json", DataType::Utf8, false),
        ]));

        let props = WriterProperties::builder()
            .set_compression(Compression::SNAPPY)
            .build();

        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).await?;
        }
        let file = tokio::fs::File::create(path).await?;
        let mut writer = AsyncArrowWriter::try_new(file, schema.clone(), Some(props)).unwrap();

        Table::write_row_impl(&mut writer, schema.clone(), &self.header).await?;
        for row in self.records.values() {
            Table::write_row_impl(&mut writer, schema.clone(), row).await?;
        }
        writer.close().await?;

        Ok(())
    }

    // Get a row from the table
    pub fn get_row(&self, name: &str) -> Option<&Row4> {
        self.records.get(name)
    }

    pub fn get_header(&self) -> &Row4 {
        &self.header
    }
    // TBD: Store header metadata as PARQUET Metadata?

    pub fn list_names(&self) -> Vec<Row4> {
        // Implementation goes here
        unimplemented!()
    }

    pub fn top_hash(&self) -> String {
        // TODO: Make sure floats are Python-compatible!
        let mut hasher = Sha256::new();

        let mut header_meta = match self.header.info.as_object() {
            Some(meta) => meta.clone(),
            None => serde_json::Map::default(),
        };
        if self.header.meta.is_object() {
            header_meta.insert("user_meta".into(), self.header.meta.clone());
        }

        let header_str = serde_json::to_string(&header_meta).unwrap();
        hasher.update(header_str);

        for row in self.records.values() {
            let mut row_meta = match row.info.as_object() {
                Some(meta) => meta.clone(),
                None => serde_json::Map::default(),
            };
            if row.meta.is_object() {
                row_meta.insert("user_meta".into(), row.meta.clone());
            }

            let content_hash: ContentHash = row.hash.try_into().unwrap();

            let value = serde_json::json!({
                "logical_key": row.name,
                "size": row.size,
                "hash": content_hash,
                "meta": row_meta,
            });

            let value_str = serde_json::to_string(&value).unwrap();
            hasher.update(value_str);
        }

        hex::encode(hasher.finalize())
    }
}

impl fmt::Display for Table {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut records = BTreeMap::new();
        for (name, record) in &self.records {
            records.insert(name, record.to_string());
        }
        write!(f, "Table({:?})", records)
    }
}

impl TryFrom<Manifest> for Table {
    type Error = Error;

    fn try_from(quilt3_manifest: Manifest) -> Result<Self, Self::Error> {
        let mut records = BTreeMap::new();
        for row in quilt3_manifest.rows.clone() {
            let mut info = row.meta.unwrap_or_default();
            let meta = info.remove("user_meta").unwrap_or_default();
            records.insert(
                row.logical_key.clone(),
                Row4 {
                    name: row.logical_key,
                    place: row.physical_key,
                    size: row.size,
                    hash: row.hash.try_into()?,
                    info: info.into(),
                    meta,
                },
            );
        }
        let header = Row4::from(quilt3_manifest);
        Ok(Table { header, records })
    }
}

#[cfg(test)]
mod tests {
    use crate::utils::local_uri_parquet;

    use super::*;

    #[tokio::test]
    async fn read_existing_local() {
        let table = Table::read_from_path(&local_uri_parquet()).await.unwrap();
        assert_eq!(table.records.len(), 2);

        let header = table.get_header();
        assert_eq!(header.size, 0);

        let readme = table.get_row("READ ME.md").unwrap();
        assert_eq!(readme.size, 33);
    }

    #[tokio::test]
    async fn read_write_local() {
        let table1 = Table::read_from_path(&local_uri_parquet()).await.unwrap();
        assert_eq!(table1.records.len(), 2);

        let temp_dir = temp_testdir::TempDir::default();
        let temp_path = temp_dir.join("test.parquet");

        table1.write_to_path(&temp_path).await.unwrap();

        let table2 = Table::read_from_path(&temp_path).await.unwrap();

        assert_eq!(table2.records.len(), 2);
        assert_eq!(table2.records, table1.records);
    }

    #[test]
    fn test_formatting_no_records() -> Result<(), multihash::Error> {
        let table = Table {
            header: Row4 {
                name: "Foo".to_string(),
                place: "Bar".to_string(),
                size: 123,
                hash: Multihash::wrap(345, b"hello world")?,
                info: serde_json::Value::Null,
                meta: serde_json::Value::Null,
            },
            records: BTreeMap::new(),
        };
        assert_eq!(table.to_string(), "Table({})".to_string());
        Ok(())
    }

    #[test]
    fn test_formatting_records() -> Result<(), multihash::Error> {
        let table = Table {
            header: Row4 {
                name: "Foo".to_string(),
                place: "Bar".to_string(),
                size: 123,
                hash: Multihash::wrap(345, b"hello world")?,
                info: serde_json::Value::Null,
                meta: serde_json::Value::Null,
            },
            records: BTreeMap::from([
                (
                    "one".to_string(),
                    Row4 {
                        name: "AA".to_string(),
                        place: "AB".to_string(),
                        size: 100,
                        hash: Multihash::wrap(100, b"A")?,
                        info: serde_json::Value::Null,
                        meta: serde_json::Value::Null,
                    },
                ),
                (
                    "two".to_string(),
                    Row4 {
                        name: "BA".to_string(),
                        place: "BB".to_string(),
                        size: 200,
                        hash: Multihash::wrap(200, b"B")?,
                        info: serde_json::Value::Null,
                        meta: serde_json::Value::Null,
                    },
                ),
            ]),
        };
        assert_eq!(table.to_string(), r##"Table({"one": "Row4(AA)@AB^100#[65]$$Null$Null", "two": "Row4(BA)@BB^200#[66]$$Null$Null"})"##.to_string());
        Ok(())
    }
}
