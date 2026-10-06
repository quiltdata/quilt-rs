use std::path::{Path, PathBuf};
use std::sync::Mutex;

use tempfile::TempDir;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::filter::{FilterExt, LevelFilter};
use tracing_subscriber::prelude::*;

use quilt_rs::logging::{InvalidLogFilter, LOG_ENV};

use crate::Result;
use crate::telemetry::prelude::*;

/// What the **log file** keeps.
///
/// Our own crates at `debug`, and everything else at `warn`. The trailing
/// directive is the one that matters: the dependency tree — an HTTP stack, the AWS
/// SDK, a filesystem watcher — is far chattier than this app, so a bare `debug`
/// would bury the app's own 300-odd statements under transport noise and make the
/// support archive worse than an empty one.
///
/// The file is private, cheap, and already disclosed to the user through the
/// diagnostic export, so it can afford detail the other sink cannot.
const FILE_DIRECTIVES: &str = "quilt_sync=debug,quilt_rs=debug,quilt_uri=debug,warn";

/// What the **crash reporter** receives, as breadcrumbs and structured logs.
///
/// A level lower than the file's, deliberately: this leaves the machine and is
/// billed per event, and `debug` is where paths and package names appear. So the
/// split is a privacy control as much as a cost one — which is the whole reason
/// each sink filters for itself.
const CRASH_SINK_DIRECTIVES: &str = "quilt_sync=info,quilt_rs=info,warn";

/// The most the crash reporter ever receives, whatever [`LOG_ENV`] says.
///
/// `debug` is where paths and package names appear, and the crash reporter is
/// off-machine, so raising the log to `debug` for a support case must not raise
/// what leaves the machine. A quieter [`LOG_ENV`] still quiets this sink.
const CRASH_SINK_CEILING: LevelFilter = LevelFilter::INFO;

pub enum LogsDir {
    Permanent(PathBuf),
    Temporary(TempDir),
}

impl LogsDir {
    pub fn path(&self) -> &Path {
        match self {
            LogsDir::Permanent(path) => path,
            LogsDir::Temporary(temp_dir) => temp_dir.path(),
        }
    }
}

/// Where the log goes, and the worker keeping it flowing.
///
/// The guard is the load-bearing half: writes are handed to a background thread so
/// a log line never blocks the thread that emitted it, and **dropping the guard
/// stops that thread**. Held until [`Self::shutdown`], or logging ends wherever it
/// was dropped.
pub struct Logging {
    pub dir: LogsDir,
    writer: Mutex<Option<WorkerGuard>>,
}

impl Logging {
    /// Stop the non-blocking writer and wait for its queue to drain.
    ///
    /// The `Mutex` is what lets a `&self` in shared state give the guard up. Take
    /// it out before dropping: the drop blocks until the worker drains, so holding
    /// the lock across it would stall a command handler behind a quit. Idempotent.
    pub fn shutdown(&self) {
        let writer = self
            .writer
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .take();
        drop(writer);
    }
}

fn get_logs_dir(base_path: &Path) -> Result<LogsDir> {
    let logs_dir = base_path.join("logs");

    if let Err(err) = std::fs::create_dir_all(&logs_dir)
        && err.kind() != std::io::ErrorKind::AlreadyExists
    {
        return Ok(LogsDir::Temporary(tempfile::tempdir()?));
    }

    Ok(LogsDir::Permanent(logs_dir))
}

/// Each sink's filter, from a [`LOG_ENV`] value or the built-in directives.
///
/// The value goes through the same rule as the CLI's: a bare level puts our
/// crates at it and dependencies at `warn`, a value with `=` is directives, and
/// an empty one counts as unset. It **replaces** the built-in directives for
/// both sinks rather than merging with them: a later directive wins for the
/// same target, so merging once let the built-in `quilt_sync=debug` outvote an
/// override of `quilt_sync=trace`.
///
/// A value that does not parse is ignored and handed back, so the caller can
/// log it. The app has no terminal to refuse to start in, and losing the logs
/// over a log setting would be worse than the setting being ignored.
struct Filters {
    file: EnvFilter,
    crash: EnvFilter,
    invalid: Option<InvalidLogFilter>,
}

