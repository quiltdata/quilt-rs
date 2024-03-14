mod cli;

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt::init();
    if let Err(err) = cli::init().await {
        tracing::error!("{}", err);
    }
}
