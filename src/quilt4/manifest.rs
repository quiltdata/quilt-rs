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
    table::Table,
    // entry::Entry4,
    upath::UPath,
};
#[derive(Clone, Debug)]
pub struct Manifest4 {
    path: UPath,
    table: Option<Table>,
}

impl Manifest4 {
    pub fn new(path: UPath, table: Option<Table>) -> Self {
        Manifest4 { path, table }
    }

    pub fn path(&self) -> &UPath {
        &self.path
    }

    pub fn table(&self) -> Option<&Table> {
        self.table.as_ref()
    }

    pub fn to_string(&self) -> String {
        format!("Manifest4({:?}, {:?})", self.path, self.table)
    }

    pub fn hash(&self) -> String {
        unimplemented!()
    }

    pub async fn write(&self, _path: UPath) {
        unimplemented!()
    }
}
