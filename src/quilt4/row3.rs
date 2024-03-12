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
            + &format!("@{}", self.physical_keys[0])
            + &format!("^{}", self.size)
            + &format!("#{}", self.hash)
            + &format!("${}", self.meta.len());
        write!(f, "{}", result)
    }
}
