use crate::cli::model::Commands;
use crate::cli::output::Std;

enum Uri {
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

#[derive(Debug)]
pub struct Input {
    pub namespace: Option<String>,
    pub paths: Option<Vec<String>>,
    pub uri: String,
}

pub struct Output {
    installed_package: quilt_rs::InstalledPackage,
    package_dir: std::path::PathBuf,
    paths: Option<Vec<std::path::PathBuf>>,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut output = vec![format!(
            "Package {:?} installed to {:?}",
            self.installed_package, self.package_dir,
        )];
        match &self.paths {
            Some(paths) => {
                for path in paths {
                    output.push(format!("Path: {:?}", path));
                }
            }
            None => output.push("No paths installed".to_string()),
        }
        write!(f, "{}", output.join("\n"))
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    match m.package_install(args).await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

async fn install_package_from_remote_manifest(
    local_domain: &quilt_rs::LocalDomain,
    uri: &quilt_rs::S3PackageURI,
    namespace: Option<String>,
) -> Result<(String, quilt_rs::InstalledPackage), String> {
    let namespace = namespace.unwrap_or(uri.namespace.clone());
    let installed_package = local_domain.get_installed_package(&namespace).await?;
    if let Some(installed_package) = installed_package {
        // FIXME: check the actual remote_manifest
        return Ok((namespace, installed_package));
    }
    let remote_manifest = quilt_rs::RemoteManifest::resolve(uri).await?;
    Ok((
        namespace,
        local_domain.install_package(&remote_manifest).await?,
    ))
}

async fn install_paths(
    installed_package: &quilt_rs::InstalledPackage,
    paths: Vec<String>,
) -> Result<Vec<String>, String> {
    installed_package.install_paths(&paths).await?;
    Ok(paths)
}

struct Entries {
    paths: Option<Vec<std::path::PathBuf>>,
    keys: Option<Vec<String>>,
}

fn get_entries(
    root: &std::path::Path,
    uri_path: Option<String>,
    arg_paths: Option<Vec<String>>,
) -> Entries {
    let mut keys = Vec::new();
    let mut paths = Vec::new();
    if uri_path.is_some() {
        let logical_key = uri_path.unwrap();
        paths.push(root.to_path_buf().join(&logical_key));
        keys.push(logical_key);
    }
    if arg_paths.is_some() {
        let logical_keys = arg_paths.unwrap();
        keys.extend_from_slice(&logical_keys);
        for logical_key in logical_keys {
            paths.push(root.to_path_buf().join(logical_key));
        }
    }
    Entries {
        paths: if paths.is_empty() { None } else { Some(paths) },
        keys: if keys.is_empty() { None } else { Some(keys) },
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input {
        uri,
        paths,
        namespace,
    }: Input,
) -> Result<Output, String> {
    match parse_uri(&uri)? {
        Uri::S3PackageURI(uri) => {
            let (namespace, installed_package) =
                install_package_from_remote_manifest(local_domain, &uri, namespace).await?;
            let package_dir = local_domain.working_folder(&namespace);
            let Entries { keys, paths } = get_entries(&package_dir, uri.path, paths);

            if let Some(keys) = keys {
                install_paths(&installed_package, keys).await?;
            }

            Ok(Output {
                installed_package,
                package_dir,
                paths,
            })
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use temp_testdir::TempDir;
    use tokio::sync;

    fn temp_local_domain() -> sync::Mutex<quilt_rs::LocalDomain> {
        let temp_dir = TempDir::default();
        let local_path = PathBuf::from(temp_dir.as_ref());
        println!("Local path, {:?}", local_path);
        sync::Mutex::new(quilt_rs::LocalDomain::new(local_path))
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
            _ => panic!(),
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
            _ => panic!(),
        }
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_installing_package_without_paths() -> Result<(), String> {
        let local_domain = temp_local_domain();
        let local_domain = local_domain.lock().await;
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore".to_string();
        let output = model(
            &local_domain,
            Input {
                uri,
                paths: None,
                namespace: None,
            },
        )
        .await?;
        let status = output.installed_package.status().await?;
        assert_eq!(
            status.upstream_state,
            quilt_rs::quilt::UpstreamDiscreteState::UpToDate
        );
        assert_eq!(output.installed_package.namespace, "spec/quiltcore");
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_installing_package_with_one_path() -> Result<(), String> {
        let local_domain = temp_local_domain();
        let local_domain = local_domain.lock().await;
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore&path=READ%20ME.md".to_string();
        let output = model(
            &local_domain,
            Input {
                uri,
                paths: None,
                namespace: None,
            },
        )
        .await?;
        let lineage = output.installed_package.lineage().await?;
        assert!(lineage.paths.get("READ ME.md").is_some());
        Ok(())
    }

    #[tokio::test]
    #[ignore]
    async fn test_installing_package_with_paths() -> Result<(), String> {
        let local_domain = temp_local_domain();
        let local_domain = local_domain.lock().await;
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore".to_string();
        let output = model(
            &local_domain,
            Input {
                uri,
                paths: Some(vec!["READ ME.md".to_string(), "timestamp.txt".to_string()]),
                namespace: None,
            },
        )
        .await?;
        let lineage = output.installed_package.lineage().await?;
        assert!(lineage.paths.get("timestamp.txt").is_some());
        assert!(lineage.paths.get("READ ME.md").is_some());
        Ok(())
    }
}
