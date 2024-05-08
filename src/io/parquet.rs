use std::sync::Arc;

use arrow::array::GenericByteArray;
use arrow::array::UInt64Array;
use arrow::datatypes;
use arrow::datatypes::Schema;
use arrow::error::ArrowError;
use arrow::record_batch::RecordBatch;
use parquet::arrow::AsyncArrowWriter;
use parquet::basic::Compression;
use parquet::file::properties::WriterProperties;
use tokio::fs::File;

use crate::manifest::Row;
// use crate::manifest::Table;
use crate::Error;

async fn write_row(
    writer: &mut AsyncArrowWriter<File>,
    schema: Arc<Schema>,
    row: &Row,
) -> Result<(), ArrowError> {
    let hash: &[u8] = &row.hash.to_bytes();
    let batch = RecordBatch::try_new(
        schema,
        vec![
            Arc::new(GenericByteArray::<datatypes::Utf8Type>::from(vec![row
                .name
                .display()
                .to_string()])),
            Arc::new(GenericByteArray::<datatypes::Utf8Type>::from(vec![row
                .place
                .as_str()])),
            Arc::new(UInt64Array::from(vec![row.size])),
            Arc::new(GenericByteArray::<datatypes::BinaryType>::from(vec![hash])),
            Arc::new(GenericByteArray::<datatypes::Utf8Type>::from(vec![
                serde_json::to_string(&row.meta).unwrap(),
            ])),
            Arc::new(GenericByteArray::<datatypes::Utf8Type>::from(vec![
                serde_json::to_string(&row.info).unwrap(),
            ])),
        ],
    )?;
    writer.write(&batch).await?;
    Ok(())
}

pub struct ParquetWriter {
    schema: Arc<Schema>,
    writer: AsyncArrowWriter<File>,
}
fn create_schema() -> Arc<Schema> {
    Arc::new(Schema::new(vec![
        datatypes::Field::new("name", datatypes::DataType::Utf8, false),
        datatypes::Field::new("place", datatypes::DataType::Utf8, false),
        datatypes::Field::new("size", datatypes::DataType::UInt64, false),
        datatypes::Field::new("multihash", datatypes::DataType::Binary, false),
        datatypes::Field::new("meta.json", datatypes::DataType::Utf8, false),
        datatypes::Field::new("info.json", datatypes::DataType::Utf8, false),
    ]))
}

fn create_writer(file: File, schema: Arc<Schema>) -> Result<AsyncArrowWriter<File>, Error> {
    let props = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
        .build();
    Ok(AsyncArrowWriter::try_new(file, schema, Some(props))?)
}

impl TryFrom<File> for ParquetWriter {
    type Error = Error;

    fn try_from(file: File) -> Result<Self, Self::Error> {
        let schema = create_schema();
        let writer = create_writer(file, schema.clone())?;
        Ok(ParquetWriter { schema, writer })
    }
}

impl ParquetWriter {
    // pub async fn write_to_path(&self, storage: &impl Storage, path: &PathBuf) -> Result<(), Error> {
    //     let mut writer = self
    //         .create_writer_from_path(storage, path, self.schema.clone())
    //         .await?;

    //     Self::write_row_impl(&mut writer, self.schema.clone(), &self.header).await?;
    //     for row in self.records.values() {
    //         Self::write_row_impl(&mut writer, self.schema.clone(), row).await?;
    //     }
    //     writer.close().await?;

    //     Ok(())
    // }
    // pub async fn write_manifest(table: &Table) -> Result<(), Error> {
    //     Ok(())
    // }

    pub async fn flush(self) -> Result<(), Error> {
        self.writer.close().await?;
        Ok(())
    }

    pub async fn insert_row(&mut self, row: Row) -> Result<(), Error> {
        write_row(&mut self.writer, self.schema.clone(), &row).await?;

        Ok(())
    }
}
