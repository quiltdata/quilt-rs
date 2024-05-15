use std::sync::Arc;

use arrow::array::ArrayRef;
use arrow::array::GenericByteArray;
use arrow::array::UInt64Array;
use arrow::datatypes;
use arrow::datatypes::DataType;
use arrow::datatypes::Field;
use arrow::datatypes::Schema;
use arrow::record_batch::RecordBatch;
use parquet::arrow::AsyncArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use tokio::fs::File;

use crate::manifest::Row;
use crate::Error;

/// Don't use it. It will be private
pub struct ParquetWriter {
    schema: Arc<Schema>,
    writer: AsyncArrowWriter<File>,
}

fn create_schema() -> Schema {
    Schema::new(vec![
        Field::new("name", DataType::Utf8, false),
        Field::new("place", DataType::Utf8, false),
        Field::new("size", DataType::UInt64, false),
        Field::new("multihash", DataType::Binary, false),
        Field::new("meta.json", DataType::Utf8, false),
        Field::new("info.json", DataType::Utf8, false),
    ])
}

fn create_columns_from_row(row: &Row) -> Result<Vec<ArrayRef>, Error> {
    Ok(vec![
        Arc::new(GenericByteArray::<datatypes::Utf8Type>::from(vec![
            row.display_name()
        ])),
        Arc::new(GenericByteArray::<datatypes::Utf8Type>::from(vec![
            row.display_place()
        ])),
        Arc::new(UInt64Array::from(vec![row.display_size()])),
        Arc::new(GenericByteArray::<datatypes::BinaryType>::from(vec![row
            .display_hash()
            .as_slice()])),
        Arc::new(GenericByteArray::<datatypes::Utf8Type>::from(vec![
            row.display_meta()?
        ])),
        Arc::new(GenericByteArray::<datatypes::Utf8Type>::from(vec![
            row.display_info()?
        ])),
    ])
}

fn create_columns(rows: Vec<Row>) -> Result<Vec<ArrayRef>, Error> {
    let mut names = Vec::new();
    let mut places = Vec::new();
    let mut sizes = Vec::new();
    let mut hashes = Vec::new();
    let mut metas = Vec::new();
    let mut infos = Vec::new();
    for row in rows {
        names.push(row.display_name());
        places.push(row.display_place());
        hashes.push(row.display_hash());
        sizes.push(row.display_size());
        metas.push(row.display_meta()?);
        infos.push(row.display_info()?);
    }
    Ok(vec![
        Arc::new(GenericByteArray::<datatypes::Utf8Type>::from(names)),
        Arc::new(GenericByteArray::<datatypes::Utf8Type>::from(places)),
        Arc::new(UInt64Array::from(sizes)),
        Arc::new(GenericByteArray::<datatypes::BinaryType>::from(hashes.iter().map(|h|h.as_slice()).collect::<Vec<_>>())),
        Arc::new(GenericByteArray::<datatypes::Utf8Type>::from(metas)),
        Arc::new(GenericByteArray::<datatypes::Utf8Type>::from(infos)),
    ])
}

fn create_writer(file: File, schema: Arc<Schema>) -> Result<AsyncArrowWriter<File>, Error> {
    let props = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .set_max_row_group_size(1024)
        .build();
    Ok(AsyncArrowWriter::try_new(file, schema, Some(props))?)
}

impl TryFrom<File> for ParquetWriter {
    type Error = Error;

    fn try_from(file: File) -> Result<Self, Self::Error> {
        let schema = Arc::new(create_schema());
        let writer = create_writer(file, schema.clone())?;
        Ok(ParquetWriter { schema, writer })
    }
}

impl ParquetWriter {
    /// Close and finalize the writer.
    pub async fn flush(self) -> Result<(), Error> {
        self.writer.close().await?;
        Ok(())
    }

    // TODO: add support for Vec<Row>
    pub async fn insert(&mut self, row: Row) -> Result<(), Error> {
        let columns = create_columns_from_row(&row)?;
        let batch = RecordBatch::try_new(self.schema.clone(), columns)?;
        Ok(self.writer.write(&batch).await?)
    }

    pub async fn insert_rows(&mut self, rows: Vec<Row>) -> Result<(), Error> {
        let columns = create_columns(rows)?;
        let batch = RecordBatch::try_new(self.schema.clone(), columns)?;
        Ok(self.writer.write(&batch).await?)
    }
}
