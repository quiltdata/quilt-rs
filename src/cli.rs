use clap::{Parser, Subcommand};

use std::path::Path;

mod browse;
mod install;
mod list;
mod model;
mod output;
mod package;

use model::Model;
use output::print;

#[derive(Parser)]
#[command(version, about, long_about = None)]
struct Args {
    /// Path to local domain. Should be absolute path when installing paths
    #[arg(short, long)]
    domain: String,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Browse remote manifest
    Browse { uri: String },
    /// Install package locally
    Install {
        /// Logical key relative to the root of the package to be installed locally.
        /// You can provide multiple paths.
        #[arg(short, long)]
        path: Option<Vec<String>>,
        /// Namespace for the package, ex. foo/bar.
        #[arg(short, long)]
        namespace: Option<String>,
        /// Source URI for the package.
        /// Ex. quilt+s3://bucket#package=foo/bar
        uri: String,
        // #[arg(short, long)]
        // commit_msg: Option<String>,
        // #[arg(short, long)]
        // commit_meta: Option<String>,
    },
    /// Create and install manifest to S3
    Package {
        /// Source URI for the package.
        /// Ex. s3://bucket/s3/prefix
        uri: String,
        /// If provided, package will be pushed with new commit to that quilt+s3 URI
        #[arg(short, long)]
        target: String,
    },
    /// List installed packages
    List,
}

pub async fn init() {
    let args = Args::parse();

    let local_domain = quilt_rs::LocalDomain::new(Path::new(&args.domain).to_path_buf());
    let m = Model::new(local_domain.clone());

    match args.command {
        Commands::Browse { uri } => {
            let args = browse::Input { uri };
            tracing::debug!("Browsing {:?}", args);
            print(browse::command(m, args).await);
        }
        Commands::Install {
            path,
            namespace,
            uri,
        } => {
            let args = install::Input {
                namespace,
                paths: path,
                uri,
            };
            tracing::debug!("Installing {:?}", args);
            print(install::command(m, args).await);
        }
        Commands::List => {
            tracing::debug!("Listing installed packages");
            print(list::command(m).await);
        }
        Commands::Package { uri, target } => {
            let args = package::Input { target, uri };
            tracing::debug!("Installing {:?}", args);
            print(package::command(m, args).await);
        }
    }
}
