use std::path::PathBuf;
use std::time::Duration;
use std::time::SystemTime;

use clap::Parser;
use clap::Subcommand;
use notify::Watcher;
use quilt_agent::Error;
use quilt_agent::agent::Agent;
use quilt_agent::profile::Profile;
use quilt_agent::spool::Spool;
use quilt_rs::io::remote::RemoteS3;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::paths::DomainPaths;
use quilt_uri::Host;

#[derive(Parser)]
#[command(version, about = "Moves completed instrument runs into Quilt packages")]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Validate a profile and exit.
    Check { profile: PathBuf },
    /// Run until stopped: the service entry point.
    Run {
        profile: PathBuf,
        /// Publish with ambient AWS credentials and no registry (degraded mode).
        #[arg(long)]
        bucket_only: bool,
    },
}

#[tokio::main]
async fn main() -> std::process::ExitCode {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();
    match run(Cli::parse().command).await {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(e) => {
            tracing::error!("{e}");
            std::process::ExitCode::FAILURE
        }
    }
}

async fn run(command: Command) -> Result<(), Error> {
    match command {
        Command::Check { profile } => {
            let p = Profile::load(&profile)?;
            println!("ok: {} instrument(s)", p.instruments.len());
            Ok(())
        }
        Command::Run {
            profile,
            bucket_only,
        } => serve(Profile::load(&profile)?, bucket_only).await,
    }
}

async fn serve(profile: Profile, bucket_only: bool) -> Result<(), Error> {
    let spool_root = profile
        .observer
        .spool_root
        .clone()
        .ok_or_else(|| Error::Profile("observer.spool_root is required".to_string()))?;
    let spool = Spool::open(&spool_root)?;
    let remote = RemoteS3::new(
        DomainPaths::new(spool_root.join("domain")),
        LocalStorage::default(),
    );
    let host = if bucket_only {
        None
    } else {
        let url = profile.registry.url.trim_end_matches('/');
        let host: Host = url
            .trim_start_matches("https://")
            .parse()
            .map_err(|e| Error::Profile(format!("registry.url: {e}")))?;
        // ponytail: credential_ref names an environment variable; an OS
        // keystore lookup replaces this with the installer.
        let var = &profile.registry.credential_ref;
        let key = std::env::var(var)
            .map_err(|_| Error::Profile(format!("registry.credential_ref: {var} is unset")))?;
        remote.set_api_key(&host, key);
        Some(host)
    };

    let poll = Duration::from_secs(
        profile
            .instruments
            .iter()
            .map(|i| i.source.poll_s)
            .min()
            .unwrap_or(30),
    );
    // notify only shortens the wait to the next scan; polling is the truth.
    let (tx, mut rx) = tokio::sync::mpsc::channel(1);
    // Kept alive for the loop; `None` (no inotify slots, say) means polling only.
    let _watcher = match notify::recommended_watcher(move |_| {
        let _ = tx.try_send(());
    }) {
        Ok(mut watcher) => {
            for i in &profile.instruments {
                if let Err(e) = watcher.watch(&i.source.path, notify::RecursiveMode::Recursive) {
                    tracing::info!(path = %i.source.path.display(), "no change events, polling only: {e}");
                }
            }
            Some(watcher)
        }
        Err(e) => {
            tracing::info!("no change events, polling only: {e}");
            None
        }
    };

    let mut agent = Agent::new(profile, remote, spool, host);
    tracing::info!("running; poll every {}s", poll.as_secs());
    loop {
        agent.pass(SystemTime::now()).await?;
        tokio::select! {
            () = tokio::time::sleep(poll) => {}
            // `Some` only: with no watcher the channel is closed and `recv`
            // returns `None` at once, which would turn polling into a busy loop.
            Some(()) = rx.recv() => tokio::time::sleep(Duration::from_secs(1)).await,
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("stopping; the journal resumes on next start");
                return Ok(());
            }
        }
    }
}