/// [`Filters`] for a [`LOG_ENV`] value, read by the caller so this stays pure.
fn filters(quilt_log: Option<&str>) -> Filters {
    let (from_env, invalid) = match quilt_log.map(log_filter) {
        Some(Ok(from_env)) => (from_env, None),
        Some(Err(invalid)) => (None, Some(invalid)),
        None => (None, None),
    };
    let (file, crash) = match from_env {
        Some(from_env) => (from_env.clone(), from_env),
        None => (built_in(FILE_DIRECTIVES), built_in(CRASH_SINK_DIRECTIVES)),
    };
    Filters {
        file,
        crash,
        invalid,
    }
}

/// A [`LOG_ENV`] value as a filter under the shared rule, or `Ok(None)` when
/// empty. Parsed strictly, so `quilt_rs=lots` is refused rather than dropped.
///
/// The CLI has the same few lines; the library can't hold them without
/// depending on `tracing-subscriber`.
fn log_filter(value: &str) -> std::result::Result<Option<EnvFilter>, InvalidLogFilter> {
    let Some(directives) = quilt_rs::logging::directives(value)? else {
        return Ok(None);
    };
    EnvFilter::builder()
        .parse(directives)
        .map(Some)
        .map_err(|_| InvalidLogFilter::new(value))
}

/// The built-in directives are not user input, and a test pins what each
/// built filter admits.
fn built_in(directives: &str) -> EnvFilter {
    EnvFilter::builder()
        .with_default_directive(LevelFilter::WARN.into())
        .parse_lossy(directives)
}

/// The crash sink's filter held at [`CRASH_SINK_CEILING`].
fn capped<S: ::tracing::Subscriber>(
    crash: EnvFilter,
) -> tracing_subscriber::filter::combinator::And<EnvFilter, LevelFilter, S>
where
    EnvFilter: tracing_subscriber::layer::Filter<S>,
{
    crash.and(CRASH_SINK_CEILING)
}

pub fn init_file_logging(base_path: &Path) -> Result<Logging> {
    let dir = get_logs_dir(base_path)?;
    let writer = init_tracing(&dir);
    Ok(Logging {
        dir,
        writer: Mutex::new(writer),
    })
}

