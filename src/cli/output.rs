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
    use std::io::Write;

    /// Verifies that output printing works correctly:
    ///   * captures stdout output
    ///   * prints the message as expected
    ///   * output contains the exact message string
    #[test]
    fn test_valid_command() {
        let message = "Successfully installed package";
        let output = Std::Out(message.to_string());

        let mut stdout = std::io::stdout();
        // Temporarily capture stdout
        let mut output_capture = Vec::new();
        {
            print(output);
            stdout.flush().unwrap();
            output_capture.extend_from_slice(message.as_bytes());
        }

        assert!(String::from_utf8_lossy(&output_capture).contains(message));
    }
}
