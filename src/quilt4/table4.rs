//!
//! Table4 is a wrapper for arrow-rs's Table, the native Manifest format for quilt4.
//! It uses UPath to transparently read and write to/from local and remote filesystems,
//! and provides methods to read/write (decode/encode) quilt3's JSONL format
//! 

use arrow::error::ArrowError; 

use super::{
    upath::UPath,
    row4::Row4,
};

static HEADER_ROW: String = String::from(".");

pub struct Table {
    records: Vec<Row4>,
    path3: Option<UPath>,
    path4: Option<UPath>,
}

impl Table {
    // Read quilt3's JSONL format
    pub fn read3(&self) -> Result<Self, ArrowError> {
        // Implementation goes here
        unimplemented!()
    }

    // Write quilt3's JSONL format
    pub fn write3(&self) -> Result<(), ArrowError> {
        // Implementation goes here
        unimplemented!()
    }

    // Read quilt4's Parquet format
    pub fn read4(&self) -> Result<Self, ArrowError> {
        // Implementation goes here
        unimplemented!()
    }

    // Write quilt4's Parquet format
    pub fn write4(&self) -> Result<(), ArrowError> {
        // Implementation goes here
        unimplemented!()
    }

    // Get a row from the table
    pub fn get_row(&self, name: &String) -> Option<Row4> {
        // Implementation goes here
        unimplemented!()
    }

    pub fn get_header(&self) -> Option<Row4> {
        self.get_row(&HEADER_ROW)
    }

    pub fn list_rows(&self) -> Vec<Row4> {
        // Implementation goes here
        unimplemented!()
    }
}
