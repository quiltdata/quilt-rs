//!
//! # Manifest
//! 
//! Manifest wraps a Table4 containing an Arrow Table,
//! which is usually associated with a UPath.
//! It presents itself as a list of Entries,
//! the first of which is the Header.
//! 
//! NOTE: The names Manifest4/Entry4 are temporary, to avoid confusion
//! with the existing Manifest/Entry types.
//! Before 1.0, they will be renamed to Manifest/Entry
//! and the existing types will be obsoleted.
//! 

use super::{
    upath::UPath,
    table::Table,
    // entry::Entry4,
};
#[derive(Clone, Debug)]
pub struct Manifest4 {
    table: Table,
}

impl Manifest4 {
  pub fn new(table: Table) -> Self {
    Manifest4 {
      table,
    }
  }

  pub fn to_string(&self) -> String {
    format!("Manifest4({})", self.table.to_string())
  }

  pub fn hash(&self) -> String {
    unimplemented!()
  }

  pub async fn write4(&self, _path: UPath) {
    unimplemented!()
  }
}

