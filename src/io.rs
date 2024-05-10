//! Contains helpers and wrappers to work with IO. 

pub mod manifest;
mod parquet;
/// It is public only for documentation and testing
pub mod remote;
/// It is public only for documentation and testing
pub mod storage;

pub use parquet::ParquetWriter;
