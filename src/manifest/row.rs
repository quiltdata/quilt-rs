use std::fmt;
use std::path::PathBuf;

use multihash::Multihash;
// use url::Url;

use crate::io::remote::S3Attributes;
use crate::manifest::JsonObject;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::manifest::Workflow;
use crate::Error;
use crate::Res;
use base64::prelude::BASE64_STANDARD;
use base64::Engine;

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
    pub(crate) info: serde_json::Value, // system metadata
    pub(crate) meta: serde_json::Value, // user metadata
}

// There is some confusion between `display_*` and `get_*` methods :(
// TODO: Probably, it makes sense to create structs
//       for `message`, `user_meta` and `workflow`
//       and implement `From` converters for them
impl Header {
    pub fn new(
        message: Option<String>,
        user_meta: Option<JsonObject>,
        workflow: Option<Workflow>,
    ) -> Header {
        Header {
            info: serde_json::json!({
                "message": message.unwrap_or_default(),
                "version": "v0",
                "workflow": match workflow {
                    Some(w) => serde_json::json!(w),
                    None => serde_json::Value::Null,
                },
            }),
            meta: match user_meta {
                Some(meta) => meta.into(),
                None => serde_json::Value::Null,
            },
        }
    }

    pub fn display_message(&self) -> Option<String> {
        self.info
            .get("message")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string())
    }

    pub fn display_user_meta(&self) -> Option<JsonObject> {
        self.meta.as_object().cloned()
    }

    pub fn display_workflow(&self) -> Option<Workflow> {
        // FIXME: when workflow is null
        match self.info.get("workflow") {
            Some(value) => {
                match value {
                    serde_json::Value::Object(workflow) => Some(Workflow {
                        id: match workflow.get("id").unwrap_or(&serde_json::Value::Null) {
                            serde_json::Value::String(id) => Some(WorkflowId {
                                id: id.to_string(),
                                url: workflow.get("config")
                                    .and_then(|v| v.as_str())
                                    .and_then(|s| s.parse().ok())
                                    .unwrap_or_default()
                            }),
                            _ => None,
                        },
                        config: workflow
                            .get("config")
                            .expect("Workflow URL is empty")
                            .as_str()
                            .expect("Workflow config must be a string")
                            .to_string(),
                    }),
                    serde_json::Value::Null => None,
                    _ => None, // TODO: make Result and return Error
                }
            }
            None => None,
        }
    }

    // TODO: return Result consistently to `get_version`
    pub fn get_message(&self) -> Option<serde_json::Value> {
        self.info.get("message").cloned()
    }

    // TODO: return Result consistently to `get_version`
    //       also, validate the value is object
    pub fn get_user_meta(&self) -> Option<JsonObject> {
        self.meta.as_object().cloned()
    }

    // TODO: return Result<String, Error>, because the value required
    pub fn get_version(&self) -> Option<serde_json::Value> {
        self.info.get("version").cloned()
    }

    // TODO: return Result consistently to `get_version`
    //              also validate the value
    // FIXME: when workflow is null
    pub fn get_workflow(&self) -> Option<serde_json::Value> {
        self.info.get("workflow").cloned()
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

#[derive(tabled::Tabled)]
pub struct RowDisplay {
    name: String,
    place: String,
    size: u64,
    hash_base64: String,
    hash_hex: String,
    info: String,
    meta: String,
}

impl From<&Row> for RowDisplay {
    fn from(row: &Row) -> Self {
        RowDisplay {
            name: row.name.display().to_string(),
            place: row.place.clone(),
            size: row.size,
            hash_base64: BASE64_STANDARD.encode(row.hash.digest()),
            hash_hex: hex::encode(row.hash.to_bytes()),
            info: row
                .display_info()
                .unwrap_or(serde_json::Value::default().to_string()),
            meta: row
                .display_meta()
                .unwrap_or(serde_json::Value::default().to_string()),
        }
    }
}

impl fmt::Display for Row {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let table = tabled::Table::new(vec![RowDisplay::from(self)]);
        write!(f, "{}", table)
    }
}

impl From<&Manifest> for Header {
    fn from(quilt3_manifest: &Manifest) -> Self {
        Header {
            info: serde_json::json!({
                "message": quilt3_manifest.header.message,
                "version": quilt3_manifest.header.version,
                "workflow": quilt3_manifest.header.workflow,
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
    fn test_formatting() -> Res {
        let row = Row {
            name: PathBuf::from("Foo"),
            place: "Bar".to_string(),
            size: 123,
            hash: Multihash::wrap(345, b"hello world")?,
            info: serde_json::Value::Bool(false),
            meta: serde_json::json!({"foo":"bar"}),
        };
        assert_eq!(
            row.to_string(),
            r###"+------+-------+------+------------------+------------------------------+-------+---------------+
| name | place | size | hash_base64      | hash_hex                     | info  | meta          |
+------+-------+------+------------------+------------------------------+-------+---------------+
| Foo  | Bar   | 123  | aGVsbG8gd29ybGQ= | d9020b68656c6c6f20776f726c64 | false | {"foo":"bar"} |
+------+-------+------+------------------+------------------------------+-------+---------------+"###
        );
        Ok(())
    }

    #[test]
    fn test_from_s3_attributes() -> Res {
        use crate::uri::S3Uri;

        let listing_uri = S3Uri {
            bucket: "test-bucket".to_string(),
            key: "prefix/".to_string(),
            version: None,
        };

        let object_uri = S3Uri {
            bucket: "test-bucket".to_string(),
            key: "prefix/data/file.txt".to_string(),
            version: Some("v1".to_string()),
        };

        let attrs = S3Attributes {
            listing_uri,
            object_uri,
            size: 42,
            hash: Multihash::wrap(345, b"test hash")?,
        };

        assert_eq!(
            Row::from(attrs),
            Row {
                name: PathBuf::from("data/file.txt"),
                place: "s3://test-bucket/prefix/data/file.txt?versionId=v1".to_string(),
                size: 42,
                hash: Multihash::wrap(345, b"test hash")?,
                info: serde_json::Value::Null,
                meta: serde_json::Value::Null,
            }
        );
        Ok(())
    }

    #[test]
    fn test_display_workflow_none() {
        let header = Header::new(None, None, None);
        assert_eq!(header.display_workflow(), None);
    }

    #[test]
    fn test_display_workflow_null() {
        let header = Header {
            meta: serde_json::Value::Null,
            info: serde_json::json!({
                "message": "",
                "version": "v0",
                "workflow": null,
            }),
        };
        assert_eq!(header.display_workflow(), None);
    }

    #[test]
    fn test_display_workflow_invalid() {
        let header = Header {
            meta: serde_json::Value::Null,
            info: serde_json::json!({
                "message": "",
                "version": "v0",
                "workflow": "invalid",
            }),
        };
        assert_eq!(header.display_workflow(), None);
    }

    #[test]
    fn test_display_workflow_valid() -> Res {
        let workflow = Workflow {
            id: Some("test-id".to_string()),
            config: "test-config".to_string(),
        };
        let header = Header::new(None, None, Some(workflow.clone()));
        assert_eq!(header.display_workflow(), Some(workflow));
        Ok(())
    }

    #[test]
    fn test_display_workflow_no_id() -> Res {
        let workflow = Workflow {
            id: None,
            config: "test-config".to_string(),
        };
        let header = Header::new(None, None, Some(workflow.clone()));
        assert_eq!(header.display_workflow(), Some(workflow));
        Ok(())
    }
}
