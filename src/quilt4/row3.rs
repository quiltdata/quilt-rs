//!
//! This module contains the Row3 struct and its associated methods.
//! for importing and exporting quilt3's JSONL format.
//! 

use std::collections::HashMap;
use serde_json::Value as Json;
use serde::{Deserialize, Serialize};
use aptos_openapi_link::impl_poem_type;
impl_poem_type!(Row3Hash, "object", ());
impl_poem_type!(Row3, "object", ());

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Row3Hash {
    value: String,
    _type: String, // FIXME: This should be a HashType enum
}

impl Row3Hash {
    pub fn to_string(&self) -> String {
        format!("Row3Hash({})", self.value)
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

impl Row3 {
    pub fn to_string(&self) -> String {
        format!("Row3({})", self.logical_key) +
        &format!("@{}", self.physical_keys[0].to_string()) +
        &format!("^{}", self.size.to_string()) + 
        &format!("#{}", self.hash.to_string()) +
        &format!("${}", self.meta.len().to_string())
    }
}
