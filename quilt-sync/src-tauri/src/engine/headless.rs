//! `QuiltSync` with no window and no Tauri: `--headless`, serving its UI to a
//! browser until Ctrl-C.

use std::net::IpAddr;
use std::net::Ipv4Addr;
use std::path::PathBuf;
use std::process::ExitCode;
use std::time::Duration;

use semver::Version;

use crate::engine::start;
use crate::engine::start::Options;
use crate::engine::start::Port;
use crate::engine::start::Started;

/// The app's identifier (`tauri.conf.json`), which names its data folder.
pub const IDENTIFIER: &str = "com.quiltdata.quilt-sync";

/// Replaces the data folder in both modes, as it replaces the CLI's domain.
const DOMAIN_VAR: &str = "QUILT_DOMAIN";

const USAGE: &str =
    "Usage: quilt-sync --headless [--bind <ip>] [--port <n>] [--domain <path>] [--reset-token]";

/// What `--headless` was given.
#[derive(Debug, PartialEq, Eq)]
pub struct Args {
    pub bind: IpAddr,
    pub port: Port,
    pub domain: Option<PathBuf>,
    pub reset_token: bool,
}

/// The arguments after the program's name: `None` when they don't ask for
/// headless, so the desktop app starts.
pub fn parse(args: &[String]) -> Option<Result<Args, String>> {
    args.iter()
        .any(|arg| arg == "--headless")
        .then(|| parse_headless(args).map_err(|err| format!("{err}\n{USAGE}")))
}

fn parse_headless(args: &[String]) -> Result<Args, String> {
    let mut parsed = Args {
        bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
        port: Port::Headless,
        domain: None,
        reset_token: false,
    };
    let mut rest = args.iter().peekable();
    while let Some(arg) = rest.next() {
        let (flag, inline) = match arg.split_once('=') {
            Some((flag, value)) => (flag, Some(value.to_owned())),
            None => (arg.as_str(), None),
        };
        // A flag next isn't a value; `--domain=--odd` still names such a folder.
        let mut value = || {
            inline
                .clone()
                .or_else(|| rest.next_if(|next| !next.starts_with("--")).cloned())
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag {
            "--headless" => {}
            "--reset-token" => parsed.reset_token = true,
            "--bind" => {
                parsed.bind = value()?
                    .parse()
                    .map_err(|_| "--bind takes an IP address".to_owned())?;
            }
            "--port" => {
                parsed.port = Port::Given(
                    value()?
                        .parse()
                        .map_err(|_| "--port takes a number".to_owned())?,
                );
            }
            "--domain" => parsed.domain = Some(PathBuf::from(value()?)),
            other => return Err(format!("Unknown argument: {other}")),
        }
    }
    Ok(parsed)
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
        let options = Options {
            data_dir: data_dir.clone(),
            version,
            desktop: None,
            bind: args.bind,
            port: args.port,
            reset_token: args.reset_token,
        };
        let engine = match start::start(options).await {
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
        if !args.bind.is_loopback() {
            let port = url::Url::parse(&engine.link)
                .ok()
                .and_then(|url| url.port_or_known_default())
                .unwrap_or(start::HEADLESS_PORT);
            println!(
                "Warning: QuiltSync listens on {} over plain HTTP, so the link's token and your \
                 data travel unencrypted. Prefer the default bind and an SSH tunnel, e.g. \
                 `ssh -L {port}:localhost:{port} <this host>`.",
                args.bind
            );
        }
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

    fn args(list: &[&str]) -> Option<Result<Args, String>> {
        parse(&list.iter().map(|&s| s.to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn without_headless_the_desktop_starts() {
        assert_eq!(args(&[]), None);
        assert_eq!(args(&["quilt+s3://bucket#package=a/b"]), None);
    }

    #[test]
    fn the_defaults_are_loopback_and_the_headless_port_rule() {
        assert_eq!(
            args(&["--headless"]),
            Some(Ok(Args {
                bind: IpAddr::V4(Ipv4Addr::LOCALHOST),
                port: Port::Headless,
                domain: None,
                reset_token: false,
            }))
        );
    }

    #[test]
    fn every_flag_is_read_with_a_space_or_an_equals() {
        assert_eq!(
            args(&[
                "--headless",
                "--bind",
                "0.0.0.0",
                "--port=0",
                "--domain",
                "/tmp/q",
                "--reset-token",
            ]),
            Some(Ok(Args {
                bind: IpAddr::V4(Ipv4Addr::UNSPECIFIED),
                port: Port::Given(0),
                domain: Some(PathBuf::from("/tmp/q")),
                reset_token: true,
            }))
        );
    }

    #[test]
    fn a_bad_argument_says_what_and_how() {
        for bad in [
            &["--headless", "--port", "lots"][..],
            &["--headless", "--bind", "here"],
            &["--headless", "--port"],
            &["--headless", "--domain", "--reset-token"],
            &["--headless", "--verbose"],
        ] {
            let err = args(bad).unwrap().unwrap_err();
            assert!(err.contains("Usage:"), "{err}");
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
