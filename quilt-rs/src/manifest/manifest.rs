use std::collections::HashMap;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncRead;
use tokio::io::BufReader;

use crate::checksum;
use crate::io::manifest::RowsStream;
use crate::io::manifest::StreamRowsChunk;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

#[derive(Debug, Clone, PartialEq)]
pub struct MetadataSchema {
    pub id: String,
    pub url: S3Uri,
}

#[derive(Debug, Clone, PartialEq)]
pub struct WorkflowId {
    pub id: String,
    pub metadata: Option<MetadataSchema>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Workflow {
    pub config: S3Uri,
    pub id: Option<WorkflowId>,
}

impl<'de> Deserialize<'de> for WorkflowId {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Ok(WorkflowId {
            id: s,
            metadata: None, // This will be filled in from schemas
        })
    }
}

impl<'de> Deserialize<'de> for Workflow {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        #[derive(Deserialize)]
        struct WorkflowHelper {
            config: String,
            id: Option<String>,
            schemas: Option<HashMap<String, String>>,
        }

        let helper = WorkflowHelper::deserialize(deserializer)?;

        let id = match (helper.id, helper.schemas) {
            (None, _) => None,
            (Some(id), None) => Some(WorkflowId {
                id: id.clone(),
                metadata: None,
            }),
            (Some(id), Some(schemas)) => match schemas.iter().collect::<Vec<_>>().first() {
                Some((schema_id, schema_url)) => match schema_url.parse() {
                    Ok(url) => Some(WorkflowId {
                        id: id.clone(),
                        metadata: Some(MetadataSchema {
                            id: schema_id.to_string(),
                            url,
                        }),
                    }),
                    Err(_) => {
                        return Err(serde::de::Error::custom(Error::S3Uri(
                            schema_url.to_string(),
                        )))
                    }
                },
                None => None,
            },
        };

        Ok(Workflow {
            config: helper.config.parse().map_err(serde::de::Error::custom)?,
            id,
        })
    }
}

impl Serialize for Workflow {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        use serde::ser::SerializeStruct;
        match &self.id {
            Some(workflow_id) => {
                let mut state = serializer.serialize_struct("Workflow", 3)?;
                state.serialize_field("config", &self.config.to_string())?;
                state.serialize_field("id", &workflow_id.id)?;
                if let Some(metadata) = &workflow_id.metadata {
                    let mut schemas = HashMap::new();
                    schemas.insert(metadata.id.clone(), metadata.url.to_string());
                    state.serialize_field("schemas", &schemas)?;
                }
                state.end()
            }
            None => {
                let mut state = serializer.serialize_struct("Workflow", 2)?;
                state.serialize_field("config", &self.config.to_string())?;
                state.serialize_field("id", &None::<String>)?;
                state.end()
            }
        }
    }
}

/// Header (or first row) in JSONL manifest
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct ManifestHeader {
    pub version: String,
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] // Attempt to be quilt3-compatible.
    pub user_meta: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow: Option<Workflow>,
}

impl Default for ManifestHeader {
    fn default() -> Self {
        ManifestHeader {
            version: "v0".to_string(),
            message: Some("".to_string()),
            user_meta: Some(serde_json::Value::Object(serde_json::Map::new())),
            workflow: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct Quilt3ManifestRow {
    pub logical_key: PathBuf,
    pub physical_keys: Vec<String>,
    pub hash: checksum::ObjectHash,
    // XXX: u64 cannot be safely deserialized by standard JS json parser,
    //      which treats numbers as 64-bit floats.
    //      However, having file size more than ~9PB (max safe/lossless integer - 53 bits)
    //      is quite unlikely ATM, given S3 limitations of 5TB per object.
    #[serde(deserialize_with = "number_to_u64")]
    pub size: u64,
    pub meta: Option<serde_json::Value>,
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
#[derive(Clone, Debug, Deserialize, Serialize, Default)]
pub struct ManifestRow {
    pub logical_key: PathBuf,
    // XXX: use Url to have validated string?
    pub physical_key: String,
    pub hash: checksum::ObjectHash,
    pub size: u64,
    pub meta: Option<serde_json::Value>,
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
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Default)]
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
            Error::ManifestHeader(format!("Failed to read the manifest header: {err}"))
        })?;

