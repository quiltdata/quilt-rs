//! `QuiltSync` with no window and no Tauri: `--headless`, serving its UI to a
//! browser until Ctrl-C.

use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use clap::Parser;
use clap::builder::TypedValueParser;
use semver::Version;

use crate::engine::start;
use crate::engine::start::Started;

/// The app's identifier (`tauri.conf.json`), which names its data folder.
pub const IDENTIFIER: &str = "com.quiltdata.quilt-sync";

/// Replaces the data folder in both modes, as it replaces the CLI's domain.
const DOMAIN_VAR: &str = "QUILT_DOMAIN";

/// What `--headless` takes.
#[derive(Debug, PartialEq, Eq, Parser)]
#[command(
    name = "quilt-sync",
    about = "Serve QuiltSync to a browser on this machine, with no window or tray."
)]
pub struct Args {
    /// Run with no window or tray, until Ctrl-C.
    #[arg(long, required = true)]
    pub headless: bool,

    /// The data folder, which is also the `quilt` CLI's domain. Without it,
    /// `QUILT_DOMAIN`, else the desktop app's folder.
    // Not clap's `env`: an empty variable is unset here, as in the desktop app.
    #[arg(
        long,
        value_name = "PATH",
        value_parser = clap::builder::NonEmptyStringValueParser::new().map(PathBuf::from),
    )]
    pub domain: Option<PathBuf>,
}

/// The arguments after the program's name: `None` when they don't ask for
/// headless (or its help), so the desktop app starts on whatever the OS
/// passed, a deep link included.
pub fn parse(args: &[String]) -> Option<Result<Args, clap::Error>> {
    args.iter()
        .any(|arg| {
            arg == "--headless" || arg.starts_with("--headless=") || arg == "--help" || arg == "-h"
        })
        .then(|| {
            Args::try_parse_from(
                std::iter::once("quilt-sync").chain(args.iter().map(String::as_str)),
            )
        })
}

/// `QUILT_DOMAIN`, when set, made absolute.
pub fn domain_from_env() -> Option<PathBuf> {
    std::env::var_os(DOMAIN_VAR)
        .filter(|v| !v.is_empty())
        .map(|dir| absolute(PathBuf::from(dir)))
}

/// The data folder's paths become file URLs, which a relative path can't be.
fn absolute(dir: PathBuf) -> PathBuf {
    std::path::absolute(&dir).unwrap_or(dir)
}

/// Where the desktop app keeps its data, found without Tauri.
fn default_data_dir() -> Option<PathBuf> {
    dirs::data_local_dir().map(|dir| dir.join(IDENTIFIER))
}

/// Serve until Ctrl-C (or SIGTERM).
pub fn run(args: Args) -> ExitCode {
    let Some(data_dir) = args
        .domain
        .map(absolute)
        .or_else(domain_from_env)
        .or_else(default_data_dir)
    else {
        eprintln!("No data folder: give one with --domain");
        return ExitCode::FAILURE;
    };
    let version = Version::parse(env!("CARGO_PKG_VERSION")).expect("the crate's version");
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => {
            eprintln!("Failed to start: {err}");
            return ExitCode::FAILURE;
        }
    };
    // The lock outlives the runtime: tasks still writing hold the services,
    // so a new engine mustn't take the folder before they stop.
    let served = runtime.block_on(async {
        let engine = match start::start(data_dir.clone(), version, None).await {
            Ok(Started::Engine(engine)) => engine,
            Ok(Started::Running(link)) => {
                match link {
                    Some(link) => println!("QuiltSync is already running: {link}"),
                    None => println!(
                        "QuiltSync is already running for {}, but its address is unknown",
                        data_dir.display()
                    ),
                }
                return Err(ExitCode::SUCCESS);
            }
            Err(err) => {
                eprintln!("{err}");
                return Err(ExitCode::FAILURE);
            }
        };
        println!("QuiltSync is running. Open {}", engine.link);
        println!("Press Ctrl-C to stop.");
        stopped().await;
        // Files half-written are what a stop would leave behind; a second
        // Ctrl-C doesn't wait for them.
        if engine.services.watcher.apply_in_progress() {
            println!("Finishing the files being written. Press Ctrl-C again to stop now.");
            tokio::select! {
                () = async {
                    while engine.services.watcher.apply_in_progress() {
                        tokio::time::sleep(Duration::from_millis(200)).await;
                    }
                } => {}
                () = stopped() => {}
            }
        }
        engine.services.app.logging.shutdown();
        Ok(engine.into_lock())
    });
    runtime.shutdown_timeout(Duration::from_secs(5));
    match served {
        Ok(lock) => {
            drop(lock);
            ExitCode::SUCCESS
        }
        Err(code) => code,
    }
}

/// Ctrl-C, or SIGTERM where there is one.
async fn stopped() {
    #[cfg(unix)]
    {
        use tokio::signal::unix::SignalKind;
        use tokio::signal::unix::signal;
        if let Ok(mut term) = signal(SignalKind::terminate()) {
            tokio::select! {
                _ = tokio::signal::ctrl_c() => {}
                _ = term.recv() => {}
            }
            return;
        }
    }
    let _ = tokio::signal::ctrl_c().await;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(list: &[&str]) -> Option<Result<Args, clap::Error>> {
        parse(&list.iter().map(|&s| s.to_owned()).collect::<Vec<_>>())
    }

    fn usage_error(list: &[&str]) -> clap::error::ErrorKind {
        args(list).unwrap().unwrap_err().kind()
    }

    #[test]
    fn without_headless_the_desktop_starts() {
        assert!(args(&[]).is_none());
        assert!(args(&["quilt+s3://bucket#package=a/b"]).is_none());
        assert!(args(&["--some-os-flag"]).is_none());
    }

    #[test]
    fn the_domain_is_read_with_a_space_or_an_equals() {
        let expect = Args {
            headless: true,
            domain: Some(PathBuf::from("/tmp/q")),
        };
        assert_eq!(
            args(&["--headless", "--domain", "/tmp/q"])
                .unwrap()
                .unwrap(),
            expect
        );
        assert_eq!(
            args(&["--domain=/tmp/q", "--headless"]).unwrap().unwrap(),
            expect
        );
    }

    #[test]
    fn a_bad_argument_is_a_usage_error() {
        use clap::error::ErrorKind;
        assert_eq!(
            usage_error(&["--headless", "--domain"]),
            ErrorKind::InvalidValue
        );
        assert_eq!(
            usage_error(&["--headless", "--domain", ""]),
            ErrorKind::InvalidValue
        );
        assert_eq!(
            usage_error(&["--headless", "--verbose"]),
            ErrorKind::UnknownArgument
        );
        assert_eq!(usage_error(&["--headless=x"]), ErrorKind::TooManyValues);
        assert_eq!(usage_error(&["--help"]), ErrorKind::DisplayHelp);
        for list in [
            &["--headless", "--domain"][..],
            &["--headless", "--verbose"],
        ] {
            assert_eq!(args(list).unwrap().unwrap_err().exit_code(), 2, "{list:?}");
        }
    }

    #[test]
    fn a_relative_data_folder_becomes_absolute() {
        let dir = absolute(PathBuf::from("data"));
        assert!(dir.is_absolute());
        assert!(dir.ends_with("data"));
    }

    #[test]
    fn the_identifier_is_the_apps() {
        let config: serde_json::Value =
            serde_json::from_str(include_str!("../../tauri.conf.json")).unwrap();
        assert_eq!(config["identifier"], IDENTIFIER);
    }
}
