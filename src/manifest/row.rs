use std::fmt;
use std::path::PathBuf;

use multihash::Multihash;
use url::Url;

use crate::io::remote::S3Attributes;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::uri::S3Uri;
use crate::Error;
use crate::Res;

const HEADER_ROW: &str = ".";

#[derive(Clone, Debug, PartialEq)]
pub enum PlaceValue {
    Header,
    PathBuf(PathBuf),
    S3Uri(S3Uri),
    SharePoint(Url), // TODO: SharePointUri
}

#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub value: PlaceValue,
}

impl Place {
    pub fn new(value: PlaceValue) -> Self {
        Place { value }
    }

    pub fn from_path_buf(path: PathBuf) -> Self {
        Place::new(PlaceValue::PathBuf(path))
    }

    pub fn from_s3_uri(s3_uri: S3Uri) -> Self {
        Place::new(PlaceValue::S3Uri(s3_uri))
    }

    pub fn from_sharepoint_uri(url: Url) -> Self {
        Place::new(PlaceValue::SharePoint(url))
    }

    pub fn header() -> Self {
        Place {
            value: PlaceValue::Header,
        }
    }
}

impl Default for Place {
    fn default() -> Self {
        Place {
            value: PlaceValue::PathBuf(PathBuf::default()),
        }
    }
}

impl From<PathBuf> for Place {
    fn from(path: PathBuf) -> Place {
        Place {
            value: PlaceValue::PathBuf(path),
        }
    }
}

impl From<Place> for PathBuf {
    fn from(place: Place) -> Self {
        match place.value {
            PlaceValue::PathBuf(path) => path,
            _ => panic!("Place is not a file://"),
        }
    }
}

impl From<S3Uri> for Place {
    fn from(s3_uri: S3Uri) -> Place {
        Place {
            value: PlaceValue::S3Uri(s3_uri),
        }
    }
}

impl From<Place> for S3Uri {
    fn from(place: Place) -> Self {
        match place.value {
            PlaceValue::S3Uri(s3_uri) => s3_uri,
            _ => panic!("Place is not an S3 URI"),
        }
    }
}

impl TryFrom<&str> for Place {
    type Error = Error;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        let s3_uri = S3Uri::try_from(input);
        if s3_uri.is_ok() {
            return s3_uri.map(Place::from);
        }
        let s3_uri = Url::try_from(input);
        if s3_uri.is_ok() {
            let s3_uri = s3_uri.unwrap();
            if s3_uri.scheme() == "file" {
                let path = match s3_uri.domain() {
                    Some(domain) => format!("{}{}", domain, s3_uri.path()),
                    None => s3_uri.path().to_string(),
                };
                return Ok(Place::from(PathBuf::from(path)));
            }
        }
        Err(Self::Error::Place(input.to_string()))
    }
}

impl TryFrom<String> for Place {
    type Error = Error;

    fn try_from(input: String) -> Result<Self, Self::Error> {
        input.as_str().try_into()
    }
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.value {
            PlaceValue::Header => write!(f, "."),
            PlaceValue::PathBuf(path) => write!(f, "file://{}", path.display()),
            PlaceValue::S3Uri(uri) => write!(f, "{}", uri),
            PlaceValue::SharePoint(url) => write!(f, "{}", url),
        }
    }
}

/// Represents the header row in Parquet manifest
#[derive(Clone, Debug, PartialEq)]
pub struct Header {
    // TODO: use `message` and `version` instead
    pub info: serde_json::Value, // system metadata
    pub meta: serde_json::Value, // user metadata
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
            place: Place::header(),
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
    pub place: Place,
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
        self.place.to_string()
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
            place: manifest_row.physical_key.try_into()?,
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
            place: attrs.object_uri.into(),
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
            place: "file://B/Ar".try_into()?,
            size: 123,
            hash: Multihash::wrap(345, b"hello world")?,
            info: serde_json::Value::Bool(false),
            meta: serde_json::json!({"foo":"bar"}),
        };
        assert_eq!(row.to_string(), r##"Row(Foo)@file://b/Ar^123#[104, 101, 108, 108, 111, 32, 119, 111, 114, 108, 100]$$Bool(false)$Object {"foo": String("bar")}"##.to_string());
        Ok(())
    }
}
