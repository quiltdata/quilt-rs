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
    namespace::Namespace,
    upath::UPath,
    table::Table,
    entry::Entry4,
};
#[derive(Clone, Debug)]
pub struct Manifest4<'a> {
    _namespace: &'a Namespace<'a>,
    table: &'a Table,
    path: Option<UPath>,
}

impl<'a> Manifest4<'a> {
  pub fn new(_namespace: &'a Namespace, table: &'a Table, path: Option<UPath>) -> Self {
    Manifest4 {
      _namespace,
      table,
      path,
    }
  }

  pub fn to_string(&self) -> String {
    if self.path.is_some() {
      format!("Manifest4({:?})^{}", self.path, self._namespace.to_string())
    } else {
      format!("Manifest4({})^{}", self.table.to_string(), self._namespace.to_string())
    }
  }
}

