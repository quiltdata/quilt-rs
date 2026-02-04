//!
//! Table4 is a wrapper for arrow-rs's Table, the native Manifest format for quilt4.
//! It uses UPath to transparently read and write to/from local and remote filesystems,
//! and provides methods to read/write (decode/encode) quilt3's JSONL format
//!

use std::collections::BTreeMap;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;

use arrow::array::GenericByteArray;
use arrow::array::UInt64Array;
use arrow::datatypes::BinaryType;
use arrow::datatypes::Utf8Type;
use arrow::error::ArrowError;
use multihash::Multihash;
use parquet::arrow::ParquetRecordBatchStreamBuilder;
use tokio::io::AsyncRead;
use tokio::io::AsyncSeek;
use tokio_stream::StreamExt;

use crate::io::manifest::RowsStream;
use crate::io::manifest::StreamRowsChunk;
use crate::io::storage::Storage;
use crate::manifest::Header;
use crate::manifest::Manifest;
#[cfg(test)]
use crate::manifest::ManifestHeader;
#[cfg(test)]
use crate::manifest::ManifestRow;
#[cfg(test)]
use crate::manifest::TopHasher;
#[cfg(test)]
use crate::manifest::top_hasher::serialize_manifest_row_entry;
use crate::manifest::Row;
use crate::manifest::RowDisplay;
use crate::Error;
use crate::Res;


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

    /// Convert from Manifest to Table format
    pub fn from_manifest(manifest: &Manifest) -> Res<Self> {
        let header = Header::try_from(&manifest.header)?;

        let records = manifest
            .rows
            .iter()
            .map(|row| (row.logical_key.clone(), Row::from(row.clone())))
            .collect::<BTreeMap<PathBuf, Row>>();

        Ok(Table::new(header, records))
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
            .ok_or(Error::Table(format!("Cannot remove {path:?}")))
    }

    pub async fn records_len(&self) -> usize {
        // TOdO: when records is unavailable, read first column and `column.values().len()`
        self.records.len()
    }

    pub async fn records_stream(&self) -> impl RowsStream {
        let entries: StreamRowsChunk = self
            .records
            .values()
            .cloned()
            .map(|row| {
                row.try_into()
                    .map_err(|e: Error| Error::Table(e.to_string()))
            })
            .collect();
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
    use serde_json::json;
    use test_log::test;

    use crate::checksum::Sha256ChunkedHash;
    use crate::checksum::MULTIHASH_SHA256;
    use crate::fixtures;
    use crate::io::storage::mocks::MockStorage;

    #[test(tokio::test)]
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

        let readme = table
            .get_record(&PathBuf::from("READ ME.md"))
            .await?
            .unwrap();
        assert_eq!(readme.size, 33);

        Ok(())
    }

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
    fn test_serialize_manifest_row_entry_with_info() -> Res {
        let hash = Multihash::<256>::wrap(MULTIHASH_SHA256, b"test")?;
        let manifest_row = ManifestRow {
            logical_key: PathBuf::from("test.txt"),
            physical_key: "s3://test-bucket/test.txt".to_string(),
            size: 42,
            hash: hash.try_into()?,
            meta: Some(json!({"user_meta": {"foo": "bar"}})),
        };

        let serialized = serialize_manifest_row_entry(&manifest_row)?;

        assert_eq!(
            serialized,
            serde_json::Map::from_iter([
                (
                    "hash".to_string(),
                    json!({"type": "SHA256", "value": hex::encode(hash.digest())}),
                ),
                ("logical_key".to_string(), json!("test.txt")),
                ("meta".to_string(), json!({"user_meta": {"foo": "bar"}})),
                ("size".to_string(), json!(42)),
            ])
        );
        Ok(())
    }

    #[test(tokio::test)]
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

    #[test(tokio::test)]
    async fn test_top_hash() -> Res {
        // Test with the old Row-based approach for backward compatibility

        let manifest_header = ManifestHeader {
            version: "v0".to_string(),
            message: Some("Second revision".to_string()),
            user_meta: Some(json!({"1234567890": "a"})),
            workflow: None,
        };

        let manifest_row = ManifestRow {
            logical_key: PathBuf::from("test.md"),
            physical_key: "doesn't matter".to_string(),
            size: 3568,
            hash: Sha256ChunkedHash::try_from("MhntcZnyIL1AIPJNNh8LwzB68M5lFBW0pTEMFTeOSJo=")?
                .into(),
            meta: None, // This should be treated as {} per quilt3 behavior
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&manifest_header)?;
        top_hasher.append(&manifest_row)?;

        assert_eq!(
            top_hasher.finalize(),
            "83571a1d923f1ff9a965855030e85a5bac89b4b5af45d7f920b80e89343eca1f".to_string()
        );
        Ok(())
    }
}
