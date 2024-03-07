use clap::{Parser, Subcommand};

use std::path::Path;

mod browse;
mod install;
mod list;
mod model;
mod output;

use model::Model;
use output::print as print_stdout_v2;

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
            let args = browse::CommandArgs { uri };
            tracing::debug!("Browsing {:?}", args);
            let output = browse::command(m, args).await;
            print_stdout_v2(output);
        }
        Commands::Install {
            path,
            namespace,
            uri: uri_str,
        } => {
            let args = install::CommandArgs {
                uri_str,
                paths: path,
                namespace,
            };
            tracing::debug!("Installing {:?}", args);
            let output = install::command(m, args).await;
            print_stdout_v2(output);
        }
        Commands::List => {
            tracing::debug!("Listing installed packages");
            let output = list::command(m).await;
            print_stdout_v2(output);
        }
    }
}

