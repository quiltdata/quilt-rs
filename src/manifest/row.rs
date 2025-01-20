use std::fmt;
use std::path::PathBuf;

use multihash::Multihash;
// use url::Url;

use crate::io::remote::S3Attributes;
use crate::manifest::JsonObject;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::Error;
use crate::Res;

const HEADER_ROW: &str = ".";

// enum PlaceValue {
//   S3Uri(S3Uri),
//   PathBuf(PathBuf),
// }
//
// #[derive(Clone, Debug, PartialEq)]
// pub struct Place {
//     value: PlaceValue,
// }
//
// impl Default for Place {
//     fn default() -> Self {
//         Place {
//             url: Url::from_file_path(PathBuf::default()).unwrap(),
//         }
//     }
// }
//
// impl From<PathBuf> for Place {
// }
//
// impl Into<PathBuf> for Place {
// }

/// Represents the header row in Parquet manifest
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    // TODO: use `message` and `version` instead
    pub info: serde_json::Value, // system metadata
    pub meta: serde_json::Value, // user metadata
}

impl Header {
    pub fn new(message: Option<String>, user_meta: Option<JsonObject>) -> Header {
        Header {
            info: serde_json::json!({
                "message": message.unwrap_or_default(),
                "version": "v0",
            }),
            meta: match user_meta {
                Some(meta) => meta.into(),
                None => serde_json::Value::Null,
            },
        }
    }
}

impl Default for Header {
    fn default() -> Header {
        Header {
            info: serde_json::json!({
                "message": String::default(),
                "version": "v0",
            }),
            meta: serde_json::Value::Null,
        }
    }
}

impl From<Header> for Row {
    fn from(header: Header) -> Self {
        Row {
            name: HEADER_ROW.into(),
            place: HEADER_ROW.into(),
            size: 0,
            hash: Multihash::default(),
            info: header.info,
            meta: header.meta,
        }
    }
}

/// Represents the row in Parquet manifest
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Row {
    pub name: PathBuf,
    pub place: String,
    pub size: u64,
    pub hash: Multihash<256>,
    pub info: serde_json::Value, // system metadata
    pub meta: serde_json::Value, // user metadata
}

impl Row {
    pub fn display_name(&self) -> String {
        self.name.display().to_string()
    }

    pub fn display_place(&self) -> String {
        self.place.clone()
    }

    pub fn display_size(&self) -> u64 {
        self.size
    }

    pub fn display_hash(&self) -> Vec<u8> {
        self.hash.to_bytes()
    }

    pub fn display_meta(&self) -> Res<String> {
        Ok(serde_json::to_string(&self.meta)?)
    }

    pub fn display_info(&self) -> Res<String> {
        Ok(serde_json::to_string(&self.info)?)
    }
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

impl From<&Manifest> for Header {
    fn from(quilt3_manifest: &Manifest) -> Self {
        Header {
            info: serde_json::json!({
                "message": quilt3_manifest.header.message,
                "version": quilt3_manifest.header.version,
            }),
            meta: match quilt3_manifest.header.user_meta.clone() {
                Some(meta) => meta.into(),
                None => serde_json::Value::Null,
            },
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
    fn test_formatting_without_path() -> Res {
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
