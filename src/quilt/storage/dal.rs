/// Use OpenDAL to interact with S3 and:
/// - Write a file
/// - Get the VersionID
/// - Write over that file
/// - Read the file with the first VersionID
/// - Write two multipart files concurrently



#[cfg(test)]
mod tests {
    use opendal::EntryMode;
    use opendal::Result;
    use opendal::layers::LoggingLayer;
    use opendal::services::S3;
    use opendal::Operator;

    #[tokio::test]


    async fn test_s3_access() -> Result<()> {
        use crate::utils::{WRITE_BUCKET, TEST_REGION};
        let mut builder = S3::default();
        // S3::detect_region(AWS_ENDPOINT, TEST_BUCKET).await?;
        // const AWS_ENDPOINT: &str = "https://s3.amazonaws.com";
        builder.bucket(WRITE_BUCKET);
        builder.region(&TEST_REGION);

        let op = Operator::new(builder)?
            .layer(LoggingLayer::default())
            .finish();

        op.write("hello.txt", "Hello, World!").await?;
        let bs = op.read("hello.txt").await?;
        let meta = op.stat("hello.txt").await?;
        let mode = meta.mode();
        let length = meta.content_length();

        assert_eq!(bs.as_slice(), b"Hello, World!");car
        assert_eq!(mode, EntryMode::FILE);
        assert_eq!(length, 13);

        op.delete("hello.txt").await?;

        Ok(())
    }
}
