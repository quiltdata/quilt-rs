#[derive(Debug)]
pub enum Std {
    Out(String),
    Err(String),
}

pub fn print(output: Std) {
    match output {
        Std::Out(str) => tracing::info!("{}", str),
        Std::Err(str) => tracing::error!(str),
    }
}
