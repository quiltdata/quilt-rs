use std::path::PathBuf;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncRead;
use tokio::io::BufReader;
use tokio_stream::StreamExt;

use crate::checksum::ContentHash;
use crate::manifest::Header;
use crate::manifest::Table;
use crate::Error;
use crate::Res;

pub type JsonObject = serde_json::Map<String, serde_json::Value>;

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize, Clone)]
pub struct Workflow {
    pub config: String,
    pub id: Option<String>,
}

/// Header (or first row) in JSONL manifest
#[derive(Debug, Deserialize, PartialEq, Eq, Serialize, Clone)]
pub struct ManifestHeader {
    pub version: String,
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] // Attempt to be quilt3-compatible.
    pub user_meta: Option<JsonObject>,
    pub workflow: Option<Workflow>,
}

impl Default for ManifestHeader {
    fn default() -> Self {
        Header::default().into()
    }
}

impl From<&Header> for ManifestHeader {
    fn from(header: &Header) -> Self {
        ManifestHeader {
            version: "v0".into(),
            message: header.display_message(),
            user_meta: header.display_user_meta(),
            workflow: header.display_workflow(),
        }
    }
}

impl From<Header> for ManifestHeader {
    fn from(header: Header) -> Self {
        ManifestHeader::from(&header)
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct Quilt3ManifestRow {
    pub logical_key: PathBuf,
    pub physical_keys: Vec<String>,
    pub hash: ContentHash,
    // XXX: u64 cannot be safely deserialized by standard JS json parser,
    //      which treats numbers as 64-bit floats.
    //      However, having file size more than ~9PB (max safe/lossless integer - 53 bits)
    //      is quite unlikely ATM, given S3 limitations of 5TB per object.
    #[serde(deserialize_with = "number_to_u64")]
    pub size: u64,
    pub meta: Option<JsonObject>,
}

fn number_to_u64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    f64::deserialize(deserializer).map(|n| n as u64)
}

impl From<ManifestRow> for Quilt3ManifestRow {
    fn from(row: ManifestRow) -> Self {
        Self {
            logical_key: row.logical_key,
            physical_keys: vec![row.physical_key],
            hash: row.hash,
            size: row.size,
            meta: row.meta,
        }
    }
}

/// Represents the row in JSONL manifest
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ManifestRow {
    pub logical_key: PathBuf,
    // XXX: use Url to have validated string?
    pub physical_key: String,
    pub hash: ContentHash,
    pub size: u64,
    pub meta: Option<JsonObject>,
}

impl std::cmp::PartialEq for ManifestRow {
    // TODO: add note why we don't compare meta and physical_key
    fn eq(&self, other: &Self) -> bool {
        self.logical_key == other.logical_key && self.hash == other.hash && self.size == other.size
    }
}

impl TryFrom<Quilt3ManifestRow> for ManifestRow {
    type Error = Error;

    fn try_from(row: Quilt3ManifestRow) -> Result<Self, Self::Error> {
        Ok(ManifestRow {
            logical_key: row.logical_key,
            physical_key: row
                .physical_keys
                .into_iter()
                .next()
                .ok_or(Error::ManifestHeader("Physical key is missing".to_string()))?,
            hash: row.hash,
            size: row.size,
            meta: row.meta,
        })
    }
}

/// Legacy JSONL in-memory manifest
#[derive(Debug, Deserialize, PartialEq, Serialize, Clone)]
pub struct Manifest {
    pub header: ManifestHeader,
    pub rows: Vec<ManifestRow>,
    // XXX: iterators?
}

impl Manifest {
    pub async fn from_reader<F: AsyncRead + Unpin + Send>(file: F) -> Res<Self> {
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        let header = lines.next_line().await.map_err(|err| {
            Error::ManifestHeader(format!("Failed to read the manifest header: {}", err))
        })?;

        let Some(header) = header else {
            return Err(Error::ManifestHeader("Empty manifest".into()));
        };

        let header: ManifestHeader = serde_json::from_str(&header)?;

        if header.version != "v0" {
            return Err(Error::ManifestHeader(format!(
                "Unsupported manifest version: {}",
                header.version
            )));
        }

        let mut rows = Vec::new();

        while let Some(line) = lines.next_line().await? {
            let row: Quilt3ManifestRow = serde_json::from_str(&line)?;
            rows.push(ManifestRow::try_from(row)?);
        }

        Ok(Manifest { header, rows })
    }

    pub fn to_jsonlines(&self) -> String {
        // TODO: This is slightly inefficient.
        // We could use some kind of async iterator / stream idk
        let mut buf = String::new();
        buf.push_str(
            serde_json::to_string(&self.header)
                .expect("Could not serialize manifest header")
                .as_str(),
        );
        buf.push('\n');

        for row in &self.rows {
            let q3row = Quilt3ManifestRow::from(row.to_owned());
            buf.push_str(
                serde_json::to_string(&q3row)
                    .expect("Could not serialize manifest row")
                    .as_str(),
            );
            buf.push('\n');
        }
        buf
    }

