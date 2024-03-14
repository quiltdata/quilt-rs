use std::collections::{BTreeMap, HashSet};

use base64::{prelude::BASE64_STANDARD, Engine};
use multihash::Multihash;
use serde::{Deserialize, Deserializer, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncWrite, AsyncWriteExt, BufReader, BufWriter};

use crate::Error;

use super::{Change, ChangeSet};

pub type JsonObject = serde_json::Map<String, serde_json::Value>;

#[derive(Debug, Deserialize, PartialEq, Eq, Serialize)]
pub struct ManifestHeader {
    pub version: String,
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] // Attempt to be quilt3-compatible.
    pub user_meta: Option<JsonObject>,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(tag = "type", content = "value")]
pub enum ContentHash {
    SHA256(String),
    #[serde(rename = "sha2-256-chunked")]
    SHA256Chunked(String),
}

pub const MULTIHASH_SHA256: u64 = 0x16;
pub const MULTIHASH_SHA256_CHUNKED: u64 = 0xb510;

impl TryFrom<Multihash<256>> for ContentHash {
    type Error = Error;

    fn try_from(value: Multihash<256>) -> Result<Self, Self::Error> {
        match value.code() {
            MULTIHASH_SHA256 => Ok(ContentHash::SHA256(hex::encode(value.digest()))),
            MULTIHASH_SHA256_CHUNKED => Ok(ContentHash::SHA256Chunked(
                BASE64_STANDARD.encode(value.digest()),
            )),
            code => Err(Error::InvalidMultihash(format!(
                "Unexpected code: {:#06x}",
                code
            ))),
        }
    }
}

impl TryInto<Multihash<256>> for ContentHash {
    type Error = Error;

    fn try_into(self) -> Result<Multihash<256>, Self::Error> {
        match self {
            ContentHash::SHA256(hash) => {
                let hash_bytes =
                    hex::decode(hash).map_err(|err| Error::InvalidMultihash(err.to_string()))?;
                Multihash::wrap(MULTIHASH_SHA256, &hash_bytes)
                    .map_err(|err| Error::InvalidMultihash(err.to_string()))
            }
            ContentHash::SHA256Chunked(hash) => {
                let hash_bytes = BASE64_STANDARD
                    .decode(hash)
                    .map_err(|err| Error::InvalidMultihash(err.to_string()))?;
                Multihash::wrap(MULTIHASH_SHA256_CHUNKED, &hash_bytes)
                    .map_err(|err| Error::InvalidMultihash(err.to_string()))
            }
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct Quilt3ManifestRow {
    pub logical_key: String,
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
    pub logical_key: String,
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
    pub async fn from_file<F: AsyncRead + Unpin + Send>(file: F) -> Result<Self, Error> {
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

    pub fn find_path(&self, path: impl AsRef<str>) -> Option<usize> {
        let path = path.as_ref();
        self.rows
            .binary_search_by(|row| row.logical_key.as_str().cmp(path))
            .ok()
    }

    pub fn get(&self, path: impl AsRef<str>) -> Option<&ManifestRow> {
        let idx = self.find_path(path)?;
        Some(&self.rows[idx])
    }

    pub fn get_mut(&mut self, path: impl AsRef<str>) -> Option<&mut ManifestRow> {
        let idx = self.find_path(path)?;
        Some(&mut self.rows[idx])
    }

    pub fn has_path(&self, path: impl AsRef<str>) -> bool {
        // TODO: handle directories
        self.find_path(path).is_some()
    }

    pub fn rows_map(&self) -> BTreeMap<String, ManifestRow> {
        self.rows
            .iter()
            .map(|row| (row.logical_key.clone(), row.to_owned()))
            .collect()
    }

    // other is "previous"
    pub fn diff_filtered(
        &self,
        other: &Self,
        keys: Option<&HashSet<String>>,
    ) -> ChangeSet<String, ManifestRow> {
        let mut changes = ChangeSet::new();

        let self_map = self.rows_map();
        let other_map = other.rows_map();

        let self_keys: HashSet<&String> = self_map.keys().collect();
        let other_keys: HashSet<&String> = self_map.keys().collect();
        let all_keys: HashSet<&String> = self_keys.union(&other_keys).cloned().collect();
        let all_keys: Vec<&String> = match keys {
            Some(keys) => all_keys.into_iter().filter(|k| keys.contains(*k)).collect(),
            None => all_keys.into_iter().collect(),
        };

        for k in all_keys {
            let self_row = self_map.get(k);
            let other_row = other_map.get(k);

            if match (self_row, other_row) {
                (Some(self_row), Some(other_row)) => self_row == other_row,
                (None, None) => true,
                _ => false,
            } {
                continue;
            }

            let change = Change {
                current: self_row.cloned(),
                previous: other_row.cloned(),
            };

            changes.insert(k.to_owned(), change);
        }

        changes
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_equality_of_strictly_equal() {
        let left = ManifestRow {
            logical_key: "A".to_string(),
            physical_key: "B".to_string(),
            hash: ContentHash::SHA256("C".to_string()),
            size: 1,
            meta: None,
        };
        let right = ManifestRow {
            logical_key: "A".to_string(),
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
            logical_key: "A".to_string(),
            physical_key: "FOO".to_string(),
            hash: ContentHash::SHA256("C".to_string()),
            size: 1,
            meta: Some(meta),
        };
        let right = ManifestRow {
            logical_key: "A".to_string(),
            physical_key: "BAR".to_string(),
            hash: ContentHash::SHA256("C".to_string()),
            size: 1,
            meta: None,
        };
        assert!(left == right)
    }
}
