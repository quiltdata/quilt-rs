use clap::Parser;
use tracing::log;
use std::io::{self, Write};

mod cli;
mod perf;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let args = cli::Args::parse();
    let result = cli::init(args).await;
    
    let stdout = io::stdout();
    let stderr = io::stderr();
    let mut stdout_handle = stdout.lock();
    let mut stderr_handle = stderr.lock();
    
    if let Err(err) = cli::output::print(result, &mut stdout_handle, &mut stderr_handle) {
        log::error!("Failed to print output: {}", err);
    }
}
