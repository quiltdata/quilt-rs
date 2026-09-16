// `test_log`'s `#[test]` macro injects an init statement before the body, so it
// trips `items_after_statements` on leading `use`s/items in test fns. Enforce
// the lint in production; allow it only under `cfg(test)`.
#![cfg_attr(test, allow(clippy::items_after_statements))]

use clap::Parser;
use std::io;
use tracing::log;
use tracing_subscriber::filter::{EnvFilter, LevelFilter};

mod cli;

use cli::Args;
use cli::Error;
use cli::Format;
use cli::Std;
use cli::print;

#[tokio::main]
async fn main() {
    let args = Args::parse();
    init_logging(args.verbose);
    let format = format_from_args(&args);

    // An error raised before dispatch — an unreadable domain, a rejected flag
    // combination — is a command failure like any other. It used to go out as a
    // tracing line, which under `--json` would hand a consumer prose where it
    // expects an object.
    let result = to_std(cli::init(args).await);

    let failed = matches!(&result, Std::Err(_));
    let stdout = io::stdout();
    let stderr = io::stderr();
    let mut stdout_handle = stdout.lock();
    let mut stderr_handle = stderr.lock();

    if let Err(err) = print(result, format, &mut stdout_handle, &mut stderr_handle) {
        log::error!("Failed to print output: {err}");
        std::process::exit(1);
    }

    if failed {
        std::process::exit(1);
    }
}

/// Which format a run uses, decided once from the global `--json` flag before
/// `args` is consumed by [`cli::init`].
fn format_from_args(args: &Args) -> Format {
    if args.json {
        Format::Json
    } else {
        Format::Text
    }
}

/// Route `init`'s pre-dispatch failure into the same [`Std::Err`] shape a
/// command's own failure takes, so `--json` sees an object on either path.
/// Pulled out of `main` so a future tidy-up that restores the old
/// `log::error!` arm here fails a test instead of shipping silently.
fn to_std(result: Result<Std, Error>) -> Std {
    match result {
        Ok(result) => result,
        Err(err) => Std::Err(err),
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

    #[test]
    fn format_from_args_is_json_only_when_flag_is_set() {
        let json = Args::parse_from(["quilt", "--json", "list"]);
        assert_eq!(format_from_args(&json), Format::Json);

        let text = Args::parse_from(["quilt", "list"]);
        assert_eq!(format_from_args(&text), Format::Text);
    }

    /// The regression this guards: a pre-dispatch failure used to print as a
    /// tracing line instead of routing through [`Std::Err`] like every other
    /// command failure. A tidy-up that brings that arm back would fail this.
    #[test]
    fn to_std_routes_pre_dispatch_error_the_same_as_a_command_error() {
        assert!(matches!(
            to_std(Err(Error::Domain)),
            Std::Err(Error::Domain)
        ));
    }

    #[test]
    fn to_std_passes_a_successful_init_through_unchanged() {
        assert!(matches!(
            to_std(Ok(Std::Err(Error::Home))),
            Std::Err(Error::Home)
        ));
    }
}
