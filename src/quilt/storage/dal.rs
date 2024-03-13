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
    use opendal::{EntryMode, Metakey};

    #[tokio::test]
    async fn test_s3_write() -> Result<()> {
        const TEST_DIR: &str = "test_dal/";
        const TEST_FILE: &str = "hello.txt";
        let test_path = format!("{}{}", TEST_DIR, TEST_FILE);
        const TEST_BYTES: &[u8] = b"Hello, World!";

        let op = bucket_operator(WRITE_BUCKET).await?;
        op.write(test_path.as_str(), TEST_BYTES).await?;

        let entries = op.list_with(TEST_DIR).metakey(Metakey::Version).await?;
        for entry in entries {
            println!("entry: {:?}", entry.path());
            let meta = entry.metadata().metakey();
            println!("meta: {:?}", meta);
        }

        let bs = op.read("hello.txt").await?;

        let meta = op.stat_with(path)
        println!("{:?}", meta);
        let mode = meta.mode();
        let version = meta.version();
        let length = meta.content_length();

        assert_eq!(bs.as_slice(), b"Hello, World!");
        assert_eq!(mode, EntryMode::FILE);
        assert_eq!(length, 13);
        assert!(version.is_some());
        assert_eq!(version.unwrap().len(), 32);

        // op.delete("hello.txt").await?;

        Ok(())
    }
}
