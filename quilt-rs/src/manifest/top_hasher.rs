//! Top-level manifest hash calculation utilities.
//!
//! This module provides the `TopHasher` struct and related functions for calculating
//! deterministic hashes of manifest headers and rows to create top-level manifest hashes.

use std::fmt;

use aws_smithy_checksums::ChecksumAlgorithm;
use serde::Serialize;
use serde_json_fmt::JsonFormat;

use crate::manifest::ManifestHeader;
use crate::manifest::ManifestRow;
#[cfg(test)]
use crate::manifest::MetadataSchema;
#[cfg(test)]
use crate::manifest::Workflow;
#[cfg(test)]
use crate::manifest::WorkflowId;
use crate::Res;

/// Serialize JSON to match Python's json.JSONEncoder separators=(',', ':') and ensure_ascii=True
/// TODO: Also implement sort_keys=True to fully match Python's behavior
fn serialize_like_python<T: Serialize>(value: &T) -> Res<String> {
    // Use serde-json-fmt to configure JSON formatting to match Python's behavior
    // JsonFormat::new() defaults to compact format (comma:",", colon":") which matches Python
    let format = JsonFormat::new().ascii(true); // Match Python's ensure_ascii=True - escape non-ASCII characters

    let json_str = format.format_to_string(value)?;
    Ok(json_str)
}

fn serialize_manifest_header(
    manifest_header: &ManifestHeader,
) -> Res<serde_json::Map<String, serde_json::Value>> {
    let mut header_meta = serde_json::Map::new();

    // Handle message
    if let Some(message) = &manifest_header.message {
        header_meta.insert("message".to_string(), serde_json::to_value(message)?);
    } else {
        header_meta.insert("message".to_string(), serde_json::Value::Null);
    }

    // Handle user_meta - preserve null vs missing distinction for header
    if let Some(user_meta) = &manifest_header.user_meta {
        let u = match user_meta {
            serde_json::Value::Object(m) => {
                let mut m = m.clone();
                m.values_mut().for_each(serde_json::Value::sort_all_objects);
                m.sort_keys();
                serde_json::Value::Object(m)
            }
            _ => user_meta.clone(),
        };
        header_meta.insert("user_meta".into(), u);
    }

    header_meta.insert(
        "version".to_string(),
        serde_json::Value::String(manifest_header.version.clone()),
    );

    if let Some(workflow) = &manifest_header.workflow {
        header_meta.insert("workflow".to_string(), serde_json::to_value(workflow)?);
    }

    Ok(header_meta)
}

pub fn serialize_manifest_row_entry(
    manifest_row: &ManifestRow,
) -> Res<serde_json::Map<String, serde_json::Value>> {
    // Handle meta field - match quilt3 behavior where null becomes {}
    let meta = match &manifest_row.meta {
        Some(serde_json::Value::Object(obj)) => {
            let mut m = obj.clone();
            m.values_mut().for_each(serde_json::Value::sort_all_objects);
            m.sort_keys();
            m
        }
        Some(serde_json::Value::Null) | None => serde_json::Map::default(), // quilt3: meta = meta or {}
        Some(other) => {
            // If meta is not an object or null, wrap it in an object
            let mut obj = serde_json::Map::new();
            obj.insert("user_meta".to_string(), other.clone());
            obj
        }
    };

    Ok(serde_json::Map::from_iter([
        (
            "hash".to_string(),
            serde_json::to_value(&manifest_row.hash)?,
        ),
        (
            "logical_key".to_string(),
            serde_json::to_value(&manifest_row.logical_key)?,
        ),
        ("meta".to_string(), serde_json::Value::Object(meta)),
        (
            "size".to_string(),
            serde_json::Value::Number(manifest_row.size.into()),
        ),
    ]))
}

/// Helper for creating `top_hash`
pub struct TopHasher {
    pub hasher: Box<dyn aws_smithy_checksums::Checksum>,
}

impl fmt::Debug for TopHasher {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TopHasher")
            .field("hasher", &"<aws_smithy_checksums::Checksum>")
            .finish()
    }
}

impl Default for TopHasher {
    fn default() -> Self {
        Self::new()
    }
}

impl TopHasher {
    pub fn new() -> Self {
        TopHasher {
            hasher: ChecksumAlgorithm::Sha256.into_impl(),
        }
    }

