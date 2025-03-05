use std::io::Write;
use crate::cli::Error;

#[derive(Debug)]
pub enum Std {
    Out(String),
    Err(Error),
}

pub fn print(output: Std) {
    let stdout = std::io::stdout();
    let stderr = std::io::stderr();
    
    match output {
        Std::Out(str) => {
            let mut handle = stdout.lock();
            writeln!(handle, "{}", str).expect("Failed to write to stdout");
        }
        Std::Err(err) => {
            let mut handle = stderr.lock();
            writeln!(handle, "{}", err).expect("Failed to write to stderr");
        }
    }
}
