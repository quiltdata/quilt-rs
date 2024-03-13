use opendal::layers::LoggingLayer;
use opendal::services::S3;
use opendal::Operator;
use opendal::Result;

/// Use OpenDAL to interact with S3 and:
/// - Write a file
/// - Get the VersionID
/// - Write over that file
/// - Read the file with the first VersionID
/// - Write two multipart files concurrently

pub async fn bucket_operator(bucket: &str) -> Result<Operator> {
    let mut builder = S3::default();
    builder.bucket(bucket);
    builder.region("us-east-1");
    let op = Operator::new(builder)?
        .layer(LoggingLayer::default())
        .finish();
    Ok(op)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::utils::WRITE_BUCKET;
    use opendal::EntryMode;

    #[tokio::test]
    async fn test_s3_access() -> Result<()> {
        let op = bucket_operator(WRITE_BUCKET).await?;

        op.write("hello.txt", "Hello, World!").await?;
        let bs = op.read("hello.txt").await?;
        let meta = op.stat("hello.txt").await?;
        let mode = meta.mode();
        let length = meta.content_length();

        assert_eq!(bs.as_slice(), b"Hello, World!");
        assert_eq!(mode, EntryMode::FILE);
        assert_eq!(length, 13);

        op.delete("hello.txt").await?;

        Ok(())
    }
}
