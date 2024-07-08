//! Contains helpers and wrappers to work with IO.

pub mod manifest;
mod parquet;
/// It is public only for documentation and testing
pub mod remote;
/// It is public only for documentation and testing
pub mod storage;

mod entry;
pub use entry::get_relative_name;
pub use entry::RowUnmaterialized;
pub use parquet::ParquetWriter;
