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
    use mockall::automock;
    use mockall::predicate::*;

    #[automock]
    trait Logger {
        fn error(&self, message: &str);
    }

    #[test]
    fn test_invalid_command() {
        let error_message = "quilt_rs error: Invalid package URI: S3 package URI must contain a fragment: quilt+s3://some-nonsense";
        let error = Error::Test(error_message.to_string());
        let output = Std::Err(error);
        
        let mut mock_logger = MockLogger::new();
        mock_logger
            .expect_error()
            .with(eq(error_message))
            .times(1)
            .return_const(());

        // Replace log::error with our mock during the test
        let _guard = mockall::mock_guard::MockGuard::new()
            .expect_log_error(mock_logger.error);

        print(output);
    }
}
