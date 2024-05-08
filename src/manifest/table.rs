//!
//! Table4 is a wrapper for arrow-rs's Table, the native Manifest format for quilt4.
//! It uses UPath to transparently read and write to/from local and remote filesystems,
//! and provides methods to read/write (decode/encode) quilt3's JSONL format
//!

use std::collections::btree_map;
use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;
use std::sync::Arc;
use tokio_stream::Stream;

use crate::checksum::ContentHash;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use arrow::array::GenericByteArray;
use arrow::array::UInt64Array;
use arrow::datatypes::BinaryType;
use arrow::datatypes::DataType;
use arrow::datatypes::Field;
use arrow::datatypes::Schema;
use arrow::datatypes::Utf8Type;
use arrow::error::ArrowError;
use multihash::Multihash;
use parquet::arrow::ParquetRecordBatchStreamBuilder;
use sha2::Digest;
use sha2::Sha256;
use tokio::io::AsyncRead;
use tokio::io::AsyncSeek;
use tokio_stream::StreamExt;

use crate::manifest::Row;
use crate::Error;

pub const HEADER_ROW: &str = ".";

fn serialize_table_header(header: &Row) -> serde_json::Map<String, serde_json::Value> {
    let mut header_meta = serde_json::Map::new();
    if let Some(message) = header.info.get("message") {
        header_meta.insert("message".to_string(), message.clone());
    }
    if header.meta.is_object() {
        header_meta.insert("user_meta".into(), header.meta.clone());
    }
    if let Some(version) = header.info.get("version") {
        header_meta.insert("version".to_string(), version.clone());
    }
    header_meta
}

#[derive(Debug)]
pub struct TopHasher {
    pub hasher: Box<Sha256>,
}

impl Default for TopHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl TopHasher {
    pub fn new() -> Self {
        TopHasher {
            hasher: Box::new(Sha256::new()),
        }
    }

    pub fn append(&mut self, row: &Row) -> Result<(), Error> {
        let value_str = if row.name.display().to_string() == HEADER_ROW {
            let value = serialize_table_header(row);
            serde_json::to_string(&value)?
        } else {
            let value = serialize_row_entry(row);
            serde_json::to_string(&value)?
        };
        self.hasher.update(value_str);
        Ok(())
    }

    pub fn finalize(self) -> String {
        hex::encode(self.hasher.finalize())
    }
}

// TODO: fix return type to Map<String, serde_json::Value>
fn serialize_row_entry(row: &Row) -> serde_json::Value {
    let mut row_meta = match row.info.as_object() {
        Some(meta) => meta.clone(),
        None => serde_json::Map::default(),
    };
    // TODO: correct order (alphabetical, as it in header)
    if row.meta.is_object() {
        row_meta.insert("user_meta".into(), row.meta.clone());
    }

    let content_hash: ContentHash = row.hash.try_into().unwrap();

    serde_json::json!({
        "hash": content_hash,
        "logical_key": row.name,
        "meta": row_meta,
        "size": row.size,
    })
}

#[derive(Clone, Debug, PartialEq)]
pub struct Table {
    pub header: Row,
    records: BTreeMap<PathBuf, Row>, // TODO: use PathBuf and iterator of records
    schema: Arc<Schema>,
}

impl Table {
    // TODO: new creates empty records, from(header, records) creates full Table
    pub fn new(header: Row, records: BTreeMap<PathBuf, Row>) -> Self {
        let schema = Arc::new(Schema::new(vec![
            Field::new("name", DataType::Utf8, false),
            Field::new("place", DataType::Utf8, false),
            Field::new("size", DataType::UInt64, false),
            Field::new("multihash", DataType::Binary, false),
            Field::new("meta.json", DataType::Utf8, false),
            Field::new("info.json", DataType::Utf8, false),
        ]));
        Table {
            header,
            records,
            schema,
        }
    }

    async fn read_rows_impl<T>(reader: T) -> Result<Self, Error>
    where
        T: AsyncSeek + AsyncRead + Unpin + Send + 'static,
    {
        let mut stream = ParquetRecordBatchStreamBuilder::new(reader)
            .await?
            .build()?;

        let mut header: Option<Row> = None;
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

                let row = Row {
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

        Ok(Table::new(
            header.ok_or(ArrowError::SchemaError("missing header row".into()))?,
            records,
        ))
    }

    // Read quilt4's Parquet format
    pub async fn read_from_path(
        storage: &impl Storage,
        path: impl AsRef<Path>,
    ) -> Result<Self, Error> {
        let file = storage.open_file(path.as_ref()).await?;
        Table::read_rows_impl(file).await
    }

    // Get a row from the table
    pub fn get_row(&self, name: &PathBuf) -> Option<&Row> {
        self.records.get(name)
    }

    pub async fn get_header(&self) -> Result<Row, Error> {
        Ok(self.header.clone())
    }

    pub fn remove_record(&mut self, path: &PathBuf) -> Result<Row, Error> {
        self.records
            .remove(path)
            .ok_or(Error::Table(format!("Cannot remove {:?}", path)))
    }

    pub async fn records_len(&self) -> usize {
        // TOdO: when records is unavailable, read first column and `column.values().len()`
        self.records.len()
    }

    pub fn records_values(&self) -> btree_map::Values<'_, PathBuf, Row> {
        // TODO: when records is unavailable, read first column and `column.values().len()`
        self.records.values()
    }

    pub async fn records_stream(&self) -> impl Stream<Item = Row> {
        let entries: Vec<Row> = self.records_values().cloned().collect();
        tokio_stream::iter(entries)
    }

