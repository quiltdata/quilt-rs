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
        
        // Setup test subscriber to capture logs
        let (subscriber, handle) = tracing_subscriber::reload::Layer::new(
            tracing_subscriber::fmt::layer()
                .with_test_writer()
                .with_filter(tracing_subscriber::filter::LevelFilter::ERROR)
        );
        
        let _guard = tracing::subscriber::set_default(
            tracing_subscriber::registry().with(subscriber)
        );

        // Execute the print function
        print(output);

        // Verify that exactly one ERROR level message was logged
        let events = handle.modifications();
        assert_eq!(events.len(), 1);
        assert!(events[0].contains(error_message));
    }
}
