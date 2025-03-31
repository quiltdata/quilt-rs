//!
//! Table4 is a wrapper for arrow-rs's Table, the native Manifest format for quilt4.
//! It uses UPath to transparently read and write to/from local and remote filesystems,
//! and provides methods to read/write (decode/encode) quilt3's JSONL format
//!

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;

use crate::checksum::ContentHash;
use crate::io::storage::Storage;
use arrow::array::GenericByteArray;
use arrow::array::UInt64Array;
use arrow::datatypes::BinaryType;
use arrow::datatypes::Utf8Type;
use arrow::error::ArrowError;
use multihash::Multihash;
use parquet::arrow::ParquetRecordBatchStreamBuilder;
use sha2::Digest;
use sha2::Sha256;
use tokio::io::AsyncRead;
use tokio::io::AsyncSeek;
use tokio_stream::StreamExt;

use crate::io::manifest::RowsStream;
use crate::io::manifest::StreamRowsChunk;
use crate::manifest::Header;
use crate::manifest::Row;
use crate::manifest::RowDisplay;
use crate::Error;
use crate::Res;

fn serialize_table_header(header: &Header) -> Res<serde_json::Map<String, serde_json::Value>> {
    let mut header_meta = serde_json::Map::new();
    if let Some(message) = header.get_message()? {
        header_meta.insert("message".to_string(), serde_json::to_value(message)?);
    } else {
        header_meta.insert("message".to_string(), serde_json::Value::Null);
    }
    if let Some(user_meta) = header.get_user_meta()? {
        let u = match user_meta {
            serde_json::Value::Object(mut m) => {
                m.values_mut().for_each(serde_json::Value::sort_all_objects);
                m.sort_keys();
                serde_json::Value::Object(m)
            }
            _ => user_meta,
        };
        header_meta.insert("user_meta".into(), u);
    }
    header_meta.insert(
        "version".to_string(),
        serde_json::Value::String(header.get_version()?),
    );
    if let Some(workflow) = header.get_workflow()? {
        header_meta.insert("workflow".to_string(), serde_json::to_value(&workflow)?);
    }
    Ok(header_meta)
}

/// Helper for creating `top_hash`
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

    /// Append `Header` to the hasher
    pub fn append_header(&mut self, header: &Header) -> Res {
        let value = serialize_table_header(header)?;
        let value_str = serde_json::to_string(&value)?;
        self.hasher.update(value_str);
        Ok(())
    }

    /// Append `Row` to the hasher
    pub fn append(&mut self, row: &Row) -> Res {
        let value = serialize_row_entry(row);
        let value_str = serde_json::to_string(&value)?;
        self.hasher.update(value_str);
        Ok(())
    }

    /// Consume `self` and return `top_hash`
    pub fn finalize(self) -> String {
        hex::encode(self.hasher.finalize())
    }
}

// TODO: fix return type to Map<String, serde_json::Value>
fn serialize_row_entry(row: &Row) -> serde_json::Value {
    let mut meta = match row.info.as_object() {
        Some(meta) => meta.clone(),
        None => serde_json::Map::default(),
    };
    if let Some(m) = &row.meta {
        meta.insert("user_meta".into(), m.clone());
    }

    let content_hash: ContentHash = row.hash.try_into().unwrap();

    serde_json::json!({
        "hash": content_hash,
        "logical_key": row.name,
        "meta": meta,
        "size": row.size,
    })
}

/// Helper for reading Parquet manifest and get `Row`s
// TODO: use PathBuf and iterator of records,
// don't store records in memory
#[derive(Clone, Debug)]
pub struct Table {
    pub header: Header,
    // path: PathBuf, // TODO
    records: BTreeMap<PathBuf, Row>,
}

impl Table {
    // TODO: new creates empty records, from(header, records) creates full Table
    pub fn new(header: Header, records: BTreeMap<PathBuf, Row>) -> Self {
        Table { header, records }
    }

