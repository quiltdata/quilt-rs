//!
//! # Row4
//! 
//! Row4 is the native entry format for quilt4.
//! It provides methods to decode/encode quilt3's JSONL format
//!

// use multihash::Multihash;
use serde_json::Value as Json;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use aptos_openapi_link::impl_poem_type;
impl_poem_type!(Row4, "object", ());

use super::{
    upath::UPath,
    row3::Row3,
};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Row4 {
    name: String,
    // scheme: Enum<file,s3,https>
    place: String,
    path: Option<UPath>,
    size: usize,
    hash: String, // TODO: save as bytes versus encoded string
    info: HashMap<String, Json>, // system metadata
    meta: HashMap<String, Json>, // user metadata
}

impl Row4 {

    pub fn to_string(&self) -> String {
        let result = format!("Row4({})", self.name) +
        &format!("@{}", self.place) +
        &format!("^{:?}", self.size) +
        &format!("#{:?}", self.hash) +
        &format!("$${:?}", self.info) +
        &format!("${:?}", self.meta);
        if self.path.is_some() {
           result + &format!("${:?}", self.path)
        } else {
           result
        }
    }

    pub fn from_row3(_row3: Row3) -> Self {
        // Implementation goes here
        unimplemented!()
    }

    pub fn to_row3(&self) -> Row3 {
        // Implementation goes here
        unimplemented!()
    }
}
