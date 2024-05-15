use tracing::log;

mod cli;
mod perf;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    if let Err(err) = cli::init().await {
        log::error!("{}", err);
    }
}
