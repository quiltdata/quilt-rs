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

pub struct Manifest4 {
    path: Option<UPath>,
    table: Table4,
}

impl Manifest4 {
    pub async fn new(parent: Domain, path: Option<UPath>) -> Self {
        Namespace {
            parent,
            path,
        }
    }

    pub async fn entry_from_key(pkg_name: &str) -> Option<Entry4> {
        // TODO: Implement stub for entry_keys
        unimplemented!()
    }

    pub async fn entry_keys(&self) -> Vec<String> {
        // TODO: Implement stub for entry_keys
        unimplemented!()
    }

    pub async fn entry_objects(&self, entry: &str) -> Vec<Entry4> {
        // TODO: Implement stub for entry_objects
        unimplemented!()
    }
    
}