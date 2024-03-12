//!
//! # Row4
//!
//! Row4 is the native entry format for quilt4.
//! It provides methods to decode/encode quilt3's JSONL format
//!

use multihash::Multihash;
use std::fmt;

use super::{row3::Row3, upath::UPath};

#[derive(Clone, Debug, PartialEq)]
pub struct Row4 {
    pub name: String,
    // scheme: Enum<file,s3,https>
    pub place: String,
    pub path: Option<UPath>,
    pub size: u64,
    pub hash: Multihash<256>,
    pub info: serde_json::Value, // system metadata
    pub meta: serde_json::Value, // user metadata
}

impl Row4 {
    pub fn from_row3(_row3: Row3) -> Self {
        // Implementation goes here
        unimplemented!()
    }

    pub fn to_row3(&self) -> Row3 {
        // Implementation goes here
        unimplemented!()
    }
}

impl fmt::Display for Row4 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let result = format!("Row4({})", self.name)
            + &format!("@{}", self.place)
            + &format!("^{:?}", self.size)
            + &format!("#{:?}", self.hash)
            + &format!("$${:?}", self.info)
            + &format!("${:?}", self.meta);
        if self.path.is_some() {
            write!(f, "{}", result + &format!("${:?}", self.path))
        } else {
            write!(f, "{}", result)
        }
    }
}