    /// Append `ManifestHeader` to the hasher
    pub fn append_header(&mut self, manifest_header: &ManifestHeader) -> Res {
        let value = serialize_manifest_header(manifest_header)?;
        let value_str = serialize_like_python(&value)?;
        self.hasher.update(value_str.as_bytes());
        Ok(())
    }

    /// Append `ManifestRow` to the hasher
    pub fn append(&mut self, manifest_row: &ManifestRow) -> Res {
        let value = serialize_manifest_row_entry(manifest_row)?;
        let value_str = serialize_like_python(&value)?;
        self.hasher.update(value_str.as_bytes());
        Ok(())
    }

    /// Consume `self` and return `top_hash`
    pub fn finalize(self) -> String {
        hex::encode(self.hasher.finalize())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use test_log::test;

    use crate::checksum::Crc64Hash;
    use crate::checksum::Sha256ChunkedHash;
    use crate::checksum::Sha256Hash;
    use crate::fixtures;
    use crate::fixtures::objects;
    use crate::fixtures::top_hash;
    use crate::Res;

    #[test]
    fn test_manifest_header_default_no_rows() -> Res {
        let header = ManifestHeader::default();

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::EMPTY_EMPTY_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_none_no_rows() -> Res {
        let header = ManifestHeader {
            user_meta: None,
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::EMPTY_NONE_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_empty_null_no_rows() -> Res {
        let header = ManifestHeader {
            user_meta: Some(serde_json::Value::Null),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::EMPTY_NULL_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_null_empty_no_rows() -> Res {
        let header = ManifestHeader {
            message: None,
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::NULL_EMPTY_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_null_none_no_rows() -> Res {
        let header = ManifestHeader {
            message: None,
            user_meta: None,
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::NULL_NONE_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_null_null_no_rows() -> Res {
        let header = ManifestHeader {
            message: None,
            user_meta: Some(serde_json::Value::Null),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::NULL_NULL_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_empty_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({})),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::INITIAL_EMPTY_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_none_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: None,
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::INITIAL_NONE_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_null_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::Value::Null),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::INITIAL_NULL_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_meta_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({"key": "value"})),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::INITIAL_META_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_complex_meta_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({"author": "user", "timestamp": "2024-01-01"})),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::INITIAL_COMPLEX_META_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_large_meta_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({
                "author": "user",
                "timestamp": "2024-01-01T10:30:00Z",
                "description": "This is a comprehensive test with larger metadata",
                "tags": ["test", "manifest", "quilt"],
                "version": 1,
                "nested": {
                    "key1": "value1",
                    "key2": 42,
                    "key3": true
                }
            })),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::INITIAL_LARGE_META_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_manifest_header_empty_empty_simple_workflow_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("".to_string()),
            user_meta: Some(serde_json::json!({})),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: None,
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(
            calculated_hash,
            top_hash::EMPTY_EMPTY_SIMPLE_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_manifest_header_empty_empty_complex_workflow_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("".to_string()),
            user_meta: Some(serde_json::json!({})),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: Some(WorkflowId {
                    id: "test-workflow".to_string(),
                    metadata: Some(MetadataSchema {
                        id: "test-schema".to_string(),
                        url: "s3://bucket/workflows/test.json".parse()?,
                    }),
                }),
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(
            calculated_hash,
            top_hash::EMPTY_EMPTY_COMPLEX_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_empty_simple_workflow_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({})),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: None,
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(
            calculated_hash,
            top_hash::INITIAL_EMPTY_SIMPLE_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_empty_complex_workflow_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({})),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: Some(WorkflowId {
                    id: "test-workflow".to_string(),
                    metadata: Some(MetadataSchema {
                        id: "test-schema".to_string(),
                        url: "s3://bucket/workflows/test.json".parse()?,
                    }),
                }),
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(
            calculated_hash,
            top_hash::INITIAL_EMPTY_COMPLEX_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_manifest_header_empty_none_simple_workflow_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("".to_string()),
            user_meta: None,
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: None,
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(
            calculated_hash,
            top_hash::EMPTY_NONE_SIMPLE_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_manifest_header_empty_null_simple_workflow_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("".to_string()),
            user_meta: Some(serde_json::Value::Null),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: None,
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(
            calculated_hash,
            top_hash::EMPTY_NULL_SIMPLE_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_meta_simple_workflow_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: Some(serde_json::json!({"key": "value"})),
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: None,
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(
            calculated_hash,
            top_hash::INITIAL_META_SIMPLE_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_manifest_header_initial_none_complex_workflow_no_rows() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: None,
            workflow: Some(Workflow {
                config: "s3://workflow/config".parse()?,
                id: Some(WorkflowId {
                    id: "test-workflow".to_string(),
                    metadata: Some(MetadataSchema {
                        id: "test-schema".to_string(),
                        url: "s3://bucket/workflows/test.json".parse()?,
                    }),
                }),
            }),
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(
            calculated_hash,
            top_hash::INITIAL_NONE_COMPLEX_WORKFLOW_TOP_HASH
        );

        Ok(())
    }

    #[test]
    fn test_checksummed_manifest_top_hash_direct() -> Res {
        let header = ManifestHeader {
            message: Some("Initial".to_string()),
            user_meta: None,
            ..ManifestHeader::default()
        };

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        for i in 0..10 {
            let manifest_row = ManifestRow {
                logical_key: PathBuf::from(format!("e0-{}.txt", i)),
                physical_key: "ignored".to_string(),
                hash: Sha256ChunkedHash::try_from("/UMjH1bsbrMLBKdd9cqGGvtjhWzawhz1BfrxgngUhVI=")?
                    .into(),
                size: 29,
                meta: Some(serde_json::Value::Null),
            };
            top_hasher.append(&manifest_row)?;
        }

        let calculated_hash = top_hasher.finalize();
        assert_eq!(calculated_hash, fixtures::manifest::CHECKSUMMED_HASH);

        Ok(())
    }

    #[test]
    fn test_single_row() -> Res {
        // Single row with default header
        let header = ManifestHeader::default();

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        let manifest_row = ManifestRow {
            logical_key: PathBuf::from("data.txt"),
            physical_key: "s3://bucket/data.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::LESS_THAN_8MB_HASH_B64)?.into(),
            size: 16,
            meta: Some(serde_json::json!({"type": "text"})),
        };
        top_hasher.append(&manifest_row)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::SINGLE_ROW_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_mixed_hash_types() -> Res {
        // Mixed hash types with default header
        let header = ManifestHeader::default();

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        // Row 1: SHA256 hash (legacy format)
        let row1 = ManifestRow {
            logical_key: PathBuf::from("file1.txt"),
            physical_key: "s3://bucket/file1.txt".to_string(),
            hash: Sha256Hash::try_from(
                "7465737464617461000000000000000000000000000000000000000000000000",
            )?
            .into(),
            size: 8,
            meta: None,
        };
        top_hasher.append(&row1)?;

        // Row 2: SHA256-chunked hash (current format)
        let row2 = ManifestRow {
            logical_key: PathBuf::from("file2.txt"),
            physical_key: "s3://bucket/file2.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::LESS_THAN_8MB_HASH_B64)?.into(),
            size: 16,
            meta: None,
        };
        top_hasher.append(&row2)?;

        // Row 3: CRC64-NVMe hash (newest format)
        let row3 = ManifestRow {
            logical_key: PathBuf::from("file3.txt"),
            physical_key: "s3://bucket/file3.txt".to_string(),
            hash: Crc64Hash::try_from("dGVzdGRhdGEAAAAAAAAAAAAAAAAAAAAA")?.into(),
            size: 32,
            meta: None,
        };
        top_hasher.append(&row3)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::MIXED_HASH_TYPES_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_hash_normalization_equivalence() -> Res {
        // Test that different JSON representations produce the same hash
        // when they represent equivalent data structures

        // First manifest: meta: {}, logical_key first
        let header1 = ManifestHeader::default();
        let mut top_hasher1 = TopHasher::new();
        top_hasher1.append_header(&header1)?;

        let row1a = ManifestRow {
            logical_key: PathBuf::from("test1.txt"),
            physical_key: "s3://bucket/test1.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::ZERO_HASH_B64)?.into(),
            size: 0,
            meta: Some(serde_json::json!({})), // Empty object
        };

        let row1b = ManifestRow {
            logical_key: PathBuf::from("test2.txt"),
            physical_key: "s3://bucket/test2.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::LESS_THAN_8MB_HASH_B64)?.into(),
            size: 16,
            meta: Some(serde_json::json!({"alpha": "first", "beta": "second"})), // Keys in alphabetical order
        };

        top_hasher1.append(&row1a)?;
        top_hasher1.append(&row1b)?;
        let hash1 = top_hasher1.finalize();

        // Second manifest: meta: null (becomes {}), different key order, field order doesn't matter for hashing
        let header2 = ManifestHeader::default();
        let mut top_hasher2 = TopHasher::new();
        top_hasher2.append_header(&header2)?;

        let row2a = ManifestRow {
            logical_key: PathBuf::from("test1.txt"),
            physical_key: "s3://bucket/test1.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::ZERO_HASH_B64)?.into(),
            size: 0,
            meta: Some(serde_json::Value::Null), // Null becomes {}
        };

        let row2b = ManifestRow {
            logical_key: PathBuf::from("test2.txt"),
            physical_key: "s3://bucket/test2.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::LESS_THAN_8MB_HASH_B64)?.into(),
            size: 16,
            meta: Some(serde_json::json!({"beta": "second", "alpha": "first"})), // Keys in different order
        };

        top_hasher2.append(&row2a)?;
        top_hasher2.append(&row2b)?;
        let hash2 = top_hasher2.finalize();

        // Third manifest: meta: None (becomes {})
        let header3 = ManifestHeader::default();
        let mut top_hasher3 = TopHasher::new();
        top_hasher3.append_header(&header3)?;

        let row3a = ManifestRow {
            logical_key: PathBuf::from("test1.txt"),
            physical_key: "s3://bucket/test1.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::ZERO_HASH_B64)?.into(),
            size: 0,
            meta: None, // None becomes {}
        };

        let row3b = ManifestRow {
            logical_key: PathBuf::from("test2.txt"),
            physical_key: "s3://bucket/test2.txt".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::LESS_THAN_8MB_HASH_B64)?.into(),
            size: 16,
            meta: Some(serde_json::json!({"beta": "second", "alpha": "first"})), // Same as above after normalization
        };

        top_hasher3.append(&row3a)?;
        top_hasher3.append(&row3b)?;
        let hash3 = top_hasher3.finalize();

        // All three should produce the same hash despite different representations
        assert_eq!(
            hash1, hash2,
            "Empty object {{}} and null should normalize to same hash"
        );
        assert_eq!(
            hash1, hash3,
            "Empty object {{}}, null, and None should normalize to same hash"
        );
        assert_eq!(
            hash2, hash3,
            "All meta empty representations should normalize to same hash"
        );

        // Test that the normalized hash matches our expected constant
        assert_eq!(hash1, top_hash::NORMALIZED_EQUIVALENCE_TOP_HASH);

        Ok(())
    }

    #[test]
    fn test_multiple_rows() -> Res {
        // Multiple rows with default header
        let header = ManifestHeader::default();

        let mut top_hasher = TopHasher::new();
        top_hasher.append_header(&header)?;

        // Row 1: Small file
        let row1 = ManifestRow {
            logical_key: PathBuf::from("config.json"),
            physical_key: "s3://bucket/config.json".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::ZERO_HASH_B64)?.into(),
            size: 0,
            meta: Some(serde_json::json!({"format": "json"})),
        };
        top_hasher.append(&row1)?;

        // Row 2: Medium file
        let row2 = ManifestRow {
            logical_key: PathBuf::from("data/file.csv"),
            physical_key: "s3://bucket/data/file.csv".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::EQUAL_TO_8MB_HASH_B64)?.into(),
            size: 8388608,
            meta: Some(serde_json::Value::Null),
        };
        top_hasher.append(&row2)?;

        // Row 3: Large file
        let row3 = ManifestRow {
            logical_key: PathBuf::from("images/photo.jpg"),
            physical_key: "s3://bucket/images/photo.jpg".to_string(),
            hash: Sha256ChunkedHash::try_from(objects::MORE_THAN_8MB_HASH_B64)?.into(),
            size: 18874368,
            meta: Some(serde_json::json!({"width": 1920, "height": 1080})),
        };
        top_hasher.append(&row3)?;

        let calculated_hash = top_hasher.finalize();

        assert_eq!(calculated_hash, top_hash::MULTIPLE_ROWS_TOP_HASH);

        Ok(())
    }
}
