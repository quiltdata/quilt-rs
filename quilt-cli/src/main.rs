// `test_log`'s `#[test]` macro injects an init statement before the body, so it
// trips `items_after_statements` on leading `use`s/items in test fns. Enforce
// the lint in production; allow it only under `cfg(test)`.
#![cfg_attr(test, allow(clippy::items_after_statements))]

use quilt_rs::logging::{LOG_ENV, directives};
use std::io;
use tracing::log;
use tracing_subscriber::filter::{EnvFilter, LevelFilter};

mod cli;

use cli::Args;
use cli::Error;
use cli::Std;
use cli::print;

#[tokio::main]
async fn main() {
    let args = Args::try_parse_with_env(std::env::args_os()).unwrap_or_else(|err| err.exit());
    init_logging(args.verbose);
    let format = args.format();
    cli::notice_lock_waits(|line| eprintln!("{line}"));

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

/// Logs go to stderr, so stdout carries only the command's result.
fn init_logging(verbose: bool) {
    let quilt_log = std::env::var(LOG_ENV).ok();

    tracing_subscriber::fmt()
        .with_env_filter(build_filter(quilt_log.as_deref(), verbose))
        .with_writer(io::stderr)
        .init();
}

/// The filter for `QUILT_LOG`'s value and `-v`. Reads no environment, so the
/// rule is tested without touching the test process's.
///
/// A directive that does not parse is dropped, and when none is left the
/// filter falls back to WARN.
fn build_filter(quilt_log: Option<&str>, verbose: bool) -> EnvFilter {
    EnvFilter::builder()
        .with_default_directive(LevelFilter::WARN.into())
        .parse_lossy(filter_directives(quilt_log, verbose))
}

/// `-v`, then `QUILT_LOG`, then WARN for everything. `-v` is `info` under the
/// shared rule, and a flag beats the variable. An empty `QUILT_LOG` counts as
/// unset.
fn filter_directives(quilt_log: Option<&str>, verbose: bool) -> String {
    let chosen = if verbose {
        directives("info")
    } else {
        quilt_log.and_then(directives)
    };
    chosen.unwrap_or_else(|| "warn".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ours_at(level: &str) -> String {
        format!("quilt={level},quilt_rs={level},quilt_uri={level},quilt_sync={level},warn")
    }

    #[test]
    fn without_quilt_log_or_verbose_everything_is_at_warn() {
        assert_eq!(filter_directives(None, false), "warn");
    }

    #[test]
    fn verbose_puts_our_crates_at_info() {
        assert_eq!(filter_directives(None, true), ours_at("info"));
    }

    #[test]
    fn quilt_log_sets_the_filter_through_the_shared_rule() {
        assert_eq!(filter_directives(Some("debug"), false), ours_at("debug"));
        assert_eq!(filter_directives(Some("error"), false), "error");
        assert_eq!(
            filter_directives(Some("quilt_rs=trace,aws_smithy_runtime=debug"), false),
            "quilt_rs=trace,aws_smithy_runtime=debug"
        );
    }

    #[test]
    fn an_empty_quilt_log_counts_as_unset() {
        assert_eq!(filter_directives(Some(""), false), "warn");
        assert_eq!(filter_directives(Some("  "), true), ours_at("info"));
    }

    /// The regression this guards: under `RUST_LOG`, any usable value made
    /// `-v` a no-op. A flag is the more local statement, so it wins.
    #[test]
    fn verbose_beats_quilt_log() {
        assert_eq!(filter_directives(Some("trace"), true), ours_at("info"));
        assert_eq!(filter_directives(Some("off"), true), ours_at("info"));
    }

    /// What the built filter admits, not only the string it was built from.
    #[test]
    fn a_bare_level_keeps_dependencies_at_warn() {
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(build_filter(Some("debug"), false))
            .with_writer(io::sink)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            assert!(tracing::enabled!(target: "quilt_rs::flow", tracing::Level::DEBUG));
            assert!(tracing::enabled!(target: "quilt", tracing::Level::DEBUG));
            assert!(!tracing::enabled!(target: "quilt_rs", tracing::Level::TRACE));
            assert!(!tracing::enabled!(target: "hyper", tracing::Level::DEBUG));
            assert!(tracing::enabled!(target: "hyper", tracing::Level::WARN));
        });
    }

    #[test]
    fn unparseable_directives_fall_back_to_warn() {
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(build_filter(Some("quilt_rs=loud"), false))
            .with_writer(io::sink)
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            assert!(tracing::enabled!(target: "quilt_rs", tracing::Level::WARN));
            assert!(!tracing::enabled!(target: "quilt_rs", tracing::Level::INFO));
        });
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