/// Install the subscriber, with a filter per sink.
///
/// **Each layer filters for itself.** A filter attached to the registry instead
/// would be *global*: it drops events before any layer sees them, so a level chosen
/// for the file silently decides what the crash reporter receives — including
/// whether it gets the lower-severity events it turns into the breadcrumb trail
/// before a crash. That is exactly what used to happen, and it starved a sink by a
/// decision that was never about it.
///
/// Returns the writer guard, or `None` when no file could be opened — in which case
/// the crash sink is still installed. Previously a failed appender installed *no
/// subscriber at all*, so the error explaining why went nowhere.
fn init_tracing(logs_dir: &LogsDir) -> Option<WorkerGuard> {
    let Filters {
        file: file_filter,
        crash,
        invalid,
    } = filters(std::env::var(LOG_ENV).ok().as_deref());

    let appender = rolling::RollingFileAppender::builder()
        .rotation(rolling::Rotation::DAILY)
        .filename_prefix("quilt-sync")
        .filename_suffix("log")
        // Ten *files*, which is ten days on which something was written — not ten
        // calendar days. Pruning runs on rotation and rotation needs a write, so a
        // quiet install keeps older files and a busy one fewer. Deliberate: the
        // export is user-initiated, so "reproduce it and send the file" is the
        // question it answers.
        .max_log_files(10)
        .build(logs_dir.path())
        .ok();

    // The crash-sink layer is written out in both arms rather than shared. Its type
    // is parameterised by the subscriber it composes with, so the two arms want
    // different instantiations — a binding or a closure would fix it to whichever
    // arm was built first.
    let guard = if let Some(appender) = appender {
        // Non-blocking, so a log line costs the emitting thread a channel send
        // rather than a file write — the appender is otherwise synchronous, and
        // several of these threads belong to an async runtime.
        let (writer, guard) = tracing_appender::non_blocking(appender);
        let file = tracing_subscriber::fmt::layer()
            // ANSI is on by default when the feature is enabled, which it is. A
            // file is not a terminal: colour codes here reach whoever reads the
            // support archive as escape sequences.
            .with_ansi(false)
            .with_writer(writer)
            .with_filter(file_filter);

        tracing_subscriber::registry()
            .with(file)
            .with(sentry::integrations::tracing::layer().with_filter(capped(crash)))
            .init();
        Some(guard)
    } else {
        tracing_subscriber::registry()
            .with(sentry::integrations::tracing::layer().with_filter(capped(crash)))
            .init();
        None
    };

    // Reported *after* the subscriber exists, or it goes nowhere — which is what
    // used to happen. The ignored value first, so it heads the log it changed.
    if let Some(invalid) = invalid {
        warn!("Ignoring {LOG_ENV}: {invalid}; using the built-in log filters");
    }
    match logs_dir {
        LogsDir::Temporary(_) => error!(
            "Failed to create permanent logs directory, using temporary directory: {}",
            logs_dir.path().display()
        ),
        LogsDir::Permanent(_) if guard.is_none() => error!(
            "Failed to open a log file in {}; only crash reporting will receive logs",
            logs_dir.path().display()
        ),
        LogsDir::Permanent(_) => {}
    }

    guard
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    /// Both sinks' directives parse, and the app's own crates are held above the
    /// dependency floor in each. A typo here would silently degrade to `warn` and
    /// look like the bug this unit exists to fix.
    #[test]
    fn the_default_directives_parse_and_raise_our_own_crates() {
        for directives in [FILE_DIRECTIVES, CRASH_SINK_DIRECTIVES] {
            for directive in directives.split(',') {
                assert!(
                    directive
                        .parse::<tracing_subscriber::filter::Directive>()
                        .is_ok(),
                    "unparseable directive {directive:?} in {directives:?}"
                );
            }
            assert!(
                directives.contains("quilt_sync="),
                "{directives:?} never raises the app's own crate above the floor"
            );
            assert!(
                directives.ends_with("warn"),
                "{directives:?} lacks the trailing dependency floor, so the log fills with transport noise"
            );
        }
    }

    use tracing_subscriber::Layer;
    use tracing_subscriber::layer::Filter;
    use tracing_subscriber::registry::Registry;

    /// The most the file admits for a `QUILT_LOG` value.
    fn file_level(quilt_log: Option<&str>) -> Option<LevelFilter> {
        Layer::<Registry>::max_level_hint(&filters(quilt_log).file)
    }

    /// The most the crash reporter admits for a `QUILT_LOG` value, ceiling
    /// included.
    fn crash_level(quilt_log: Option<&str>) -> Option<LevelFilter> {
        Filter::<Registry>::max_level_hint(&capped::<Registry>(filters(quilt_log).crash))
    }

    /// The **built filter** admits what its directives say — not the directives in
    /// isolation, which is the gap that let this ship broken once.
    #[test]
    fn each_built_filter_admits_what_its_directives_ask_for() {
        let file = file_level(None);
        assert_eq!(
            file,
            Some(LevelFilter::DEBUG),
            "the file filter admits {file:?}, so the log will be near-empty again"
        );

        let crash = crash_level(None);
        assert_eq!(
            crash,
            Some(LevelFilter::INFO),
            "the crash sink admits {crash:?}, so there will be no breadcrumbs"
        );
    }

    /// An empty `QUILT_LOG` counts as unset and leaves the defaults intact.
    #[test]
    fn an_empty_value_leaves_the_defaults_alone() {
        for value in ["", "   "] {
            let filters = filters(Some(value));
            assert!(filters.invalid.is_none(), "{value:?}");
            let rendered = filters.file.to_string();
            assert!(
                rendered.contains("quilt_sync=debug"),
                "{value:?} cost the file filter the app's own crate: {rendered:?}"
            );
        }
    }

    /// A bare level goes through the shared rule: our crates at it, dependencies
    /// at `warn`.
    #[test]
    fn a_bare_level_raises_our_crates_in_the_file() {
        let rendered = filters(Some("trace")).file.to_string();
        assert!(rendered.contains("quilt_sync=trace"), "{rendered:?}");
        assert_eq!(file_level(Some("trace")), Some(LevelFilter::TRACE));
    }

    /// The value replaces the built-in directives, so it wins for a target they
    /// already name. The rendered filter is what this asserts: the level hint
    /// is a ceiling and cannot see a directive outvoted by a later one.
    #[test]
    fn a_value_replaces_the_built_in_directives() {
        let rendered = filters(Some("quilt_sync=trace")).file.to_string();
        assert!(
            rendered.contains("quilt_sync=trace"),
            "{rendered:?} lost the value"
        );
        assert!(
            !rendered.contains("quilt_sync=debug"),
            "{rendered:?} still carries the default the value replaces"
        );
    }

    /// The crash reporter never goes below `info`, however loud the file is.
    #[test]
    fn the_crash_sink_never_admits_debug() {
        for value in ["debug", "trace", "quilt_sync=trace", "trace,hyper=trace"] {
            assert_eq!(
                crash_level(Some(value)),
                Some(LevelFilter::INFO),
                "QUILT_LOG={value:?}"
            );
        }
    }

    /// A quieter value still quiets the crash reporter.
    #[test]
    fn a_quieter_value_still_applies_to_the_crash_sink() {
        for (value, level) in [
            ("warn", LevelFilter::WARN),
            ("error", LevelFilter::ERROR),
            ("off", LevelFilter::OFF),
        ] {
            assert_eq!(crash_level(Some(value)), Some(level), "QUILT_LOG={value:?}");
        }
    }

    /// A value that does not parse is ignored and handed back for the warning,
    /// leaving both sinks on their built-in filters.
    #[test]
    fn an_invalid_value_falls_back_to_the_built_in_filters() {
        for value in ["debgu", "quilt_rs=lots"] {
            let invalid = filters(Some(value)).invalid;
            assert_eq!(invalid, Some(InvalidLogFilter::new(value)), "{value:?}");
            assert_eq!(file_level(Some(value)), file_level(None), "{value:?}");
            assert_eq!(crash_level(Some(value)), crash_level(None), "{value:?}");
        }
    }

    /// The file keeps more than the crash reporter, which is the point of filtering
    /// per sink: detail belongs on the user's own disk, not billed and off-machine.
    #[test]
    fn the_file_keeps_more_than_the_crash_sink() {
        assert!(FILE_DIRECTIVES.contains("quilt_sync=debug"));
        assert!(CRASH_SINK_DIRECTIVES.contains("quilt_sync=info"));
        assert_ne!(FILE_DIRECTIVES, CRASH_SINK_DIRECTIVES);
    }

    /// A missing logs directory falls back to a temporary one rather than failing
    /// startup — and the fallback is reported, which it could not be before.
    #[test]
    fn an_unwritable_base_falls_back_to_a_temporary_directory() {
        let parent = TempDir::new().expect("tempdir");
        let base = parent.path().join("nested").join("deeper");

        let dir = get_logs_dir(&base).expect("a logs dir either way");

        assert!(dir.path().exists(), "the chosen directory must exist");
    }

    /// A writer slow enough that the worker falls behind.
    ///
    /// Over the real file appender it never does — formatting an event costs the
    /// emitting thread about what a small write costs the worker — so a drain test
    /// written against that appender passes whether or not anything drained.
    struct SlowWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SlowWriter {
        fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
            std::thread::sleep(std::time::Duration::from_millis(1));
            self.0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    /// [`Logging::shutdown`] returns only once the queue is empty. Delete the call
    /// and this fails with the buffer all but empty.
    ///
    /// The subscriber is local, not global: the global slot is process-wide and
    /// every other test in this file would contend for it.
    #[test]
    fn shutdown_waits_for_the_writer_to_drain() {
        // Enough that the worker is certainly behind, few enough that draining
        // stays inside the appender's own one-second cap on the shutdown wait.
        const LINES: usize = 100;

        let written = Arc::new(Mutex::new(Vec::new()));
        let (writer, guard) = tracing_appender::non_blocking(SlowWriter(Arc::clone(&written)));
        let logging = Logging {
            dir: LogsDir::Temporary(TempDir::new().expect("tempdir")),
            writer: Mutex::new(Some(guard)),
        };

        let subscriber = tracing_subscriber::registry().with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(writer),
        );
        ::tracing::subscriber::with_default(subscriber, || {
            for line in 0..LINES {
                error!("line {line}");
            }
        });

        logging.shutdown();

        let kept = |written: &Mutex<Vec<u8>>| {
            String::from_utf8_lossy(
                &written
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            )
            .lines()
            .count()
        };
        assert_eq!(
            kept(&written),
            LINES,
            "the tail was lost: {} of {LINES} lines reached the writer",
            kept(&written)
        );

        // `take` leaves nothing for a second call to drop.
        logging.shutdown();
        assert_eq!(kept(&written), LINES, "a second shutdown lost lines");
    }
}