        let Some(header_str) = header else {
            return Err(Error::ManifestHeader("Empty manifest".into()));
        };

        // Parse the raw JSON to check if user_meta is explicitly null
        let raw_value: serde_json::Value = serde_json::from_str(&header_str)?;

        // Parse the header normally
        let mut header: ManifestHeader = serde_json::from_str(&header_str)?;

        // Handle user_meta field based on the raw JSON
        if let Some(user_meta) = raw_value.get("user_meta") {
            if user_meta.is_null() {
                header.user_meta = Some(serde_json::Value::Null);
            }
        }

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

    /// Read manifest from a file path, converting from Table format if needed
    pub async fn from_path(
        storage: &impl crate::io::storage::Storage,
        path: &std::path::Path,
    ) -> Res<Self> {
        let file = storage.open_file(path).await?;
        Self::from_reader(file)
            .await
            .map_err(|e| crate::Error::ManifestLoad {
                path: path.to_path_buf(),
                source: Box::new(e),
            })
    }

    /// Find a record by path (for compatibility with Table API)
    pub fn get_record(&self, path: &PathBuf) -> Option<&ManifestRow> {
        self.rows.iter().find(|row| &row.logical_key == path)
    }

    /// Check if manifest contains a record for the given path
    pub fn contains_record(&self, path: &PathBuf) -> bool {
        self.rows.iter().any(|row| &row.logical_key == path)
    }

    /// Create a stream of rows compatible with Table API
    /// Returns a stream of Row chunks for compatibility with io::manifest streaming functions
    /// Sorted by logical_key to match Table's BTreeMap behavior and uses proper TryFrom conversion
    pub async fn records_stream(&self) -> impl RowsStream {
        // Sort by logical_key to match Table's BTreeMap ordering
        let mut indices: Vec<usize> = (0..self.rows.len()).collect();
        indices.sort_by(|&a, &b| self.rows[a].logical_key.cmp(&self.rows[b].logical_key));

        let rows: StreamRowsChunk = indices
            .into_iter()
            .map(|i| Ok(self.rows[i].clone()))
            .collect();
        tokio_stream::iter(vec![Ok(rows)])
    }

    /// Get the number of records in the manifest
    pub async fn records_len(&self) -> usize {
        self.rows.len()
    }

