use std::path::PathBuf;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncRead;
use tokio::io::AsyncWrite;
use tokio::io::AsyncWriteExt;
use tokio::io::BufReader;
use tokio::io::BufWriter;

use crate::checksum::ContentHash;
use crate::manifest::Table;
use crate::Error;

pub type JsonObject = serde_json::Map<String, serde_json::Value>;

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ManifestHeader {
    pub version: String,
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] // Attempt to be quilt3-compatible.
    pub user_meta: Option<JsonObject>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct Quilt3ManifestRow {
    pub logical_key: PathBuf,
    pub physical_keys: Vec<String>,
    pub hash: ContentHash,
    // XXX: u64 cannot be safely deserialized by standard JS json parser,
    //      which treats numbers as 64-bit floats.
    //      However, having file size more than ~9PB (max safe/lossless integer - 53 bits)
    //      is quite unlikely ATM, given S3 limitations of 5TB per object.
    #[serde(deserialize_with = "number_to_u64")]
    pub size: u64,
    pub meta: Option<JsonObject>,
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

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct ManifestRow {
    pub logical_key: PathBuf,
    // XXX: use Url to have validated string?
    pub physical_key: String,
    pub hash: ContentHash,
    pub size: u64,
    pub meta: Option<JsonObject>,
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

#[derive(Debug, Deserialize, PartialEq, Serialize)]
pub struct Manifest {
    pub header: ManifestHeader,
    pub rows: Vec<ManifestRow>,
    // XXX: iterators?
}

impl Manifest {
    pub async fn from_reader<F: AsyncRead + Unpin + Send>(file: F) -> Result<Self, Error> {
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        let header = lines.next_line().await.map_err(|err| {
            Error::ManifestHeader(format!("Failed to read the manifest header: {}", err))
        })?;

        let Some(header) = header else {
            return Err(Error::ManifestHeader("Empty manifest".into()));
        };

        let header: ManifestHeader = serde_json::from_str(&header)?;

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

    pub async fn to_file<W: AsyncWrite + Unpin>(&self, file: W) -> Result<(), std::io::Error> {
        let mut writer = BufWriter::new(file);
        writer.write_all(self.to_jsonlines().as_bytes()).await?;
        writer.flush().await?;
        Ok(())
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
}

impl From<&Table> for Manifest {
    fn from(table: &Table) -> Self {
        Manifest {
            header: ManifestHeader {
                version: "v0".into(),
                message: table
                    .header
                    .info
                    .get("message")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string()),
                user_meta: table.header.meta.as_object().cloned(),
            },
            rows: table
                .records_values()
                .map(|row| {
                    let mut meta = match row.info.as_object() {
                        Some(meta) => meta.clone(),
                        None => serde_json::Map::default(),
                    };
                    if row.meta.is_object() {
                        meta.insert("user_meta".into(), row.meta.clone());
                    }
                    ManifestRow {
                        logical_key: row.name.clone(),
                        physical_key: row.place.clone(),
                        hash: row.hash.try_into().unwrap(),
                        size: row.size,
                        meta: Some(meta),
                    }
                })
                .collect(),
        }
    }
}

impl Default for Manifest {
    fn default() -> Self {
        Manifest {
            header: ManifestHeader {
                version: "v0".to_string(),
                message: Some("".to_string()),
                user_meta: None,
            },
            rows: Vec::new(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;

    use crate::manifest::Row;
    use crate::manifest::HEADER_ROW;

    #[test]
    fn test_equality_of_strictly_equal() {
        let left = ManifestRow {
            logical_key: PathBuf::from("A"),
            physical_key: "B".to_string(),
            hash: ContentHash::SHA256("C".to_string()),
            size: 1,
            meta: None,
        };
        let right = ManifestRow {
            logical_key: PathBuf::from("A"),
            physical_key: "B".to_string(),
            hash: ContentHash::SHA256("C".to_string()),
            size: 1,
            meta: None,
        };
        assert!(left == right)
    }

    #[test]
    fn test_equality_of_partialy_equal() {
        let mut meta = serde_json::Map::new();
        meta.insert("foo".to_string(), serde_json::json!("bar"));
        let left = ManifestRow {
            logical_key: PathBuf::from("A"),
            physical_key: "FOO".to_string(),
            hash: ContentHash::SHA256("C".to_string()),
            size: 1,
            meta: Some(meta),
        };
        let right = ManifestRow {
            logical_key: PathBuf::from("A"),
            physical_key: "BAR".to_string(),
            hash: ContentHash::SHA256("C".to_string()),
            size: 1,
            meta: None,
        };
        assert!(left == right)
    }

    #[test]
    fn test_manifest_from_table() -> Result<(), Error> {
        let default_table = Table::default();
        let default_manifest = Manifest::default();
        assert_eq!(Manifest::from(&default_table), default_manifest);

        let mut user_meta = serde_json::Map::new();
        user_meta.insert(
            "foo".to_string(),
            serde_json::Value::String("bar".to_string()),
        );
        let mut meta = serde_json::Map::new();
        meta.insert(
            "version".to_string(),
            serde_json::Value::String("v0".to_string()),
        );
        let mut info = serde_json::Map::new();
        info.insert(
            "message".to_string(),
            serde_json::Value::String("test".to_string()),
        );

        let table = Table::new(
            Row {
                name: HEADER_ROW.into(),
                place: HEADER_ROW.into(),
                size: 0,
                hash: multihash::Multihash::default(),
                info: serde_json::Value::Object(info),
                meta: serde_json::Value::Object(user_meta.clone()),
            },
            BTreeMap::from([(
                PathBuf::from("foo/bar"),
                Row {
                    name: PathBuf::from("foo/bar"),
                    place: "s3://z/x/y?versionId=foo".to_string(),
                    size: 123,
                    hash: multihash::Multihash::wrap(0xb510, b"abcdef")?,
                    info: serde_json::Value::Null,
                    meta: serde_json::Value::Object(meta.clone()),
                },
            )]),
        );
        let reference_manifest = Manifest {
            header: ManifestHeader {
                version: "v0".to_string(),
                message: Some("test".to_string()),
                // user_meta: Some(serde_json::json!({"foo": "bar"})),
                user_meta: Some(user_meta),
            },
            rows: vec![ManifestRow {
                logical_key: PathBuf::from("foo/bar"),
                physical_key: "s3://z/x/y?versionId=foo".to_string(),
                hash: ContentHash::SHA256Chunked("YWJjZGVm".to_string()),
                size: 123,
                meta: Some(meta),
            }],
        };
        assert_eq!(Manifest::from(&table), reference_manifest);
        Ok(())
    }
}
