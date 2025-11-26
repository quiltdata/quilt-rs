use std::fmt;
use std::path::PathBuf;

use multihash::Multihash;

use crate::io::remote::S3Attributes;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::manifest::Workflow;
use crate::Error;
use crate::Res;
use multibase;

const HEADER_ROW: &str = ".";

/// Represents the header row in Parquet manifest
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    pub(crate) info: serde_json::Value,         // system metadata
    pub(crate) meta: Option<serde_json::Value>, // user metadata
}

// There is some confusion between `display_*` and `get_*` methods :(
// TODO: Probably, it makes sense to create structs
//       for `message`, `user_meta` and `workflow`
//       and implement `From` converters for them
impl Header {
    pub fn new(
        message: Option<String>,
        meta: Option<serde_json::Value>,
        workflow: Option<Workflow>,
    ) -> Header {
        Header {
            info: serde_json::json!({
                "message": message,
                "version": "v0",
                "workflow": match workflow {
                    Some(w) => serde_json::json!(w),
                    None => serde_json::Value::Null,
                },
            }),
            meta,
        }
    }

    pub fn get_message(&self) -> Res<Option<String>> {
        match self.info.get("message") {
            Some(serde_json::Value::String(message)) => Ok(Some(message.clone())),
            _ => Ok(None),
        }
    }

    pub fn get_user_meta(&self) -> Res<Option<serde_json::Value>> {
        Ok(self.meta.clone())
    }

    pub fn get_version(&self) -> Res<String> {
        match self.info.get("version") {
            Some(serde_json::Value::String(version)) => Ok(version.clone()),
            _ => Err(Error::ManifestHeader("Version not found".to_string())),
        }
    }

    pub fn get_workflow(&self) -> Res<Option<Workflow>> {
        match self.info.get("workflow").cloned() {
            Some(serde_json::Value::Null) => Ok(None),
            Some(value) => Ok(Some(serde_json::from_value(value)?)),
            None => Ok(None),
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
            meta: None,
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
#[derive(Clone, Debug, Default)]
pub struct Row {
    pub name: PathBuf,
    pub place: String,
    pub size: u64,
    pub hash: Multihash<256>,
    pub info: serde_json::Value,         // system metadata
    pub meta: Option<serde_json::Value>, // user metadata
}

impl PartialEq for Row {
    fn eq(&self, other: &Self) -> bool {
        // Not: self.place == other.place
        // because we
        //   1. change the place for local files
        //   2. place is not hashed
        self.name == other.name
            && self.size == other.size
            && self.hash == other.hash
            && self.info == other.info
            && self.meta == other.meta
    }
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
            hash_base64: multibase::encode(multibase::Base::Base64Pad, row.hash.digest())[1..]
                .to_string(),
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
        write!(f, "{table}")
    }
}

impl From<&Manifest> for Header {
    fn from(quilt3_manifest: &Manifest) -> Self {
        let header = &quilt3_manifest.header;
        Header {
            info: serde_json::json!({
                "message": header.message,
                "version": header.version,
                "workflow": header.workflow,
            }),
            meta: header.user_meta.clone(),
        }
    }
}

impl TryFrom<ManifestRow> for Row {
    type Error = Error;

    fn try_from(manifest_row: ManifestRow) -> Result<Self, Self::Error> {
        // Extract user_meta from manifest_row.meta if it exists
        let (meta, info) = match manifest_row.meta {
            Some(serde_json::Value::Object(mut obj)) => {
                // Extract user_meta if it exists
                let user_meta = obj.remove("user_meta");
                // The rest of the object becomes info
                (user_meta, serde_json::Value::Object(obj))
            }
            Some(other_value) => {
                // If meta is not an object or doesn't have user_meta, use it as info
                (None, other_value)
            }
            None => {
                // If no meta, both are null
                (None, serde_json::Value::Null)
            }
        };

        Ok(Row {
            name: manifest_row.logical_key,
            place: manifest_row.physical_key,
            hash: manifest_row.hash.into(),
            size: manifest_row.size,
            meta,
            info,
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
            hash: attrs.hash.into(),
            info: serde_json::Value::Null, // XXX: is this right?
            meta: None,                    // XXX: is this right?
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    use crate::manifest::MetadataSchema;
    use crate::manifest::WorkflowId;

    #[test]
    fn test_formatting() -> Res {
        let row = Row {
            name: PathBuf::from("Foo"),
            place: "Bar".to_string(),
            size: 123,
            hash: Multihash::wrap(345, b"hello world")?,
            info: serde_json::Value::Bool(false),
            meta: Some(serde_json::json!({"foo":"bar"})),
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
        use crate::checksum::Sha256ChunkedHash;
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
            hash: Sha256ChunkedHash::try_from("Zm9vYmFy")?.into(),
        };

        assert_eq!(
            Row::from(attrs),
            Row {
                name: PathBuf::from("data/file.txt"),
                place: "s3://test-bucket/prefix/data/file.txt?versionId=v1".to_string(),
                size: 42,
                hash: Sha256ChunkedHash::try_from("Zm9vYmFy")?.into(),
                info: serde_json::Value::Null,
                meta: None,
            }
        );
        Ok(())
    }

    #[test]
    fn test_display_workflow_none() -> Res {
        let header = Header::new(None, None, None);
        assert_eq!(header.get_workflow()?, None);
        Ok(())
    }

    #[test]
    fn test_display_workflow_null() -> Res {
        let header = Header {
            meta: None,
            info: serde_json::json!({
                "message": "",
                "version": "v0",
                "workflow": null,
            }),
        };
        assert_eq!(header.get_workflow()?, None);
        Ok(())
    }

    #[test]
    fn test_display_workflow_invalid() -> Res {
        let header = Header {
            meta: None,
            info: serde_json::json!({
                "message": "",
                "version": "v0",
                "workflow": "invalid",
            }),
        };
        let workflow_result = header.get_workflow();
        assert!(workflow_result.is_err());
        assert_eq!(
            workflow_result.unwrap_err().to_string(),
            "JSON error: invalid type: string \"invalid\", expected struct WorkflowHelper"
        );
        Ok(())
    }

    #[test]
    fn test_display_workflow_valid() -> Res {
        let workflow = Workflow {
            id: Some(WorkflowId {
                id: "test-id".to_string(),
                metadata: Some(MetadataSchema {
                    id: "test-id".to_string(),
                    url: "s3://test-url/workflows/schema.json".parse()?,
                }),
            }),
            config: "s3://test/config".parse()?,
        };
        let header = Header::new(None, None, Some(workflow.clone()));
        assert_eq!(header.get_workflow()?, Some(workflow));
        Ok(())
    }

    #[test]
    fn test_display_workflow_no_id() -> Res {
        let workflow = Workflow {
            id: None,
            config: "s3://test/config".parse()?,
        };
        let header = Header::new(None, None, Some(workflow.clone()));
        assert_eq!(header.get_workflow()?, Some(workflow));
        Ok(())
    }
}
