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
