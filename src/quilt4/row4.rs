//!
//! # Row4
//! 
//! Row4 is the native entry format for quilt4.
//! It provides methods to decode/encode quilt3's JSONL format
//!

use multihash::Multihash;
use serde_json::Value as Json;
use std::collections::HashMap;

use super::upath::UPath;

pub struct Row3Hash {
    value: String,
    _type: String,
}
pub struct Row3 {
    logical_key: String,
    physical_key: Vec<String>,
    size: usize,
    hash: Row3Hash,
    meta: HashMap<String, Json>,
}
pub struct Row4 {
    name: String,
    place: String,
    path: Option<UPath>,
    size: usize,
    hash: Multihash<256>,
    info: HashMap<String, Json>,
    meta: HashMap<String, Json>,
}

impl Row4 {
    pub fn from_row3(row3: Row3) -> Self {
        // Implementation goes here
        unimplemented!()
    }

    pub fn to_row3(&self) -> Row3 {
        // Implementation goes here
        unimplemented!()
    }
}
