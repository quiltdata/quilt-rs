use clap::{Parser, Subcommand};

use std::path::Path;
use tokio::sync;

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

pub enum Uri {
    S3PackageURI(quilt_rs::S3PackageURI),
    S3URI(quilt_rs::quilt::storage::s3::S3Uri),
}

fn parse_uri(uri: &str) -> Result<Uri, String> {
    if uri.starts_with("quilt+s3") {
        return Ok(Uri::S3PackageURI(quilt_rs::S3PackageURI::try_from(uri)?));
    }
    if uri.starts_with("s3") {
        return Ok(Uri::S3URI(quilt_rs::quilt::storage::s3::S3Uri::try_from(
            uri,
        )?));
    }
    Err("Invalid scheme".into())
}

struct PackageInstallArgs {
    namespace: Option<String>,
    paths: Option<Vec<String>>,
    uri_str: String,
}

struct BrowseRemoteManifestArgs {
    uri_str: String,
}

#[derive(Debug)]
enum Std {
    Out(String),
    Err(String),
}

fn print_stdout_v2(output: Std) {
    match output {
        Std::Out(str) => println!("{}", str),
        Std::Err(str) => tracing::error!("{}", str),
    }
}

struct Model {
    local_domain: sync::Mutex<quilt_rs::LocalDomain>,
}

trait CommandsModel {
    fn get_local_domain(&self) -> &sync::Mutex<quilt_rs::LocalDomain>;

    async fn browse_remote_manifest(
        &self,
        BrowseRemoteManifestArgs { uri_str }: BrowseRemoteManifestArgs,
    ) -> Result<quilt_rs::Table, String> {
        let uri = quilt_rs::S3PackageURI::try_from(uri_str.as_str())?;
        let remote_manifest = quilt_rs::RemoteManifest::resolve(&uri).await?;
        let local_domain = &self.get_local_domain().lock().await;
        local_domain.browse_remote_manifest(&remote_manifest).await
    }

    async fn package_install(
        &self,
        PackageInstallArgs {
            uri_str,
            paths,
            namespace,
        }: PackageInstallArgs,
    ) -> Result<
        (
            quilt_rs::InstalledPackage,
            std::path::PathBuf,
            Option<Vec<std::path::PathBuf>>,
        ),
        String,
    > {
        match parse_uri(&uri_str)? {
            Uri::S3PackageURI(uri) => {
                let namespace = namespace.or(Some(uri.namespace.clone()));
                let local_domain = &self.get_local_domain().lock().await;
                let installed_package = match local_domain
                    .get_installed_package(&namespace.unwrap())
                    .await?
                {
                    // FIXME: check the actual remote_manifest
                    Some(i) => i,
                    None => {
                        let remote_manifest = quilt_rs::RemoteManifest::resolve(&uri).await?;
                        local_domain.install_package(&remote_manifest).await?
                    }
                };

                let package_folder = local_domain.working_folder(&uri.namespace);
                let mut paths_output = Vec::new();

                if uri.path.is_some() {
                    let paths_strings = vec![uri.path.unwrap()];
                    installed_package.install_paths(&paths_strings).await?;
                    for path in paths_strings {
                        paths_output.push(package_folder.clone().join(path));
                    }
                    return Ok((installed_package, package_folder, Some(paths_output)));
                }

                if paths.is_some() {
                    let paths_strings = paths.unwrap();
                    installed_package.install_paths(&paths_strings).await?;
                    for path in paths_strings {
                        paths_output.push(package_folder.clone().join(path));
                    }
                    return Ok((installed_package, package_folder, Some(paths_output)));
                }
                if paths_output.is_empty() {
                    Ok((installed_package, package_folder, None))
                } else {
                    Ok((installed_package, package_folder, Some(paths_output)))
                }
            }
            Uri::S3URI(uri) => {
                if namespace.is_none() {
                    panic!("Namespace is required when using s3:// URLs");
                }
                println!("package_s3_prefix {:?}", uri);
                quilt_rs::quilt::package_s3_prefix(&namespace.unwrap(), &uri).await;
                Err("FIXME: Should return installed package".into())
            }
        }
    }

    async fn get_installed_packages_list(&self) -> Result<Vec<quilt_rs::InstalledPackage>, String> {
        let local_domain = &self.get_local_domain().lock().await;
        local_domain.list_installed_packages().await
    }
}

impl CommandsModel for Model {
    fn get_local_domain(&self) -> &sync::Mutex<quilt_rs::LocalDomain> {
        &self.local_domain
    }
}

impl Model {
    fn new(local_domain: quilt_rs::LocalDomain) -> Self {
        Model {
            local_domain: sync::Mutex::new(local_domain),
        }
    }
}

async fn command_browse_remote_manifest(
    model: impl CommandsModel,
    args: BrowseRemoteManifestArgs,
) -> Std {
    match model.browse_remote_manifest(args).await {
        Ok(manifest_contents) => {
            let mut output: Vec<String> = Vec::new();
            let mut header_table = tabled::Table::new(vec![RemoteManifestHeader {
                info: manifest_contents.header.info.to_string(),
                meta: manifest_contents.header.meta.to_string(),
            }]);
            header_table.with(tabled::settings::Panel::header("Remote manifest header"));
            output.push(header_table.to_string());

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
            output.push(entries_table.to_string());
            Std::Out(output.join("\n"))
        }
        Err(err) => Std::Err(err),
    }
}

