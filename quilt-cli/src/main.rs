// `test_log`'s `#[test]` macro injects an init statement before the body, so it
// trips `items_after_statements` on leading `use`s/items in test fns. Enforce
// the lint in production; allow it only under `cfg(test)`.
#![cfg_attr(test, allow(clippy::items_after_statements))]

use quilt_rs::logging::{InvalidLogFilter, LOG_ENV};
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
    let logging = init_logging(
        std::env::var(LOG_ENV).ok().as_deref(),
        args.log_directives(),
    );
    let format = args.format();
    cli::notice_lock_waits(|line| eprintln!("{line}"));

    // An error raised before dispatch — an unreadable domain, a rejected flag
    // combination — is a command failure like any other. It used to go out as a
    // tracing line, which under `--json` would hand a consumer prose where it
    // expects an object. A `QUILT_LOG` that does not parse is one too: the
    // command does not run.
    let result = match logging {
        Ok(()) => to_std(cli::init(args).await),
        Err(err) => Std::Err(err),
    };

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
///
/// A `QUILT_LOG` that does not parse is returned as the error, after
/// installing the default filter so the rest of the run still logs warnings.
fn init_logging(quilt_log: Option<&str>, flag: Option<String>) -> Result<(), Error> {
    let (directives, result) = match filter_directives(quilt_log, flag) {
        Ok(directives) => (directives, Ok(())),
        Err(err) => (DEFAULT_DIRECTIVES.to_string(), Err(Error::LogEnv(err))),
    };

    tracing_subscriber::fmt()
        .with_env_filter(build_filter(&directives))
        .with_writer(io::stderr)
        .init();
    result
}

/// Everything at WARN: what the CLI logs with neither a flag nor `QUILT_LOG`.
const DEFAULT_DIRECTIVES: &str = "warn";

/// The filter for directives [`filter_directives`] chose, which already
/// parsed.
fn build_filter(directives: &str) -> EnvFilter {
    EnvFilter::builder()
        .with_default_directive(LevelFilter::WARN.into())
        .parse_lossy(directives)
}

/// The directives from `--log` or `-v`, then `QUILT_LOG`, then WARN for
/// everything. The flag replaces the variable. An empty `QUILT_LOG` counts as
/// unset. Reads no environment, so the rule is tested without touching the
/// test process's.
///
/// A `QUILT_LOG` that does not parse is an error even under a flag: a broken
/// variable is reported whether or not a flag overrides it.
fn filter_directives(
    quilt_log: Option<&str>,
    flag: Option<String>,
) -> Result<String, InvalidLogFilter> {
    let from_env = match quilt_log {
        Some(value) => cli::log_filter(value)?,
        None => None,
    };
    Ok(flag
        .or(from_env)
        .unwrap_or_else(|| DEFAULT_DIRECTIVES.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ours_at(level: &str) -> String {
        format!("quilt={level},quilt_rs={level},quilt_uri={level},quilt_sync={level},warn")
    }

    fn ok(quilt_log: Option<&str>, flag: Option<&str>) -> String {
        filter_directives(quilt_log, flag.map(str::to_string))
            .unwrap_or_else(|err| panic!("{quilt_log:?}: {err}"))
    }

    #[test]
    fn without_quilt_log_or_a_flag_everything_is_at_warn() {
        assert_eq!(ok(None, None), "warn");
    }

    #[test]
    fn the_flag_sets_the_filter_without_quilt_log() {
        assert_eq!(ok(None, Some("quilt_rs=trace")), "quilt_rs=trace");
    }

    #[test]
    fn quilt_log_sets_the_filter_through_the_shared_rule() {
        assert_eq!(ok(Some("debug"), None), ours_at("debug"));
        assert_eq!(ok(Some("error"), None), "error");
        assert_eq!(
            ok(Some("quilt_rs=trace,aws_smithy_runtime=debug"), None),
            "quilt_rs=trace,aws_smithy_runtime=debug"
        );
    }

    #[test]
    fn an_empty_quilt_log_counts_as_unset() {
        assert_eq!(ok(Some(""), None), "warn");
        assert_eq!(ok(Some("  "), Some("error")), "error");
    }

    /// The regression this guards: under `RUST_LOG`, any usable value made
    /// `-v` a no-op. A flag is the more local statement, so it wins, and it
    /// replaces the variable rather than adding to it.
    #[test]
    fn the_flag_beats_quilt_log() {
        assert_eq!(ok(Some("trace"), Some(&ours_at("info"))), ours_at("info"));
        assert_eq!(ok(Some("off"), Some(&ours_at("info"))), ours_at("info"));
        assert_eq!(
            ok(Some("quilt_rs=trace"), Some("hyper=debug")),
            "hyper=debug"
        );
    }

    /// `debgu` used to read as a target name and hide every quilt line;
    /// `quilt_rs=lots` used to be dropped without a word.
    #[test]
    fn a_quilt_log_that_does_not_parse_is_an_error() {
        for value in [
            "debgu",
            "quilt_rs",
            "quilt_rs=lots",
            "quilt_rs=trace,hyper=lots",
        ] {
            assert_eq!(
                filter_directives(Some(value), None),
                Err(InvalidLogFilter::new(value)),
                "{value}"
            );
        }
    }

    /// As `QUILT_FORMAT=yaml` with `--json`: a broken variable is reported
    /// even when a flag overrides it.
    #[test]
    fn a_quilt_log_that_does_not_parse_is_an_error_with_a_flag_too() {
        assert_eq!(
            filter_directives(Some("debgu"), Some(ours_at("info"))),
            Err(InvalidLogFilter::new("debgu"))
        );
    }

    fn printed(err: InvalidLogFilter, format: cli::Format) -> String {
        let (mut stdout, mut stderr) = (Vec::new(), Vec::new());
        print(
            Std::Err(Error::LogEnv(err)),
            format,
            &mut stdout,
            &mut stderr,
        )
        .unwrap();
        assert_eq!(stdout, b"");
        String::from_utf8(stderr).unwrap()
    }

    #[test]
    fn an_invalid_quilt_log_prints_as_the_json_error_object() {
        let json: serde_json::Value =
            serde_json::from_str(&printed(InvalidLogFilter::new("debgu"), cli::Format::Json))
                .unwrap();
        assert_eq!(json["error"]["kind"], "invalid_log_filter");
        assert_eq!(
            json["error"]["message"],
            "QUILT_LOG=\"debgu\" is not a log level or a list of directives"
        );
    }

    #[test]
    fn an_invalid_quilt_log_prints_as_text() {
        assert_eq!(
            printed(InvalidLogFilter::new("debgu"), cli::Format::Text),
            "QUILT_LOG=\"debgu\" is not a log level or a list of directives\n"
        );
    }

    /// What the built filter admits, not only the string it was built from.
    #[test]
    fn a_bare_level_keeps_dependencies_at_warn() {
        let subscriber = tracing_subscriber::fmt()
            .with_env_filter(build_filter(&ours_at("debug")))
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
