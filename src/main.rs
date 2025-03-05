use clap::Parser;
use tracing::log;

mod cli;
mod perf;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let args = cli::Args::parse();
    let result = cli::init(args).await;
    if let Err(err) = cli::output::print(result) {
        log::error!("Failed to print output: {}", err);
    }
}
