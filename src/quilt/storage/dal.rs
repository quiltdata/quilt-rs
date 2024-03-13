/// Use OpenDAL to interact with S3 and:
/// - Write a file
/// - Get the VersionID
/// - Write over that file
/// - Read the file with the first VersionID
/// - Write two multipart files concurrently



#[cfg(test)]
mod tests {
    use opendal::Result;
    use opendal::layers::LoggingLayer;
    use opendal::services;
    use opendal::Operator;
    
    #[tokio::test]
    async fn test_s3_access() -> Result<()> {
        let mut builder = services::S3::default();
        builder.bucket("test");

        let op = Operator::new(builder)?
            .layer(LoggingLayer::default())
            .finish();

        op.write("hello.txt", "Hello, World!").await?;
        let bs = op.read("hello.txt").await?;
        let meta = op.stat("hello.txt").await?;
        let mode = meta.mode();
        let length = meta.content_length();

        op.delete("hello.txt").await?;

        Ok(())
    }
}
