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
        Some(serde_json::Value::Object(obj)) => obj.clone(),
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

    use crate::fixtures;
    use crate::Res;

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
                hash: crate::checksum::Sha256ChunkedHash::try_from(
                    "/UMjH1bsbrMLBKdd9cqGGvtjhWzawhz1BfrxgngUhVI=",
                )?
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
}