    async fn read_rows_impl<T>(reader: T) -> Res<Self>
    where
        T: AsyncSeek + AsyncRead + Unpin + Send + 'static,
    {
        let mut stream = ParquetRecordBatchStreamBuilder::new(reader)
            .await?
            .build()?;

        let mut header: Option<Header> = None;
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
                if idx == 0 {
                    header = Some(Header {
                        info: serde_json::from_str(info_column.value(idx))
                            .map_err(|err| ArrowError::SchemaError(err.to_string()))?,
                        meta: serde_json::from_str(meta_column.value(idx))
                            .map_err(|err| ArrowError::SchemaError(err.to_string()))?,
                    });
                } else {
                    let name = name_column.value(idx);
                    let hash = Multihash::from_bytes(multihash_column.value(idx))
                        .map_err(|err| ArrowError::SchemaError(err.to_string()))?;

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
    pub async fn read_from_path(storage: &impl Storage, path: impl AsRef<Path>) -> Res<Self> {
        let file = storage.open_file(path.as_ref()).await?;
        Table::read_rows_impl(file).await
    }

    pub async fn get_header(&self) -> Res<Header> {
        Ok(self.header.clone())
    }

    // TODO: make async
    pub fn remove_record(&mut self, path: &PathBuf) -> Res<Row> {
        self.records
            .remove(path)
            .ok_or(Error::Table(format!("Cannot remove {:?}", path)))
    }

    pub async fn records_len(&self) -> usize {
        // TOdO: when records is unavailable, read first column and `column.values().len()`
        self.records.len()
    }

    pub async fn records_stream(&self) -> impl RowsStream {
        let entries: StreamRowsChunk = self.records.values().cloned().map(Ok).collect();
        tokio_stream::iter(vec![Ok(entries)])
    }

    pub async fn insert_record(&mut self, row: Row) -> Res<Option<Row>> {
        Ok(self.records.insert(row.name.clone(), row))
    }

    /// Get a row from the table
    pub async fn get_record(&self, path: &PathBuf) -> Res<Option<Row>> {
        Ok(self.records.get(path).cloned())
    }

    pub async fn update_record(&mut self, row: Row) -> Res<Option<Row>> {
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
        if self.records.is_empty() {
            return write!(f, "Table: empty");
        }

        let mut records = Vec::new();
        for record in self.records.values() {
            records.push(RowDisplay::from(record));
        }
        write!(f, "Table:\n{}", tabled::Table::new(records))
    }
}

impl Default for Table {
    fn default() -> Self {
        Table::new(Header::default(), BTreeMap::default())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use multihash::Multihash;

    use crate::checksum::MULTIHASH_SHA256;
    use crate::fixtures;
    use crate::io::storage::mocks::MockStorage;
    use crate::manifest::Row;

    #[tokio::test]
    async fn read_existing_local() -> Res {
        let storage = MockStorage::default();
        storage
            .write_file(
                fixtures::manifest::parquet()?,
                &std::fs::read(fixtures::manifest::parquet()?)?,
            )
            .await?;
        let table = Table::read_from_path(&storage, &fixtures::manifest::parquet()?)
            .await
            .unwrap();
        assert_eq!(table.records_len().await, 2);

        // let header = table.get_header().await?;
        // assert_eq!(header.size, 0);

        let readme = table
            .get_record(&PathBuf::from("READ ME.md"))
            .await?
            .unwrap();
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
    fn test_formatting_no_records() -> Res {
        let table = Table::new(Header::default(), BTreeMap::new());
        assert_eq!(table.to_string(), "Table: empty".to_string());
        Ok(())
    }

    #[test]
    fn test_formatting_records() -> Res {
        let table = Table::new(
            Header::default(),
            BTreeMap::from([
                (
                    PathBuf::from("one"),
                    Row {
                        name: PathBuf::from("AA"),
                        place: "AB".to_string(),
                        size: 100,
                        hash: Multihash::wrap(100, b"A")?,
                        info: serde_json::Value::Null,
                        meta: None,
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
                        meta: None,
                    },
                ),
            ]),
        );
        assert_eq!(
            table.to_string(),
            r###"Table:
+------+-------+------+-------------+----------+------+------+
| name | place | size | hash_base64 | hash_hex | info | meta |
+------+-------+------+-------------+----------+------+------+
| AA   | AB    | 100  | QQ==        | 640141   | null | null |
+------+-------+------+-------------+----------+------+------+
| BA   | BB    | 200  | Qg==        | c8010142 | null | null |
+------+-------+------+-------------+----------+------+------+"###
        );

        // assert_eq!(table.to_string(), r##"Table({"one": "Row(AA)@AB^100#[65]$$Null$Null", "two": "Row(BA)@BB^200#[66]$$Null$Null"})"##.to_string());
        Ok(())
    }

    #[test]
    fn test_serialize_row_entry_with_info() -> Res {
        let hash = Multihash::<256>::wrap(MULTIHASH_SHA256, b"test")?;
        let row = Row {
            name: PathBuf::from("test.txt"),
            place: "s3://test-bucket/test.txt".to_string(),
            size: 42,
            hash,
            info: serde_json::Value::Null,
            meta: Some(serde_json::json!({"foo": "bar"})),
        };

        let serialized = serialize_row_entry(&row);
        assert_eq!(
            serialized,
            serde_json::json!({
                "hash": {"type": "SHA256", "value": hex::encode(hash.digest())},
                "logical_key": "test.txt",
                "meta": {"user_meta": {"foo": "bar"}},
                "size": 42,
            })
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_table_record_operations() -> Res {
        let mut table = Table::default();
        let path = PathBuf::from("foo/bar");

        // 1. Check empty table doesn't contain record
        assert!(!table.contains_record(&path).await);

        // 2. Insert record and verify it exists
        let row = Row {
            name: path.clone(),
            place: "s3://test-bucket/foo/bar".to_string(),
            size: 42,
            hash: Multihash::wrap(0, b"test")?,
            info: serde_json::Value::Null,
            meta: None,
        };
        table.insert_record(row.clone()).await?;
        assert!(table.contains_record(&path).await);

        // 3. Update record
        let updated_row = Row {
            size: 84,
            ..row.clone()
        };
        let old_row = table.update_record(updated_row.clone()).await?;
        assert_eq!(old_row, Some(row));

        // 4. Remove record and verify state
        let removed_row = table.remove_record(&path)?;
        assert_eq!(removed_row, updated_row);
        assert!(!table.contains_record(&path).await);
        assert_eq!(table.records_len().await, 0);

        Ok(())
    }

    #[tokio::test]
    async fn test_top_hash() -> Res {
        let manifest = Table::new(
            Header {
                meta: Some(serde_json::json!({
                       "1234567890": "a",
                })),
                info: serde_json::json!({
                       "message": "Second revision",
                       "version": "v0",
                }),
            },
            BTreeMap::from([(
                PathBuf::from("test.md"),
                Row {
                    name: PathBuf::from("test.md"),
                    place: "doesn't matter".to_string(),
                    size: 3568,
                    hash: ContentHash::SHA256Chunked(
                        "MhntcZnyIL1AIPJNNh8LwzB68M5lFBW0pTEMFTeOSJo=".to_string(),
                    )
                    .try_into()?,
                    info: serde_json::Value::Null,
                    meta: None,
                },
            )]),
        );

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&manifest.header)?;
        let path = PathBuf::from("test.md");
        top_hasher.append(&manifest.get_record(&path).await?.unwrap())?;

        assert_eq!(
            top_hasher.finalize(),
            "83571a1d923f1ff9a965855030e85a5bac89b4b5af45d7f920b80e89343eca1f".to_string()
        );
        Ok(())
    }
}
