use clap::{Parser, Subcommand};
use std::path::Path;

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
    /// Path to local domain. Should be absolute path when installing paths
    #[arg(short, long)]
    domain: Option<String>,

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
    /// List installed packages
    List,
    /// Create and install manifest to S3
    Package {
        /// Source URI for the package.
        /// Ex. s3://bucket/s3/prefix
        uri: String,
        /// If provided, package will be pushed with new commit to that quilt+s3 URI
        #[arg(short, long)]
        target: String,
    },
    Uninstall {
        /// Namespace of the package to uninstall
        namespace: String,
    },
}

pub async fn init() -> Result<(), std::io::Error> {
    let args = Args::parse();

    match args.command {
        Commands::Browse { uri } => {
            let (m, temp_dir) = Model::from_temp_dir();
            let args = browse::Input { uri };
            tracing::info!("Browsing {:?} using {:?}", args, temp_dir);
            print(browse::command(m, args).await);
            Ok(())
        }
        Commands::Install {
            path,
            namespace,
            uri,
        } => {
            let root = Path::new(&args.domain.unwrap()).to_path_buf();
            let m = Model::from(root);
            let args = install::Input {
                namespace,
                paths: path,
                uri,
            };
            tracing::info!("Installing {:?}", args);
            print(install::command(m, args).await);
            Ok(())
        }
        Commands::List => {
            let root = Path::new(&args.domain.unwrap()).to_path_buf();
            let m = Model::from(root);
            tracing::info!("Listing installed packages");
            print(list::command(m).await);
            Ok(())
        }
        Commands::Package { uri, target } => {
            let (m, temp_dir) = Model::from_temp_dir();
            let args = package::Input { target, uri };
            tracing::info!("Packaging {:?} using {:?}", args, temp_dir);
            print(package::command(m, args).await);
            Ok(())
        }
        Commands::Uninstall { namespace } => {
            let root = Path::new(&args.domain.unwrap()).to_path_buf();
            let m = Model::from(root);
            let args = uninstall::Input { namespace };
            tracing::info!("Uninstalling {:?}", args);
            print(uninstall::command(m, args).await);
            Ok(())
        }
    }
}