    // pub fn find_path(&self, path: impl AsRef<str>) -> Option<usize> {
    //     let path = path.as_ref();
    //     self.rows
    //         .binary_search_by(|row| row.logical_key.cmp(path))
    //         .ok()
    // }

    // pub fn get(&self, path: impl AsRef<str>) -> Option<&ManifestRow> {
    //     let idx = self.find_path(path)?;
    //     Some(&self.rows[idx])
    // }

    // pub fn get_mut(&mut self, path: impl AsRef<str>) -> Option<&mut ManifestRow> {
    //     let idx = self.find_path(path)?;
    //     Some(&mut self.rows[idx])
    // }

    // pub fn has_path(&self, path: impl AsRef<str>) -> bool {
    //     // TODO: handle directories
    //     self.find_path(path).is_some()
    // }

    // pub fn rows_map(&self) -> BTreeMap<String, ManifestRow> {
    //     self.rows
    //         .iter()
    //         .map(|row| (row.logical_key.clone(), row.to_owned()))
    //         .collect()
    // }

    pub async fn from_table(table: &Table) -> Res<Self> {
        let mut manifest_rows = Vec::new();
        let mut stream = table.records_stream().await;
        while let Some(rows) = stream.next().await {
            for row in rows? {
                let row = row?;
                let mut meta = match row.info.as_object() {
                    Some(meta) => meta.clone(),
                    None => serde_json::Map::default(),
                };
                if row.meta.is_object() {
                    meta.insert("user_meta".into(), row.meta.clone());
                }
                manifest_rows.push(ManifestRow {
                    logical_key: row.name.clone(),
                    physical_key: row.place.clone(),
                    hash: row.hash.try_into().unwrap(),
                    size: row.size,
                    meta: Some(meta),
                })
            }
        }
        Ok(Manifest {
            header: (&table.header).into(),
            rows: manifest_rows,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use multihash::Multihash;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use crate::checksum::MULTIHASH_SHA256;
    use crate::io::storage::mocks::MockStorage;
    use crate::io::storage::LocalStorage;
    use crate::io::storage::Storage;
    use crate::manifest::Row;
    use crate::manifest::Table;
    use crate::mocks;

    #[test]
    fn test_equality_of_strictly_equal() {
        let left = ManifestRow {
            logical_key: PathBuf::from("A"),
            physical_key: "B".to_string(),
            hash: ContentHash::SHA256("C".to_string()),
            size: 1,
            meta: None,
        };
        let right = ManifestRow {
            logical_key: PathBuf::from("A"),
            physical_key: "B".to_string(),
            hash: ContentHash::SHA256("C".to_string()),
            size: 1,
            meta: None,
        };
        assert!(left == right)
    }

    #[test]
    fn test_equality_of_partialy_equal() {
        let mut meta = serde_json::Map::new();
        meta.insert("foo".to_string(), serde_json::json!("bar"));
        let left = ManifestRow {
            logical_key: PathBuf::from("A"),
            physical_key: "FOO".to_string(),
            hash: ContentHash::SHA256("C".to_string()),
            size: 1,
            meta: Some(meta),
        };
        let right = ManifestRow {
            logical_key: PathBuf::from("A"),
            physical_key: "BAR".to_string(),
            hash: ContentHash::SHA256("C".to_string()),
            size: 1,
            meta: None,
        };
        assert!(left == right)
    }

    #[tokio::test]
    async fn test_manifest_from_reader_invalid() -> Res {
        let storage = MockStorage::default();
        let invalid_content = r#"{"invalid": "json"}"#;
        let path = PathBuf::from("invalid_manifest.jsonl");
        storage
            .write_file(&path, invalid_content.as_bytes())
            .await?;
        let file = storage.open_file(&path).await?;

        let result = Manifest::from_reader(file).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("missing field `version`"));
        Ok(())
    }

    #[tokio::test]
    async fn test_manifest_from_reader_empty() -> Res {
        let storage = MockStorage::default();
        let path = PathBuf::from("empty_manifest.jsonl");
        storage.write_file(&path, b"").await?;
        let file = storage.open_file(&path).await?;

        let result = Manifest::from_reader(file).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Manifest header: Empty manifest"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_manifest_from_reader_invalid_utf8() -> Res {
        let storage = MockStorage::default();
        let invalid_content = b"\xFF\xFF\xFF\xFF"; // Invalid UTF-8 bytes
        let path = PathBuf::from("invalid_utf8_manifest.jsonl");
        storage.write_file(&path, invalid_content).await?;
        let file = storage.open_file(&path).await?;

        let result = Manifest::from_reader(file).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to read the manifest header"));
        Ok(())
    }

    #[tokio::test]
    async fn test_manifest_from_reader_unsupported_version() -> Res {
        let storage = MockStorage::default();
        let invalid_content = r#"{"version": "v1"}"#;
        let path = PathBuf::from("unsupported_version_manifest.jsonl");
        storage
            .write_file(&path, invalid_content.as_bytes())
            .await?;
        let file = storage.open_file(&path).await?;

        let result = Manifest::from_reader(file).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Manifest header: Unsupported manifest version: v1"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_manifest_from_reader_invalid_row() -> Res {
        let storage = MockStorage::default();
        let invalid_content = r#"{"version": "v0"}
{"invalid": "row"}"#;
        let path = PathBuf::from("invalid_row_manifest.jsonl");
        storage
            .write_file(&path, invalid_content.as_bytes())
            .await?;
        let file = storage.open_file(&path).await?;

        let result = Manifest::from_reader(file).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("missing field `logical_key`"));
        Ok(())
    }

    #[tokio::test]
    async fn test_manifest_from_reader_empty_physical_keys() -> Res {
        let storage = MockStorage::default();
        let invalid_content = r#"{"version": "v0"}
{"logical_key": "test.txt", "physical_keys": [], "size": 0, "hash": {"type": "SHA256", "value": "abc123"}, "meta": {}}"#;
        let path = PathBuf::from("empty_physical_keys_manifest.jsonl");
        storage
            .write_file(&path, invalid_content.as_bytes())
            .await?;
        let file = storage.open_file(&path).await?;

        let result = Manifest::from_reader(file).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Manifest header: Physical key is missing"
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_manifest_from_reader_valid() -> Res {
        let storage = LocalStorage::default();
        let file = storage.open_file(mocks::manifest::jsonl()).await?;

        assert_eq!(
            Manifest::from_reader(file).await?,
            Manifest {
                header: ManifestHeader {
                    version: "v0".to_string(),
                    message: None,
                    user_meta: None,
                    workflow: None,
                },
                rows: vec![
                    ManifestRow {
                        logical_key: PathBuf::from("README.md"),
                        physical_key: "s3://udp-spec/test_run/test_push/README.md?versionId=Rv.GfYdUWkLfeTT73Rodm3aBUrTIcC1X".to_string(),
                        size: 26,
                        hash: ContentHash::SHA256("bc2f10e72e751ea6cc1e0b9bdbbb531d437ccbba684b9fef90e1cc228318e112".to_string()),
                        meta: Some(serde_json::Map::new()),
                    }
                ],
            }
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_manifest_from_table_with_metadata() -> Res {
        let hash = Multihash::<256>::wrap(MULTIHASH_SHA256, b"test")?;
        let mut table = Table::default();
        table.set_records(BTreeMap::from([(
            PathBuf::from("test.txt"),
            Row {
                name: PathBuf::from("test.txt"),
                place: "s3://test-bucket/test.txt".to_string(),
                size: 42,
                hash,
                info: serde_json::json!({"foo": "bar"}),
                meta: serde_json::json!({"baz": "qux"}),
            },
        )]));
        let manifest = Manifest::from_table(&table).await?;

        assert_eq!(
            manifest,
            Manifest {
                header: ManifestHeader {
                    version: "v0".to_string(),
                    message: Some("".to_string()),
                    user_meta: None,
                    workflow: None,
                },
                rows: vec![ManifestRow {
                    logical_key: PathBuf::from("test.txt"),
                    physical_key: "s3://test-bucket/test.txt".to_string(),
                    size: 42,
                    hash: ContentHash::try_from(hash)?,
                    meta: Some(serde_json::Map::from_iter(vec![
                        ("user_meta".to_string(), serde_json::json!({"baz": "qux"})),
                        ("foo".to_string(), serde_json::json!("bar")),
                    ])),
                }],
            }
        );
        Ok(())
    }

    #[test]
    fn test_manifest_header_from_header() {
        let header = Header {
            info: serde_json::json!({
                "message": "test message",
                "version": "v0",
            }),
            meta: serde_json::json!({"user": "meta"}),
        };

        assert_eq!(
            ManifestHeader::from(header),
            ManifestHeader {
                version: "v0".to_string(),
                message: Some("test message".to_string()),
                user_meta: Some(serde_json::Map::from_iter(vec![
                    ("user".to_string(), serde_json::json!("meta")),
                ])),
                workflow: None,
            }
        );
    }

    #[test]
    fn test_manifest_header_default() {
        let header = ManifestHeader::default();
        assert_eq!(header.version, "v0");
        assert_eq!(header.message, Some("".to_string()));
        assert_eq!(header.user_meta, None);
        assert_eq!(header.workflow, None);
    }
}
