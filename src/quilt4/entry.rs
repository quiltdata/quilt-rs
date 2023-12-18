//!
//! # Entry
//! 
//! Entry wraps a Row4 with a reference to its _manifest Manifest4.
//! It is the primary unit of data in a Manifest4.
//! 

use super::{
    manifest::Manifest4,
    row4::Row4,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Entry4 {
    _manifest: Manifest4,
    row4: Row4,
}

impl Entry4 {   
    pub async fn new(_manifest: Manifest4, row4: Row4) -> Self {
        Entry4 {
            _manifest,
            row4,
        }
    }
    pub fn to_string(&self) -> String {
        format!("Entry4({})^{}", self.row4.to_string(), self._manifest.to_string())
    }        
}
