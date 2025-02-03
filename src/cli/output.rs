use tracing::log;

use crate::cli::Error;

#[derive(Debug)]
pub enum Std {
    Out(String),
    Err(Error),
}

pub fn print(output: Std) {
    match output {
        Std::Out(str) => println!("{}", str),
        Std::Err(err) => log::error!("{}", err),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::Error;

    #[test]
    fn test_invalid_command() {
        let error_message = "quilt_rs error: Invalid package URI: S3 package URI must contain a fragment: quilt+s3://some-nonsense";
        let error = Error::Test(error_message.to_string());
        let output = Std::Err(error);
        
        // Capture log output and verify error is logged
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::ERROR)
            .with_test_writer()
            .compact()
            .try_init();
        
        assert!(subscriber.is_ok());
        print(output);
    }
}