async fn command_package_install(model: impl CommandsModel, args: PackageInstallArgs) -> Std {
    match model.package_install(args).await {
        Ok((installed_package, package_folder, _)) => Std::Out(format!(
            "Package {:?} installed to {:?}",
            installed_package, package_folder,
        )),
        Err(err) => Std::Err(err),
    }
}

async fn command_get_installed_packages_list(model: impl CommandsModel) -> Std {
    match model.get_installed_packages_list().await {
        Ok(installed_packages_list) => {
            let mut output: Vec<String> = Vec::new();
            for installed_package in installed_packages_list {
                output.push(format!("InstalledPackage<{}>", installed_package.namespace));
            }
            Std::Out(output.join("\n"))
        }
        Err(err) => Std::Err(err),
    }
}

pub async fn init() {
    let args = Args::parse();

    let local_domain = quilt_rs::LocalDomain::new(Path::new(&args.domain).to_path_buf());
    let model = Model::new(local_domain.clone());

    match args.command {
        Commands::Browse { uri: uri_str } => {
            tracing::debug!("Browsing {}", uri_str);
            let args = BrowseRemoteManifestArgs { uri_str };
            let output = command_browse_remote_manifest(model, args).await;
            print_stdout_v2(output);
        }
        Commands::Install {
            path,
            namespace,
            uri: uri_str,
        } => {
            tracing::debug!("Installing {}", uri_str);
            let args = PackageInstallArgs {
                uri_str,
                paths: path,
                namespace,
            };
            let output = command_package_install(model, args).await;
            print_stdout_v2(output);
        }
        Commands::List => {
            tracing::debug!("Listing installed packages");
            let output = command_get_installed_packages_list(model).await;
            print_stdout_v2(output);
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
        let uri_str = "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md".to_string();
        let model = Model::new(local_domain.clone());
        let table = model
            .browse_remote_manifest(BrowseRemoteManifestArgs { uri_str })
            .await?;
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
    #[ignore]
    async fn install() -> Result<(), String> {
        let local_domain = temp_local_domain();
        let uri_str = "quilt+s3://udp-spec#package=spec/quiltcore".to_string();
        let model = Model::new(local_domain.clone());
        let (installed_package, _, _) = model
            .package_install(PackageInstallArgs {
                uri_str,
                paths: None,
                namespace: None,
            })
            .await?;
        let status = installed_package.status().await?;
        assert_eq!(
            status.upstream_state,
            quilt_rs::quilt::UpstreamDiscreteState::UpToDate
        );
        assert_eq!(installed_package.namespace, "spec/quiltcore");
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn install_path() -> Result<(), String> {
        let local_domain = temp_local_domain();
        let uri_str = "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md".to_string();
        let model = Model::new(local_domain.clone());
        let (installed_package, _, _) = model
            .package_install(PackageInstallArgs {
                uri_str,
                paths: None,
                namespace: None,
            })
            .await?;
        let lineage = installed_package.lineage().await?;
        assert!(lineage.paths.get("READ ME.md").is_some());
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn install_paths() -> Result<(), String> {
        let local_domain = temp_local_domain();
        let uri_str = "quilt+s3://udp-spec#package=spec/quiltcore".to_string();
        let model = Model::new(local_domain.clone());
        let (installed_package, _, _) = model
            .package_install(PackageInstallArgs {
                uri_str,
                paths: Some(vec!["READ ME.md".to_string(), "timestamp.txt".to_string()]),
                namespace: None,
            })
            .await?;
        let lineage = installed_package.lineage().await?;
        assert!(lineage.paths.get("timestamp.txt").is_some());
        assert!(lineage.paths.get("READ ME.md").is_some());
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn list() -> Result<(), String> {
        let local_domain = temp_local_domain();
        let uri_str = "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md".to_string();
        let model = Model::new(local_domain.clone());
        let _ = model
            .package_install(PackageInstallArgs {
                uri_str,
                namespace: None,
                paths: None,
            })
            .await?;
        let list = model.get_installed_packages_list().await?;
        assert_eq!(list[0].namespace, "spec/quiltcore");
        Ok(())
    }

    #[test]
    fn test_parse_uri() -> Result<(), String> {
        let uri = parse_uri("quilt+s3://bucket#package=foo/bar")?;
        match uri {
            Uri::S3PackageURI(matched_uri) => {
                assert_eq!(
                    matched_uri,
                    quilt_rs::S3PackageURI {
                        bucket: "bucket".to_string(),
                        namespace: "foo/bar".to_string(),
                        revision: quilt_rs::quilt::RevisionPointer::Tag(String::from("latest")),
                        path: None,
                    }
                );
            }
            _ => assert!(false),
        }

        let uri = parse_uri("s3://bucket/foo/bar")?;
        match uri {
            Uri::S3URI(matched_uri) => {
                assert_eq!(
                    matched_uri,
                    quilt_rs::quilt::storage::s3::S3Uri {
                        bucket: "bucket".to_string(),
                        key: "foo/bar".to_string(),
                        version: None,
                    }
                );
            }
            _ => assert!(false),
        }
        Ok(())
    }
}
