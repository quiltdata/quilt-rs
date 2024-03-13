//!
//! This module contains the Row3 struct and its associated methods.
//! for importing and exporting quilt3's JSONL format.
//!

use serde::{Deserialize, Serialize};
use serde_json::Value as Json;
use std::{collections::HashMap, fmt};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Row3Hash {
    value: String,
    _type: String, // FIXME: This should be a HashType enum
}

impl fmt::Display for Row3Hash {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Row3Hash({})", self.value)
    }
}

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Row3 {
    logical_key: String,
    physical_keys: Vec<String>,
    size: usize,
    hash: Row3Hash,
    meta: HashMap<String, Json>,
}

impl fmt::Display for Row3 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let result = format!("Row3({})", self.logical_key)
            + &format!("@{}", self.physical_keys[0]) // TODO: what if vec is empty?
            + &format!("^{}", self.size)
            + &format!("#{}", self.hash)
            + &format!("${}", self.meta.len()); // TODO: print more useful info
        write!(f, "{}", result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_row3_hash_formatting() {
        let hash = Row3Hash {
            value: "Foo".to_string(),
            _type: "Bar".to_string(),
        };
        assert_eq!("Row3Hash(Foo)".to_string(), hash.to_string())
    }

    #[test]
    fn test_row3_formatting() {
        let row = Row3 {
            logical_key: "foo/bar".to_string(),
            physical_keys: vec!["one-is-mandatory".to_string()],
            size: 123,
            hash: Row3Hash {
                value: "foo".to_string(),
                _type: "bar".to_string(),
            },
            meta: HashMap::new(),
        };
        assert_eq!(
            row.to_string(),
            "Row3(foo/bar)@one-is-mandatory^123#Row3Hash(foo)$0"
        )
    }
}
