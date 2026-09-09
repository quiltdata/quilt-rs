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
            let failed = matches!(&result, cli::Std::Err(_));
            let stdout = io::stdout();
            let stderr = io::stderr();
            let mut stdout_handle = stdout.lock();
            let mut stderr_handle = stderr.lock();

            if let Err(err) = print(result, &mut stdout_handle, &mut stderr_handle) {
                log::error!("Failed to print output: {err}");
                std::process::exit(1);
            }

            if failed {
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
    let rust_log = std::env::var(EnvFilter::DEFAULT_ENV).ok();
    let filter = build_filter(rust_log.as_deref(), verbose);

    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(io::stderr)
        .init();
}

fn build_filter(env_value: Option<&str>, verbose: bool) -> EnvFilter {
    let default_level = if verbose {
        LevelFilter::INFO
    } else {
        LevelFilter::WARN
    };
    let builder = EnvFilter::builder().with_default_directive(default_level.into());

    match env_value {
        Some(value) => builder.parse_lossy(value),
        None => builder.parse_lossy(""),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn build_filter_uses_info_default_when_verbose_without_rust_log() {
        let filter = build_filter(None, true);

        assert_eq!(filter.max_level_hint(), Some(LevelFilter::INFO));
    }

    #[test]
    fn build_filter_lets_rust_log_override_verbose_default() {
        let filter = build_filter(Some("warn"), true);

        assert_eq!(filter.max_level_hint(), Some(LevelFilter::WARN));
    }
}
