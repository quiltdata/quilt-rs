use std::collections::BTreeMap;

use multihash::Multihash;
use serde::{de::Error as DeserializeError, Deserialize, Deserializer, Serialize, Serializer};

use crate::Error;

use super::RemoteManifest;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CommitState {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    pub hash: String,
    #[serde(default = "Vec::new")]
    pub prev_hashes: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathState {
    pub timestamp: chrono::DateTime<chrono::Utc>,
    #[serde(
        serialize_with = "multihash_to_str",
        deserialize_with = "str_to_multihash"
    )]
    pub hash: Multihash<256>,
}

fn multihash_to_str<S: Serializer>(
    hash: &Multihash<256>,
    serializer: S,
) -> Result<S::Ok, S::Error> {
    let s = hex::encode(hash.to_bytes());
    serializer.serialize_str(&s)
}

fn str_to_multihash<'de, D: Deserializer<'de>>(
    deserializer: D,
) -> Result<Multihash<256>, D::Error> {
    let s = String::deserialize(deserializer)?;
    let bytes = hex::decode(s).map_err(DeserializeError::custom)?;
    Multihash::from_bytes(&bytes).map_err(DeserializeError::custom)
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PackageLineage {
    pub commit: Option<CommitState>,
    pub remote: RemoteManifest,
    pub base_hash: String,
    pub latest_hash: String,
    // installed paths
    #[serde(default = "BTreeMap::new")]
    pub paths: BTreeMap<String, PathState>,
}

impl PackageLineage {
    pub fn from_remote(remote: RemoteManifest, latest_hash: String) -> Self {
        Self {
            base_hash: remote.hash.clone(),
            remote,
            latest_hash,
            commit: None,
            paths: BTreeMap::new(),
        }
    }

    pub fn current_hash(&self) -> &str {
        self.commit.as_ref().map_or(&self.remote.hash, |c| &c.hash)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct DomainLineage {
    #[serde(default = "BTreeMap::new")]
    pub packages: BTreeMap<String, PackageLineage>,
}

impl TryFrom<&str> for DomainLineage {
    type Error = Error;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        serde_json::from_str(input).map_err(Error::LineageParse)
    }
}

impl TryFrom<Vec<u8>> for DomainLineage {
    type Error = String;

    fn try_from(input: Vec<u8>) -> Result<Self, Self::Error> {
        let input_str = String::from_utf8_lossy(&input);
        let parsed: Self = serde_json::from_str(&input_str)
            .map_err(|err| format!("Failed to parse the lineage file: {}", err))?;
        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_syntax_error() {
        assert_eq!(
            DomainLineage::try_from("err").unwrap_err().to_string(),
            "Failed to parse lineage file: expected value at line 1 column 1".to_string()
        );
    }

    #[test]
    fn test_wrong_key() {
        // NOTE: @fiskus I don't think this is developer friendly
        //       I'd like to remove serde(default), so this test fails
        assert_eq!(
            DomainLineage::try_from(r#"{"notkey": 123}"#).unwrap(),
            DomainLineage {
                packages: BTreeMap::new(),
            }
        );
    }

    #[test]
    fn test_wrong_value() {
        assert!(DomainLineage::try_from(r#"{"packages": 123}"#)
            .unwrap_err()
            .to_string()
            .starts_with("Failed to parse lineage file: invalid type:"));
    }

    #[test]
    fn test_parsing_json_ok() {
        assert_eq!(
            DomainLineage::try_from(r###"{"packages":{}}"###).unwrap(),
            DomainLineage {
                packages: BTreeMap::new(),
            }
        )
    }
}
