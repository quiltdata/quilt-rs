use clap::{Parser, Subcommand};
use std::path::PathBuf;

mod browse;
mod install;
mod list;
mod model;
mod output;
mod package;
mod uninstall;

use model::Model;
use output::print;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Browse remote manifest
    Browse { uri: quilt_rs::S3PackageURI },
    /// Install package locally
    Install {
        /// Source URI for the package.
        /// Ex. quilt+s3://bucket#package=foo/bar
        uri: quilt_rs::S3PackageURI,
        /// Path to local domain. Should be absolute path when installing paths
        #[arg(short, long)]
        domain: PathBuf,
        /// Namespace for the package, ex. foo/bar.
        #[arg(short, long)]
        namespace: Option<String>,
        /// Logical key relative to the root of the package to be installed locally.
        /// You can provide multiple paths.
        #[arg(short, long)]
        path: Option<Vec<PathBuf>>,
    },
    /// List installed packages
    List {
        /// Path to local domain
        #[arg(short, long)]
        domain: PathBuf,
    },
    /// Create and install manifest to S3
    Package {
        /// Source URI for the package.
        /// Ex. s3://bucket/s3/prefix
        uri: String,
        /// quilt+s3 URI for new package
        #[arg(short, long)]
        target: quilt_rs::S3PackageURI,
    },
    Uninstall {
        /// Namespace of the package to uninstall
        namespace: String,
        /// Path to local domain
        #[arg(short, long)]
        domain: PathBuf,
    },
}

pub async fn init() -> Result<(), Error> {
    let args = Args::parse();

    match args.command {
        Commands::Browse { uri } => {
            let (m, temp_dir) = Model::from_temp_dir()?;
            let args = browse::Input { uri };
            tracing::info!("Browsing {:?} using {:?}", args, temp_dir);
            print(browse::command(m, args).await);
            Ok(())
        }
        Commands::Install {
            path,
            domain,
            namespace,
            uri,
        } => {
            let m = Model::from(domain);
            let args = install::Input {
                namespace,
                paths: path,
                uri,
            };
            tracing::info!("Installing {:?}", args);
            print(install::command(m, args).await);
            Ok(())
        }
        Commands::List { domain } => {
            if !domain.exists() {
                return Err(Error::Domain(domain));
            }
            let m = Model::from(domain);
            tracing::info!("Listing installed packages");
            print(list::command(m).await);
            Ok(())
        }
        Commands::Package { uri, target } => {
            let (m, temp_dir) = Model::from_temp_dir()?;
            let args = package::Input { target, uri };
            tracing::info!("Packaging {:?} using {:?}", args, temp_dir);
            print(package::command(m, args).await);
            Ok(())
        }
        Commands::Uninstall { domain, namespace } => {
            if !domain.exists() {
                return Err(Error::Domain(domain));
            }
            let m = Model::from(domain);
            let args = uninstall::Input { namespace };
            tracing::info!("Uninstalling {:?}", args);
            print(uninstall::command(m, args).await);
            Ok(())
        }
    }
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Domain path doesn't exists: {0}")]
    Domain(std::path::PathBuf),

    #[error("Failed to create temp dir: {0}")]
    TempDir(String),

    #[error("quilt_rs error: {0}")]
    Quilt(quilt_rs::Error),
}

impl From<quilt_rs::Error> for Error {
    fn from(err: quilt_rs::Error) -> Error {
        Error::Quilt(err)
    }
}