    pub async fn insert_record(&mut self, row: Row) -> Result<Option<Row>, Error> {
        Ok(self.records.insert(row.name.clone(), row))
    }

    pub async fn get_record(&self, path: &PathBuf) -> Result<Option<Row>, Error> {
        Ok(self.records.get(path).cloned())
    }

    pub async fn update_record(&mut self, row: Row) -> Result<Option<Row>, Error> {
        Ok(self.records.insert(row.name.clone(), row))
    }

    pub async fn contains_record(&self, path: &PathBuf) -> bool {
        self.records.contains_key(path)
    }

    #[cfg(test)]
    pub fn set_records(&mut self, records: BTreeMap<PathBuf, Row>) {
        self.records = records;
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
impl Default for Table {
    fn default() -> Self {
        Table::new(Row::default(), BTreeMap::default())
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
                Row {
                    name: row.logical_key,
                    place: row.physical_key,
                    size: row.size,
                    hash: row.hash.try_into()?,
                    info: info.into(),
                    meta,
                },
            );
        }
        let header = Row::from(quilt3_manifest);
        Ok(Table::new(header, records))
    }
}

#[cfg(test)]
mod tests {
    use crate::mocks;

    use super::*;

    #[tokio::test]
    async fn read_existing_local() -> Result<(), Error> {
        let storage = mocks::storage::MockStorage::default();
        storage
            .write_file(
                mocks::manifest::parquet(),
                &std::fs::read(mocks::manifest::parquet())?,
            )
            .await?;
        let table = Table::read_from_path(&storage, &mocks::manifest::parquet())
            .await
            .unwrap();
        assert_eq!(table.records_len().await, 2);

        let header = table.get_header().await?;
        assert_eq!(header.size, 0);

        let readme = table.get_row(&PathBuf::from("READ ME.md")).unwrap();
        assert_eq!(readme.size, 33);

        Ok(())
    }

    // #[tokio::test]
    // #[ignore]
    // async fn read_write_local() {
    //     let storage = mocks::storage::MockStorage::default();
    //     let table1 = Table::read_from_path(&storage, &mocks::manifest::parquet())
    //         .await
    //         .unwrap();
    //     assert_eq!(table1.records_len().await, 2);

    //     let temp_dir = temp_testdir::TempDir::default();
    //     let temp_path = temp_dir.join("test.parquet");

    //     table1.write_to_path(&storage, &temp_path).await.unwrap();

    //     let table2 = Table::read_from_path(&storage, &temp_path).await.unwrap();

    //     assert_eq!(table2.records_len().await, 2);
    //     assert_eq!(table2.records, table1.records);
    // }

    #[test]
    fn test_formatting_no_records() -> Result<(), multihash::Error> {
        let table = Table::new(
            Row {
                name: PathBuf::from("Foo"),
                place: "Bar".to_string(),
                size: 123,
                hash: Multihash::wrap(345, b"hello world")?,
                info: serde_json::Value::Null,
                meta: serde_json::Value::Null,
            },
            BTreeMap::new(),
        );
        assert_eq!(table.to_string(), "Table({})".to_string());
        Ok(())
    }

    #[test]
    fn test_formatting_records() -> Result<(), multihash::Error> {
        let table = Table::new(
            Row {
                name: PathBuf::from("Foo"),
                place: "Bar".to_string(),
                size: 123,
                hash: Multihash::wrap(345, b"hello world")?,
                info: serde_json::Value::Null,
                meta: serde_json::Value::Null,
            },
            BTreeMap::from([
                (
                    PathBuf::from("one"),
                    Row {
                        name: PathBuf::from("AA"),
                        place: "AB".to_string(),
                        size: 100,
                        hash: Multihash::wrap(100, b"A")?,
                        info: serde_json::Value::Null,
                        meta: serde_json::Value::Null,
                    },
                ),
                (
                    PathBuf::from("two"),
                    Row {
                        name: PathBuf::from("BA"),
                        place: "BB".to_string(),
                        size: 200,
                        hash: Multihash::wrap(200, b"B")?,
                        info: serde_json::Value::Null,
                        meta: serde_json::Value::Null,
                    },
                ),
            ]),
        );
        assert_eq!(table.to_string(), r##"Table({"one": "Row(AA)@AB^100#[65]$$Null$Null", "two": "Row(BA)@BB^200#[66]$$Null$Null"})"##.to_string());
        Ok(())
    }

    // #[test]
    // fn test_top_hash() -> Result<(), Error> {
    //     let manifest = Table::new(
    //         Row {
    //             meta: serde_json::json!({
    //                    "1234567890": "a",
    //             }),
    //             info: serde_json::json!({
    //                    "message": "Second revision",
    //                    "version": "v0",
    //             }),
    //             ..Row::default()
    //         },
    //         BTreeMap::from([(
    //             PathBuf::from("test.md"),
    //             Row {
    //                 name: PathBuf::from("test.md"),
    //                 place: "doesn't matter".to_string(),
    //                 size: 3568,
    //                 hash: ContentHash::SHA256Chunked(
    //                     "MhntcZnyIL1AIPJNNh8LwzB68M5lFBW0pTEMFTeOSJo=".to_string(),
    //                 )
    //                 .try_into()?,
    //                 info: serde_json::Value::Null,
    //                 meta: serde_json::Value::Null,
    //             },
    //         )]),
    //     );
    //     assert_eq!(
    //         manifest.top_hash(),
    //         "83571a1d923f1ff9a965855030e85a5bac89b4b5af45d7f920b80e89343eca1f".to_string()
    //     );
    //     Ok(())
    // }
}
