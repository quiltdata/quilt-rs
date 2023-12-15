//!
//! # Entry
//! 
//! Entry wraps a Row4 with a reference to its parent Manifest4.
//! It is the primary unit of data in a Manifest4.
//! 

use super::{
    manifest::Manifest4,
    row4::Row4,
};

#[derive(Clone, Debug)]
pub struct Entry4 {
    parent: Manifest4,
    row4: Row4,
}

impl Entry4 {   
    pub async fn new(parent: Manifest4, row4: Row4) -> Self {
        Entry4 {
            parent,
            row4,
        }
    }
    pub fn to_string(&self) -> String {
        format!("Entry4({})^{}", self.row4.to_string(), self.parent.to_string())
    }        

}
