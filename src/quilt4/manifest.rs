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
use std::fmt;

use super::table::Table;

#[derive(Clone, Debug)]
pub struct Manifest4 {
    table: Table,
}

impl Manifest4 {
    pub fn new(table: Table) -> Self {
        Manifest4 { table }
    }

    pub fn hash(&self) -> String {
        unimplemented!()
    }

    pub async fn write4(&self) {
        unimplemented!()
    }
}

impl fmt::Display for Manifest4 {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "Manifest4({})", self.table)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::collections::BTreeMap;
    use std::path::PathBuf;

    use crate::quilt4::row4::Row4;

    #[test]
    fn test_manifest_formatting_default() {
        let manifest = Manifest4::new(Table::default());
        assert_eq!(format!("{}", manifest), "Manifest4(Table({}))");
    }

    #[test]
    fn test_manifest_formatting_with_records() {
        let manifest = Manifest4::new(Table {
            records: BTreeMap::from([(PathBuf::from("foo"), Row4::default())]),
            ..Table::default()
        });
        assert_eq!(
            format!("{}", manifest),
            format!(
                r###"Manifest4(Table({{"foo": "Row4(.)@.^0#[]$$Object {}$Null"}}))"###,
                r###"{\"message\": String(\"\"), \"version\": String(\"v0\")}"###
            )
        );
    }
}
