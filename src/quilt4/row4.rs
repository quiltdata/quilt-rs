//!
//! # Row4
//!
//! Row4 is the native entry format for quilt4.
//! It provides methods to decode/encode quilt3's JSONL format
//!
use std::fmt;
use std::path::PathBuf;

use multihash::Multihash;

use crate::quilt::manifest::Manifest;
use crate::quilt4::row3::Row3;
use crate::quilt4::table::HEADER_ROW;

#[derive(Clone, Debug, PartialEq)]
pub struct Row4 {
    pub name: PathBuf,
    // scheme: Enum<file,s3,https>
    pub place: String,
    pub size: u64,
    pub hash: Multihash<256>,
    pub info: serde_json::Value, // system metadata
    pub meta: serde_json::Value, // user metadata
}

impl Row4 {
    pub fn from_row3(_row3: Row3) -> Self {
        // Implementation goes here
        unimplemented!()
    }

    pub fn to_row3(&self) -> Row3 {
        // Implementation goes here
        unimplemented!()
    }
}

impl fmt::Display for Row4 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let result = format!("Row4({})", self.name.display())
            + &format!("@{}", self.place)
            + &format!("^{:?}", self.size)
            + &format!("#{:?}", self.hash.digest())
            + &format!("$${:?}", self.info)
            + &format!("${:?}", self.meta);
        write!(f, "{}", result)
    }
}

impl From<Manifest> for Row4 {
    fn from(quilt3_manifest: Manifest) -> Self {
        Row4 {
            info: serde_json::json!({
                "message": quilt3_manifest.header.message,
                "version": quilt3_manifest.header.version,
            }),
            meta: match quilt3_manifest.header.user_meta.clone() {
                Some(meta) => meta.into(),
                None => serde_json::Value::Null,
            },
            ..Row4::default()
        }
    }
}

impl Default for Row4 {
    fn default() -> Self {
        Row4 {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_formatting_without_path() -> Result<(), multihash::Error> {
        let row = Row4 {
            name: PathBuf::from("Foo"),
            place: "Bar".to_string(),
            size: 123,
            hash: Multihash::wrap(345, b"hello world")?,
            info: serde_json::Value::Bool(false),
            meta: serde_json::json!({"foo":"bar"}),
        };
        assert_eq!(row.to_string(), r##"Row4(Foo)@Bar^123#[104, 101, 108, 108, 111, 32, 119, 111, 114, 108, 100]$$Bool(false)$Object {"foo": String("bar")}"##.to_string());
        Ok(())
    }
}
