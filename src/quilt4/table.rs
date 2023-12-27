//!
//! Table4 is a wrapper for arrow-rs's Table, the native Manifest format for quilt4.
//! It uses UPath to transparently read and write to/from local and remote filesystems,
//! and provides methods to read/write (decode/encode) quilt3's JSONL format
//!

use arrow::error::ArrowError;
use arrow::record_batch::RecordBatch;
use parquet::arrow::arrow_reader::ParquetRecordBatchReaderBuilder;

use super::{row4::Row4, upath::UPath};
use serde::{Deserialize, Serialize};

const HEADER_ROW: &str = ".";

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct Table {
    #[serde(skip)]
    records: Vec<RecordBatch>, // Vec<RecordBatch>? DataFusion?
    path3: Option<UPath>,
    path4: Option<UPath>,
}

impl Table {
    pub fn new(path: Option<UPath>) -> Self {
        Table {
            records: vec![],
            path3: None,
            path4: path.clone(),
        }
    }
    pub fn to_string(&self) -> String {
        format!("Table({})", self.path4.as_ref().unwrap().to_string())
            + &format!("({:?})\n", self.path3)
            + &format!("[\n{:?}\n]", self.records)
    }
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
        let path = self
            .path4
            .as_ref()
            .ok_or(ArrowError::NotYetImplemented("only path4 supported".into()))?
            .file_path
            .as_ref()
            .ok_or(ArrowError::NotYetImplemented(
                "only file_path supported".into(),
            ))?;
        let file = std::fs::File::open(&path)?;
        let mut reader_stream = ParquetRecordBatchReaderBuilder::try_new(file)?.build()?;

        let mut records = vec![];
        while let Some(item) = reader_stream.next() {
            records.push(item?);
        }

        Ok(Self {
            records,
            path3: self.path3.clone(),
            path4: self.path4.clone(),
        })
    }

    // Write quilt4's Parquet format
    pub fn write4(&self) -> Result<(), ArrowError> {
        // Implementation goes here
        unimplemented!()
    }

    // Get a row from the table
    pub fn get_row(&self, _name: &str) -> Option<Row4> {
        // Implementation goes here
        unimplemented!()
    }

    pub fn get_header(&self) -> Option<Row4> {
        self.get_row(&HEADER_ROW.to_string())
    }
    // TBD: Store header metadata as PARQUET Metadata?

    pub fn list_names(&self) -> Vec<Row4> {
        // Implementation goes here
        unimplemented!()
    }
}

#[cfg(test)]
mod tests {
    use utils::local_uri_parquet;

    use super::*;

    #[test]
    fn read_existing() {
        let table = Table::new(Some(UPath::new(&local_uri_parquet())));
        let new_table = table.read4().unwrap();
        dbg!(&new_table);
        assert!(new_table.records.len() == 1);
        assert!(new_table.records[0].num_rows() == 2);
    }
}
