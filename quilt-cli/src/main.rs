// `test_log`'s `#[test]` macro injects an init statement before the body, so it
// trips `items_after_statements` on leading `use`s/items in test fns. Enforce
// the lint in production; allow it only under `cfg(test)`.
#![cfg_attr(test, allow(clippy::items_after_statements))]

use clap::Parser;
use std::io;
use tracing::log;
use tracing_subscriber::filter::{EnvFilter, LevelFilter};

mod cli;

use cli::print;

#[tokio::main]
async fn main() {
    let args = cli::Args::parse();
    init_logging(args.verbose);
    match cli::init(args).await {
        Ok(result) => {
            let stdout = io::stdout();
            let stderr = io::stderr();
            let mut stdout_handle = stdout.lock();
            let mut stderr_handle = stderr.lock();

            if let Err(err) = print(result, &mut stdout_handle, &mut stderr_handle) {
                log::error!("Failed to print output: {err}");
                std::process::exit(1);
            }
        }
        Err(err) => {
            log::error!("Failed to run command: {err}");
            std::process::exit(1);
        }
    }
}

fn init_logging(verbose: bool) {
    let default_level = if verbose {
        LevelFilter::INFO
    } else {
        LevelFilter::WARN
    };
    let filter = EnvFilter::builder()
        .with_default_directive(default_level.into())
        .from_env_lossy();

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .init();
}
