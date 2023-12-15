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
    client::Client,
    namespace::Namespace,
    upath::UPath,
    table::Table,
    entry::Entry4,
};

#[derive(Clone, Debug)]
pub struct Manifest4 {
    _namespace: Namespace,
    table: Table,
    path: Option<UPath>,
}

impl Manifest4 {
    pub async fn new(_namespace: Namespace, table: Table, path: Option<UPath>) -> Self {
        Manifest4 {
            _namespace,
            table,
            path,
        }
    }

    pub fn to_string(&self) -> String {
        if self.path.is_some() {
            format!("Manifest4({})^{}", self.path.as_ref().unwrap().to_string(), self._namespace.to_string())
        } else {
            format!("Manifest4({})^{}", self.table.to_string(), self._namespace.to_string())
        }
    }

    #[allow(dead_code)]
    pub fn get_client(&self) -> &Client {
        self._namespace.get_client()
    }

    pub async fn entry_from_key(_entry: &str) -> Option<Entry4> {
        // TODO: Implement stub for entry_keys
        unimplemented!()
    }

    pub async fn entry_keys(&self) -> Vec<String> {
        // TODO: Implement stub for entry_keys
        unimplemented!()
    }

    pub async fn entry_objects(&self) -> Vec<Entry4> {
        // TODO: Implement stub for entry_objects
        unimplemented!()
    }
    
}