use clap::{Parser, Subcommand};
use quilt_rs;
use std::path::Path;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to local domain
    #[arg(short, long)]
    domain: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Install package
    Install {
        #[arg(short, long)]
        uri: String,
    },
}

async fn install_package(
    loc: &quilt_rs::LocalDomain,
    uri_str: &str,
) -> Result<quilt_rs::InstalledPackage, String> {
    let uri = quilt_rs::S3PackageURI::try_from(uri_str)?;
    let remote_manifest = quilt_rs::RemoteManifest::resolve(&uri).await?;
    loc.install_package(&remote_manifest).await
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let loc = quilt_rs::LocalDomain::new(Path::new(&args.domain).to_path_buf());

    match args.command {
        Commands::Install { uri: uri_string } => {
            tracing::debug!("Installing {}", uri_string);
            match install_package(&loc, uri_string.as_str()).await {
                Ok(installed_package) => {
                    tracing::info!(
                        "Package {:?} installed to {:?}",
                        installed_package,
                        loc.working_folder(&installed_package.namespace),
                    )
                }
                Err(err) => panic!("{}", err),
            }
        }
    }
}
