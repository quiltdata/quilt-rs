use crate::cli::Error;
use std::io::Write;

#[derive(Debug)]
pub enum Std {
    Out(String),
    Err(Error),
}

pub fn print(
    output: Std,
    stdout: &mut impl Write,
    stderr: &mut impl Write,
) -> Result<(), std::io::Error> {
    match output {
        Std::Out(str) => writeln!(stdout, "{}", str)?,
        Std::Err(err) => writeln!(stderr, "{}", err)?,
    }
    Ok(())
}