    /// Insert a record into the manifest (for compatibility with Table API)
    pub async fn insert_record(&mut self, row: ManifestRow) -> Res<Option<ManifestRow>> {
        // Check if row already exists
        let existing_pos = self
            .rows
            .iter()
            .position(|r| r.logical_key == row.logical_key);

        if let Some(pos) = existing_pos {
            Ok(Some(std::mem::replace(&mut self.rows[pos], row)))
        } else {
            self.rows.push(row);
            Ok(None)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use std::path::PathBuf;

    use crate::fixtures;
    use aws_sdk_s3::primitives::ByteStream;

    use crate::io::storage::mocks::MockStorage;
    use crate::io::storage::LocalStorage;
    use crate::io::storage::Storage;

    #[test]
    fn test_equality_of_strictly_equal() -> Res {
        let left = ManifestRow {
            logical_key: PathBuf::from("A"),
            physical_key: "B".to_string(),
            hash: checksum::Sha256Hash::try_from("deadbeef")?.into(),
            size: 1,
            meta: None,
        };
        let right = ManifestRow {
            logical_key: PathBuf::from("A"),
            physical_key: "B".to_string(),
            hash: checksum::Sha256Hash::try_from("deadbeef")?.into(),
            size: 1,
            meta: None,
        };
        assert!(left == right);
        Ok(())
    }

    #[test]
    fn test_equality_of_partialy_equal() -> Res {
        let mut meta = serde_json::Map::new();
        meta.insert("foo".to_string(), serde_json::json!("bar"));
        let left = ManifestRow {
            logical_key: PathBuf::from("A"),
            physical_key: "FOO".to_string(),
            hash: checksum::Sha256Hash::try_from("deadbeef")?.into(),
            size: 1,
            meta: Some(serde_json::Value::Object(meta)),
        };
        let right = ManifestRow {
            logical_key: PathBuf::from("A"),
            physical_key: "BAR".to_string(),
            hash: checksum::Sha256Hash::try_from("deadbeef")?.into(),
            size: 1,
            meta: None,
        };
        assert!(left == right);
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_manifest_from_reader_invalid() -> Res {
        let storage = MockStorage::default();
        let invalid_content = r#"{"invalid": "json"}"#;
        let path = PathBuf::from("invalid_manifest.jsonl");
        storage
            .write_byte_stream(&path, ByteStream::from_static(invalid_content.as_bytes()))
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

    #[test(tokio::test)]
    async fn test_manifest_from_reader_empty() -> Res {
        let storage = MockStorage::default();
        let path = PathBuf::from("empty_manifest.jsonl");
        storage
            .write_byte_stream(&path, ByteStream::default())
            .await?;
        let file = storage.open_file(&path).await?;

        let result = Manifest::from_reader(file).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Manifest header: Empty manifest"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_manifest_from_reader_invalid_utf8() -> Res {
        let storage = MockStorage::default();
        let invalid_content = b"\xFF\xFF\xFF\xFF"; // Invalid UTF-8 bytes
        let path = PathBuf::from("invalid_utf8_manifest.jsonl");
        storage
            .write_byte_stream(&path, ByteStream::from_static(invalid_content))
            .await?;
        let file = storage.open_file(&path).await?;

        let result = Manifest::from_reader(file).await;
        assert!(result.is_err());
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("Failed to read the manifest header"));
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_manifest_from_reader_unsupported_version() -> Res {
        let storage = MockStorage::default();
        let invalid_content = r#"{"version": "v1"}"#;
        let path = PathBuf::from("unsupported_version_manifest.jsonl");
        storage
            .write_byte_stream(&path, ByteStream::from_static(invalid_content.as_bytes()))
            .await?;
        let file = storage.open_file(&path).await?;

        let result = Manifest::from_reader(file).await;
        if let Err(Error::ManifestHeader(error_string)) = result {
            assert_eq!(error_string, "Unsupported manifest version: v1");
        } else {
            panic!("Expected ManifestHeader error, got: {result:?}");
        }
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_manifest_from_reader_invalid_row() -> Res {
        let storage = MockStorage::default();
        let invalid_content = r#"{"version": "v0"}
{"invalid": "row"}"#;
        let path = PathBuf::from("invalid_row_manifest.jsonl");
        storage
            .write_byte_stream(&path, ByteStream::from_static(invalid_content.as_bytes()))
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

    #[test(tokio::test)]
    async fn test_manifest_from_reader_empty_physical_keys() -> Res {
        let storage = MockStorage::default();
        let invalid_content = r#"{"version": "v0"}
{"logical_key": "test.txt", "physical_keys": [], "size": 0, "hash": {"type": "SHA256", "value": "abc123"}, "meta": {}}"#;
        let path = PathBuf::from("empty_physical_keys_manifest.jsonl");
        storage
            .write_byte_stream(&path, ByteStream::from_static(invalid_content.as_bytes()))
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

    #[test(tokio::test)]
    async fn test_manifest_from_reader_valid() -> Res {
        let storage = LocalStorage::default();
        let file = storage.open_file(fixtures::manifest::path()?).await?;
        let checksummed_manifest = Manifest::from_reader(file).await?;

        assert_eq!(
            checksummed_manifest.header,
            ManifestHeader {
                message: Some("Initial".to_string()),
                user_meta: None,
                ..ManifestHeader::default()
            }
        );
        assert_eq!(
            checksummed_manifest.rows[0],
                    ManifestRow {
                        logical_key: PathBuf::from("e0-0.txt".to_string()),
                        physical_key: "s3://data-yaml-spec-tests/scale/10u/e0-0.txt?versionId=jHb6DGN43Ex7EhbxZc2G9JnAkWSeTfEY".to_string(),
                        size: 29,
                        hash: checksum::Sha256ChunkedHash::try_from("/UMjH1bsbrMLBKdd9cqGGvtjhWzawhz1BfrxgngUhVI=")?.into(),
                        meta: Some(serde_json::Value::Null),
                    }
        );
        assert_eq!(
            checksummed_manifest.rows[9],
                    ManifestRow {
                        logical_key: PathBuf::from("e0-9.txt".to_string()),
                        physical_key: "s3://data-yaml-spec-tests/scale/10u/e0-9.txt?versionId=T5tkWkC.7PVcpiFYRoCQKhhKC249fdBC".to_string(),
                        size: 29,
                        hash: checksum::Sha256ChunkedHash::try_from("/UMjH1bsbrMLBKdd9cqGGvtjhWzawhz1BfrxgngUhVI=")?.into(),
                        meta: Some(serde_json::Value::Null),
                    }
        );
        Ok(())
    }

    #[test]
    fn test_manifest_header_default() -> Res {
        let header = ManifestHeader::default();
        assert_eq!(header.version, "v0");
        assert_eq!(header.message, Some("".to_string()));
        assert_eq!(header.user_meta, Some(serde_json::json!({})));
        assert_eq!(header.workflow, None);
        Ok(())
    }

    #[test]
    fn test_workflow_deserialization() -> Res {
        let json = r#"{
            "config": "s3://workflow/config",
            "id": "test-workflow",
            "schemas": {
                "test-schema": "s3://bucket/workflows/test.json"
            }
        }"#;

        let workflow: Workflow = serde_json::from_str(json)?;

        assert_eq!(workflow.config, "s3://workflow/config".parse()?);
        assert_eq!(
            workflow.id,
            Some(WorkflowId {
                id: "test-workflow".to_string(),
                metadata: Some(MetadataSchema {
                    id: "test-schema".to_string(),
                    url: "s3://bucket/workflows/test.json".parse()?
                })
            })
        );
        Ok(())
    }

    #[test]
    fn test_workflow_deserialization_none() -> Res {
        let json = r#"{
            "config": "s3://workflow/config",
            "id": null
        }"#;

        let workflow: Workflow = serde_json::from_str(json)?;

        assert_eq!(workflow.config, "s3://workflow/config".parse()?);
        assert_eq!(workflow.id, None);
        Ok(())
    }

    #[test]
    fn test_workflow_serialization() -> Res {
        let workflow = Workflow {
            config: "s3://workflow/config".parse()?,
            id: Some(WorkflowId {
                id: "test-workflow".to_string(),
                metadata: Some(MetadataSchema {
                    id: "test-schema".to_string(),
                    url: "s3://bucket/workflows/test.json".parse()?,
                }),
            }),
        };

        let json = serde_json::to_value(&workflow).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "config": "s3://workflow/config",
                "id": "test-workflow",
                "schemas": {
                    "test-schema": "s3://bucket/workflows/test.json"
                }
            })
        );
        Ok(())
    }

    #[test]
    fn test_workflow_serialization_none() -> Res {
        let workflow = Workflow {
            config: "s3://workflow/config".parse()?,
            id: None,
        };

        let json = serde_json::to_value(&workflow).unwrap();

        assert_eq!(
            json,
            serde_json::json!({
                "config": "s3://workflow/config",
                "id": null
            })
        );
        Ok(())
    }
}
