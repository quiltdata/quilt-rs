//!
//! # Row
//!
//! Row is the native entry format for quilt4.
//! It provides methods to decode/encode quilt3's JSONL format
//!
use std::fmt;
use std::path::PathBuf;

use multihash::Multihash;

use crate::io::remote::S3Attributes;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::manifest::HEADER_ROW;
use crate::Error;

#[derive(Clone, Debug, PartialEq)]
pub struct Row {
    pub name: PathBuf,
    // scheme: Enum<file,s3,https>
    pub place: String,
    pub size: u64,
    pub hash: Multihash<256>,
    pub info: serde_json::Value, // system metadata
    pub meta: serde_json::Value, // user metadata
}

impl fmt::Display for Row {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let result = format!("Row({})", self.name.display())
            + &format!("@{}", self.place)
            + &format!("^{:?}", self.size)
            + &format!("#{:?}", self.hash.digest())
            + &format!("$${:?}", self.info)
            + &format!("${:?}", self.meta);
        write!(f, "{}", result)
    }
}

impl From<Manifest> for Row {
    fn from(quilt3_manifest: Manifest) -> Self {
        Row {
            info: serde_json::json!({
                "message": quilt3_manifest.header.message,
                "version": quilt3_manifest.header.version,
            }),
            meta: match quilt3_manifest.header.user_meta.clone() {
                Some(meta) => meta.into(),
                None => serde_json::Value::Null,
            },
            ..Row::default()
        }
    }
}

impl TryFrom<ManifestRow> for Row {
    type Error = Error;

    fn try_from(manifest_row: ManifestRow) -> Result<Self, Self::Error> {
        Ok(Row {
            name: manifest_row.logical_key,
            place: manifest_row.physical_key,
            hash: manifest_row.hash.try_into()?,
            size: manifest_row.size,
            meta: match manifest_row.meta {
                None => serde_json::Value::Null,
                Some(json) => serde_json::Value::Object(json),
            },
            info: serde_json::Value::Null,
        })
    }
}

impl Default for Row {
    fn default() -> Self {
        Row {
            name: HEADER_ROW.into(),
            place: HEADER_ROW.into(),
            size: 0,
            hash: Multihash::default(),
            info: serde_json::json!({
                "message": String::default(),
                "version": "v0",
            }),
            meta: serde_json::Value::Null,
        }
    }
}

impl From<S3Attributes> for Row {
    fn from(attrs: S3Attributes) -> Row {
        let prefix_len = attrs.listing_uri.key.len();
        let name = PathBuf::from(attrs.object_uri.key[prefix_len..].to_string());
        Row {
            name,
            place: attrs.object_uri.to_string(),
            // XXX: can we use `as u64` safely here?
            size: attrs.size,
            hash: attrs.hash,
            info: serde_json::Value::Null, // XXX: is this right?
            meta: serde_json::Value::Null, // XXX: is this right?
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_formatting_without_path() -> Result<(), multihash::Error> {
        let row = Row {
            name: PathBuf::from("Foo"),
            place: "Bar".to_string(),
            size: 123,
            hash: Multihash::wrap(345, b"hello world")?,
            info: serde_json::Value::Bool(false),
            meta: serde_json::json!({"foo":"bar"}),
        };
        assert_eq!(row.to_string(), r##"Row(Foo)@Bar^123#[104, 101, 108, 108, 111, 32, 119, 111, 114, 108, 100]$$Bool(false)$Object {"foo": String("bar")}"##.to_string());
        Ok(())
    }
}
