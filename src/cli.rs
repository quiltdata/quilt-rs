use clap::{Parser, Subcommand};

use std::path::Path;

#[derive(tabled::Tabled)]
struct RemoteManifestHeader {
    info: String,
    meta: String,
}

#[derive(tabled::Tabled)]
struct RemoteManifestEntry {
    name: String,
    place: String,
    size: u64,
}

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
    Browse {
        #[arg(short, long)]
        uri: String,
    },
    /// Install package
    Install {
        #[arg(short, long)]
        path: Option<Vec<String>>,
        uri: String,
    },
    // TODO: add as parameter to Install command
    /// Install package to installed package
    InstallPath {
        #[arg(short, long)]
        namespace: String,
        #[arg(short, long)]
        path: String,
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
    paths: Option<Vec<String>>,
) -> Result<(quilt_rs::InstalledPackage, Option<Vec<String>>), String> {
    let uri = quilt_rs::S3PackageURI::try_from(uri_str)?;
    let remote_manifest = quilt_rs::RemoteManifest::resolve(&uri).await?;
    let installed_package = local_domain.install_package(&remote_manifest).await?;
    if paths.is_some() {
        let paths_strings = paths.unwrap();
        installed_package.install_paths(&paths_strings).await?;
        // TODO: + join path
        // Ok(local_domain.working_folder(namespace))
        return Ok((installed_package, Some(paths_strings)));
    }
    Ok((installed_package, None))
}

async fn package_install_path(
    local_domain: &quilt_rs::LocalDomain,
    namespace: &str,
    path: &str,
) -> Result<std::path::PathBuf, String> {
    let installed_package = local_domain
        .get_installed_package(namespace)
        .await?
        .expect("Package not found");
    let paths = vec![path.to_string()];
    installed_package.install_paths(&paths).await?;
    // TODO: + join path
    Ok(local_domain.working_folder(namespace))
}

async fn get_installed_packages_list(
    local_domain: &quilt_rs::LocalDomain,
) -> Result<Vec<quilt_rs::InstalledPackage>, String> {
    local_domain.list_installed_packages().await
}

fn print_stdout(str: String) {
    println!("{}", str);
}

pub async fn init() {
    let args = Args::parse();

    let local_domain = quilt_rs::LocalDomain::new(Path::new(&args.domain).to_path_buf());

    match args.command {
        Commands::Browse { uri: uri_string } => {
            tracing::debug!("Browsing {}", uri_string);
            match browse_remote_manifest(&local_domain, uri_string.as_str()).await {
                Ok(manifest_contents) => {
                    let mut header_table = tabled::Table::new(vec![RemoteManifestHeader {
                        info: manifest_contents.header.info.to_string(),
                        meta: manifest_contents.header.meta.to_string(),
                    }]);
                    header_table.with(tabled::settings::Panel::header("Remote manifest header"));
                    print_stdout(header_table.to_string());

                    let mut entries = Vec::new();
                    for (_name, entry) in manifest_contents.records {
                        entries.push(RemoteManifestEntry {
                            name: entry.name.to_string(),
                            place: entry.place.to_string(),
                            size: entry.size,
                        });
                    }
                    let mut entries_table = tabled::Table::new(&entries);
                    entries_table.with(tabled::settings::Panel::header("Remote manifest entries"));
                    print_stdout(entries_table.to_string());
                }

                Err(err) => panic!("{}", err),
            }
        }
        Commands::Install {
            path,
            uri: uri_string,
        } => {
            tracing::debug!("Installing {}", uri_string);
            println!("PATH: {:?}", path);
            match package_install(&local_domain, uri_string.as_str(), None).await {
                Ok((installed_package, _)) => print_stdout(format!(
                    "Package {:?} installed to {:?}",
                    installed_package,
                    local_domain.working_folder(&installed_package.namespace),
                )),
                Err(err) => panic!("{}", err),
            }
        }
        Commands::InstallPath { namespace, path } => {
            tracing::debug!("Installing path {} to {}", path, namespace);
            match package_install_path(&local_domain, namespace.as_str(), path.as_str()).await {
                Ok(resolved_path) => print_stdout(format!(
                    "Path {:?} installed to the package {}",
                    resolved_path, namespace,
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use temp_testdir::TempDir;

    fn temp_local_domain() -> quilt_rs::LocalDomain {
        let temp_dir = TempDir::default();
        let local_path = PathBuf::from(temp_dir.as_ref());
        println!("Local path, {:?}", local_path);
        quilt_rs::LocalDomain::new(local_path)
    }

    #[tokio::test]
    async fn browse() -> Result<(), String> {
        let local_domain = temp_local_domain();
        let uri_str = "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md";
        let table = browse_remote_manifest(&local_domain, uri_str).await?;
        assert_eq!(
            table.header.info,
            serde_json::json!({
                "message": "test_spec_write 1697916638",
                "version":"v0"
            })
        );
        assert_eq!(
            table.records.get("READ ME.md").unwrap().place,
            "s3://udp-spec/spec/quiltcore/READ%20ME.md?versionId=.l3tAGbfEBC4c.L2ywTpWbnweSpYLe8a"
        );
        assert_eq!(
            table.records.get("timestamp.txt").unwrap().place,
            "s3://udp-spec/spec/quiltcore/timestamp.txt?versionId=lifktjQgrgewg1FGXxls3UKtJSjl2shy"
        );
        Ok(())
    }

    #[tokio::test]
    async fn install() -> Result<(), String> {
        let local_domain = temp_local_domain();
        let uri_str = "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md";
        let (installed_package, _) = package_install(&local_domain, uri_str, None).await?;
        let status = installed_package.status().await?;
        assert_eq!(
            status.upstream_state,
            quilt_rs::quilt::UpstreamDiscreteState::UpToDate
        );
        assert_eq!(installed_package.namespace, "spec/quiltcore");
        Ok(())
    }

    #[tokio::test]
    async fn list() -> Result<(), String> {
        let local_domain = temp_local_domain();
        let uri_str = "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md";
        let _ = package_install(&local_domain, uri_str, None).await?;
        let list = get_installed_packages_list(&local_domain).await?;
        assert_eq!(list[0].namespace, "spec/quiltcore");
        Ok(())
    }

    #[tokio::test]
    async fn install_path() -> Result<(), String> {
        let local_domain = temp_local_domain();
        let uri_str = "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md";
        let _ = package_install(&local_domain, uri_str, None).await?;
        let _ = package_install_path(&local_domain, "spec/quiltcore", "timestamp.txt").await;
        Ok(())
    }
}
