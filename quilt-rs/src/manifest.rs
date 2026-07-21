//!
//! Namespace contains helpers to work with manifest and its content (rows).

mod top_hasher;

pub use top_hasher::TopHasher;

pub use crate::workflow::Workflow;
pub use crate::workflow::WorkflowId;

use std::path::PathBuf;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncRead;
use tokio::io::BufReader;

use crate::Error;
use crate::Res;
use crate::error::ManifestError;
use crate::io::manifest::RowsStream;
use crate::io::manifest::StreamRowsChunk;
use crate::io::storage::ByteStream;
use crate::object_hash;

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
            message: Some(String::new()),
            user_meta: Some(serde_json::Value::Object(serde_json::Map::new())),
            workflow: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct Quilt3ManifestRow {
    pub logical_key: PathBuf,
    pub physical_keys: Vec<String>,
    pub hash: object_hash::ObjectHash,
    // XXX: u64 cannot be safely deserialized by standard JS json parser,
    //      which treats numbers as 64-bit floats.
    //      However, having file size more than ~9PB (max safe/lossless integer - 53 bits)
    //      is quite unlikely ATM, given S3 limitations of 5TB per object.
    #[serde(deserialize_with = "number_to_u64")]
    pub size: u64,
    pub meta: Option<serde_json::Value>,
}

fn number_to_u64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    // See note on `Quilt3ManifestRow::size` — JSON numbers come through as f64;
    // sizes within S3's 5TB cap fit losslessly in the f64 mantissa.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
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
#[derive(Clone, Debug, Deserialize, Serialize, Default, PartialEq)]
pub struct ManifestRow {
    pub logical_key: PathBuf,
    // XXX: use Url to have validated string?
    pub physical_key: String,
    pub hash: object_hash::ObjectHash,
    pub size: u64,
    pub meta: Option<serde_json::Value>,
}

impl ManifestRow {
    /// Content-identity check: two rows describe the same logical file with
    /// the same bytes. Ignores `physical_key` (same content is addressed as
    /// `file:///.../objects/<hash>` locally and `s3://...` remotely, and the
    /// push flow uses this to reuse the remote location instead of
    /// re-uploading) and `meta` (user metadata that does not change the
    /// stored bytes, so a metadata-only edit should not force a re-upload).
    pub fn matches_content(&self, other: &Self) -> bool {
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
                .ok_or(ManifestError::Header("Physical key is missing".to_string()))?,
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
            Error::Manifest(ManifestError::Header(format!(
                "Failed to read the manifest header: {err}"
            )))
        })?;

        let Some(header_str) = header else {
            return Err(Error::Manifest(ManifestError::Header(
                "Empty manifest".into(),
            )));
        };

        // Parse the raw JSON to check if user_meta is explicitly null
        let raw_value: serde_json::Value = serde_json::from_str(&header_str)?;

        // Parse the header normally
        let mut header: ManifestHeader = serde_json::from_str(&header_str)?;

        // Handle user_meta field based on the raw JSON
        if let Some(user_meta) = raw_value.get("user_meta")
            && user_meta.is_null()
        {
            header.user_meta = Some(serde_json::Value::Null);
        }

        if header.version != "v0" {
            return Err(Error::Manifest(ManifestError::Header(format!(
                "Unsupported manifest version: {}",
                header.version
            ))));
        }

        let mut rows = Vec::new();

        while let Some(line) = lines.next_line().await? {
            let row: Quilt3ManifestRow = serde_json::from_str(&line)?;
            rows.push(ManifestRow::try_from(row)?);
        }

        Ok(Manifest { header, rows })
    }

    /// Serialize the manifest to JSONL: the header line followed by one line
    /// per row.
    ///
    /// # Panics
    ///
    /// Panics if serializing the header or a row to JSON fails, which does not
    /// happen for well-formed manifest values.
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

    /// Read manifest from a file path, converting from Table format if needed
    pub async fn from_path(
        storage: &impl crate::io::storage::Storage,
        path: &std::path::Path,
    ) -> Res<Self> {
        let file = storage.open_file(path).await?;
        Self::from_reader(file).await.map_err(|e| {
            crate::Error::Manifest(ManifestError::Load {
                path: path.to_path_buf(),
                source: Box::new(e),
            })
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
    /// Returns a stream of Row chunks for compatibility with `io::manifest` streaming functions
    /// Sorted by `logical_key` to match `Table`'s `BTreeMap` behavior and uses proper `TryFrom` conversion
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

impl From<&Manifest> for ByteStream {
    fn from(manifest: &Manifest) -> Self {
        manifest.to_jsonlines().into_bytes().into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use std::path::PathBuf;

    use crate::fixtures;
    use aws_sdk_s3::primitives::ByteStream;

    use crate::io::storage::LocalStorage;
    use crate::io::storage::Storage;
    use crate::io::storage::mocks::MockStorage;

    #[test]
    fn test_matches_content_identical_rows() -> Res {
        let left = ManifestRow {
            logical_key: PathBuf::from("A"),
            physical_key: "B".to_string(),
            hash: object_hash::Sha256Hash::try_from("deadbeef")?.into(),
            size: 1,
            meta: None,
        };
        let right = ManifestRow {
            logical_key: PathBuf::from("A"),
            physical_key: "B".to_string(),
            hash: object_hash::Sha256Hash::try_from("deadbeef")?.into(),
            size: 1,
            meta: None,
        };
        assert!(left.matches_content(&right));
        Ok(())
    }

    #[test]
    fn test_matches_content_ignores_physical_key_and_meta() -> Res {
        let mut meta = serde_json::Map::new();
        meta.insert("foo".to_string(), serde_json::json!("bar"));
        let left = ManifestRow {
            logical_key: PathBuf::from("A"),
            physical_key: "FOO".to_string(),
            hash: object_hash::Sha256Hash::try_from("deadbeef")?.into(),
            size: 1,
            meta: Some(serde_json::Value::Object(meta)),
        };
        let right = ManifestRow {
            logical_key: PathBuf::from("A"),
            physical_key: "BAR".to_string(),
            hash: object_hash::Sha256Hash::try_from("deadbeef")?.into(),
            size: 1,
            meta: None,
        };
        assert!(left.matches_content(&right));
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
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing field `version`")
        );
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
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to read the manifest header")
        );
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
        if let Err(Error::Manifest(ManifestError::Header(error_string))) = result {
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
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing field `logical_key`")
        );
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
                        hash: object_hash::Sha256ChunkedHash::try_from("/UMjH1bsbrMLBKdd9cqGGvtjhWzawhz1BfrxgngUhVI=")?.into(),
                        meta: Some(serde_json::json!({})),
                    }
        );
        assert_eq!(
            checksummed_manifest.rows[9],
                    ManifestRow {
                        logical_key: PathBuf::from("e0-9.txt".to_string()),
                        physical_key: "s3://data-yaml-spec-tests/scale/10u/e0-9.txt?versionId=T5tkWkC.7PVcpiFYRoCQKhhKC249fdBC".to_string(),
                        size: 29,
                        hash: object_hash::Sha256ChunkedHash::try_from("/UMjH1bsbrMLBKdd9cqGGvtjhWzawhz1BfrxgngUhVI=")?.into(),
                        meta: Some(serde_json::json!({})),
                    }
        );
        Ok(())
    }

    #[test]
    fn test_manifest_header_default() -> Res {
        let header = ManifestHeader::default();
        assert_eq!(header.version, "v0");
        assert_eq!(header.message, Some(String::new()));
        assert_eq!(header.user_meta, Some(serde_json::json!({})));
        assert_eq!(header.workflow, None);
        Ok(())
    }
}
