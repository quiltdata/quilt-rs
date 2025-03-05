use clap::Parser;
use std::io;
use tracing::log;

mod cli;
mod perf;

use cli::print;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let args = cli::Args::parse();
    match cli::init(args).await {
        Ok(result) => {
            let stdout = io::stdout();
            let stderr = io::stderr();
            let mut stdout_handle = stdout.lock();
            let mut stderr_handle = stderr.lock();

            if let Err(err) = print(result, &mut stdout_handle, &mut stderr_handle) {
                log::error!("Failed to print output: {}", err);
                std::process::exit(1);
            }
        }
        Err(err) => {
            log::error!("Failed to run command: {}", err);
            std::process::exit(1);
        }
    }
}
