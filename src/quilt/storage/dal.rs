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
    println!("bucket_operator: {}", bucket);
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

    const TEST_DIR: &str = "test_dal/";
    const TEST_FILE: &str = "hello.txt";
    const TEST_BYTES: &[u8] = b"Hello, World!";
    const TEST_VERSION: &str = "jM7lU7jFitCoGPbUdoW.L5vmxbtIDhsU";

    #[tokio::test]
    async fn test_s3_write() -> Result<()> {
        let op = bucket_operator(WRITE_BUCKET).await?;
        let test_path = format!("{}{}", TEST_DIR, TEST_FILE);
        op.write(test_path.as_str(), TEST_BYTES).await?;

        let bs = op.read(test_path.as_str()).await?;
        assert_eq!(bs.as_slice(), b"Hello, World!");

        let meta = op.stat(test_path.as_str()).await?;
        println!("{:?}", meta);
        let mode = meta.mode();
        let length = meta.content_length();
        assert_eq!(mode, EntryMode::FILE);
        assert_eq!(length, 13);
        
        //op.delete("hello.txt").await?;

        Ok(())
    }

    #[tokio::test]
    async fn test_s3_read() -> Result<()> {
        let op = bucket_operator(WRITE_BUCKET).await?;
        let test_path = format!("{}{}", TEST_DIR, TEST_FILE);
        let bs = op.read_with(test_path.as_str()).version(TEST_VERSION).await?;
        assert_eq!(bs.as_slice(), b"Hello, World!");
        Ok(())
    }
}
