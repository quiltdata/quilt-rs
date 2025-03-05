use tracing::log;

mod cli;
mod perf;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    let args = cli::Args::parse();
    if let Err(err) = cli::init(args).await {
        log::error!("{}", err);
    }
}
