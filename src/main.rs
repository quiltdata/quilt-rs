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
    /// Browse remote manifest
    Browse {
        #[arg(short, long)]
        uri: String,
    },
    /// Install package
    Install {
        #[arg(short, long)]
        uri: String,
    },
    /// List installed packages
    List,
}

async fn browse_remote_manifest(
    local_domain: &quilt_rs::LocalDomain,
    uri_str: &str,
) -> Result<quilt_rs::Table, String> {
    let uri = quilt_rs::S3PackageURI::try_from(uri_str)?;
    let remote_manifest = quilt_rs::RemoteManifest::resolve(&uri).await?;
    local_domain.browse_remote_manifest(&remote_manifest).await
}

async fn package_install(
    local_domain: &quilt_rs::LocalDomain,
    uri_str: &str,
) -> Result<quilt_rs::InstalledPackage, String> {
    let uri = quilt_rs::S3PackageURI::try_from(uri_str)?;
    let remote_manifest = quilt_rs::RemoteManifest::resolve(&uri).await?;
    local_domain.install_package(&remote_manifest).await
}

async fn get_installed_packages_list(
    local_domain: &quilt_rs::LocalDomain,
) -> Result<Vec<quilt_rs::InstalledPackage>, String> {
    local_domain.list_installed_packages().await
}

fn print_stdout(str: String) {
    println!("{}", str);
}

#[tokio::main]
async fn main() {
    let args = Args::parse();

    let local_domain = quilt_rs::LocalDomain::new(Path::new(&args.domain).to_path_buf());

    match args.command {
        Commands::Browse { uri: uri_string } => {
            tracing::debug!("Browsing {}", uri_string);
            match browse_remote_manifest(&local_domain, uri_string.as_str()).await {
                Ok(manifest_contents) => {
                    // TODO: use tables
                    print_stdout(format!("Info:\t{:?}", manifest_contents.header.info));
                    print_stdout(format!("Meta:\t{:?}", manifest_contents.header.meta));
                    for (name, entry) in manifest_contents.records {
                        let entry_output = serde_json::json!({"name": entry.name, "place": entry.place, "size": entry.size});
                        print_stdout(format!("{}:\t{}", name, entry_output));
                    }
                }

                Err(err) => panic!("{}", err),
            }
        }
        Commands::Install { uri: uri_string } => {
            tracing::debug!("Installing {}", uri_string);
            match package_install(&local_domain, uri_string.as_str()).await {
                Ok(installed_package) => print_stdout(format!(
                    "Package {:?} installed to {:?}",
                    installed_package,
                    local_domain.working_folder(&installed_package.namespace),
                )),
                Err(err) => panic!("{}", err),
            }
        }
        Commands::List => {
            tracing::debug!("Listing installed packages");
            match get_installed_packages_list(&local_domain).await {
                Ok(installed_packages_list) => {
                    for installed_package in installed_packages_list {
                        print_stdout(format!("{:?}", installed_package));
                    }
                }
                Err(err) => panic!("{}", err),
            }
        }
    }
}
