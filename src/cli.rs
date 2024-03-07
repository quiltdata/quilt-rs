use clap::{Parser, Subcommand};

use std::path::Path;

mod browse;
mod install;
mod list;
mod model;
mod output;

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
    /// Install package
    Install {
        #[arg(short, long)]
        path: Option<Vec<String>>,
        #[arg(short, long)]
        namespace: Option<String>,
        uri: String,
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
    }
}
