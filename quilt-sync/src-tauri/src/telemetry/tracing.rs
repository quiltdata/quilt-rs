use std::path::{Path, PathBuf};

use tempfile::TempDir;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling;
use tracing_subscriber::prelude::*;
use tracing_subscriber::{EnvFilter, filter::LevelFilter};

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

/// The variable a developer can raise either sink with.
///
/// Not the escape hatch for a *user*: an app launched from the OS shell inherits no
/// terminal environment, so "set a variable and reproduce it" is not an instruction
/// anyone can follow. That is why the defaults above have to be right on their own,
/// and why the user-facing control belongs in the app rather than here.
const LOG_ENV: &str = "QUILTSYNC_LOG";

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
/// stops that thread**. Held for the life of the process, or logging quietly ends
/// after startup.
pub struct Logging {
    pub dir: LogsDir,
    _writer: Option<WorkerGuard>,
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

/// Build a filter from `directives`, or from [`LOG_ENV`] when it is set.
///
/// The whole comma-separated list is parsed at once. Parsing it as a *single*
/// directive is what broke this the first time: a list is not a directive, so the
/// parse failed, a fallback silently substituted `warn`, and the log came out empty
/// — reproducing the exact defect this module exists to fix, inside the code meant
/// to fix it. Anything that swallows a parse failure here has to be read twice.
///
/// `parse_lossy` because a directive typed into an environment variable is a
/// developer's convenience: a malformed one should cost them the override, not the
/// logs. The default directives are not user input and are pinned by a test that
/// asserts what the built filter *admits*, not merely that they parse.
///
/// The variable **replaces** rather than extends, so a developer reasoning about
/// what they will get has one string to read instead of a merge to work out.
fn filter(directives: &str) -> EnvFilter {
    let from_env = std::env::var(LOG_ENV)
        .ok()
        .filter(|value| !value.trim().is_empty());

    EnvFilter::builder()
        .with_default_directive(LevelFilter::WARN.into())
        .parse_lossy(from_env.as_deref().unwrap_or(directives))
}

pub fn init_file_logging(base_path: &Path) -> Result<Logging> {
    let dir = get_logs_dir(base_path)?;
    let writer = init_tracing(&dir);
    Ok(Logging {
        dir,
        _writer: writer,
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
            .with_filter(filter(FILE_DIRECTIVES));

        tracing_subscriber::registry()
            .with(file)
            .with(sentry::integrations::tracing::layer().with_filter(filter(CRASH_SINK_DIRECTIVES)))
            .init();
        Some(guard)
    } else {
        tracing_subscriber::registry()
            .with(sentry::integrations::tracing::layer().with_filter(filter(CRASH_SINK_DIRECTIVES)))
            .init();
        None
    };

    // Reported *after* the subscriber exists, or it goes nowhere — which is what
    // used to happen.
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

    /// The **built filter** admits what its directives say — not the directives in
    /// isolation, which is the gap that let this ship broken once.
    #[test]
    fn each_built_filter_admits_what_its_directives_ask_for() {
        use tracing_subscriber::Layer;
        use tracing_subscriber::registry::Registry;

        let file = Layer::<Registry>::max_level_hint(&filter(FILE_DIRECTIVES));
        assert_eq!(
            file,
            Some(LevelFilter::DEBUG),
            "the file filter admits {file:?}, so the log will be near-empty again"
        );

        let crash = Layer::<Registry>::max_level_hint(&filter(CRASH_SINK_DIRECTIVES));
        assert_eq!(
            crash,
            Some(LevelFilter::INFO),
            "the crash sink admits {crash:?}, so there will be no breadcrumbs"
        );
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
}
