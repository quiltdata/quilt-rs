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

use crate::io::manifest::StreamRowsChunk;
use crate::manifest::Row;
use crate::Error;
use crate::Res;

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

fn create_columns(chunk: StreamRowsChunk) -> Res<Vec<ArrayRef>> {
    let mut names = Vec::new();
    let mut places = Vec::new();
    let mut sizes = Vec::new();
    let mut hashes = Vec::new();
    let mut metas = Vec::new();
    let mut infos = Vec::new();
    for row_result in chunk {
        let manifest_row = row_result?;
        let row = Row::from(manifest_row);
        names.push(row.display_name());
        places.push(row.display_place());
        hashes.push(row.display_hash());
        sizes.push(row.display_size());
        metas.push(row.display_meta()?);
        infos.push(row.display_info()?);
    }
    let name = GenericByteArray::<datatypes::Utf8Type>::from(names);
    let place = GenericByteArray::<datatypes::Utf8Type>::from(places);
    let size = UInt64Array::from(sizes);
    let hash = GenericByteArray::<datatypes::BinaryType>::from(
        hashes.iter().map(|h| h.as_slice()).collect::<Vec<_>>(),
    );
    let meta = GenericByteArray::<datatypes::Utf8Type>::from(metas);
    let info = GenericByteArray::<datatypes::Utf8Type>::from(infos);
    Ok(vec![
        Arc::new(name),
        Arc::new(place),
        Arc::new(size),
        Arc::new(hash),
        Arc::new(meta),
        Arc::new(info),
    ])
}

fn create_writer(file: File, schema: Arc<Schema>) -> Res<AsyncArrowWriter<File>> {
    let props = WriterProperties::builder()
        .set_compression(Compression::SNAPPY)
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
    pub async fn flush(self) -> Res {
        self.writer.close().await?;
        Ok(())
    }

    /// Insert rows chunk
    pub async fn insert(&mut self, chunk: StreamRowsChunk) -> Res {
        let columns = create_columns(chunk)?;
        let batch = RecordBatch::try_new(self.schema.clone(), columns)?;
        Ok(self.writer.write(&batch).await?)
    }
}
