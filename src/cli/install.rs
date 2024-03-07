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
    package_folder: std::path::PathBuf,
    paths: Option<Vec<std::path::PathBuf>>,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut output = vec![format!(
            "Package {:?} installed to {:?}",
            self.installed_package, self.package_folder,
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
            let namespace = namespace.or(Some(uri.namespace.clone()));
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
                return Ok(Output {
                    installed_package,
                    package_folder,
                    paths: Some(paths_output),
                });
            }

            if paths.is_some() {
                let paths_strings = paths.unwrap();
                installed_package.install_paths(&paths_strings).await?;
                for path in paths_strings {
                    paths_output.push(package_folder.clone().join(path));
                }
                return Ok(Output {
                    installed_package,
                    package_folder,
                    paths: Some(paths_output),
                });
            }
            if paths_output.is_empty() {
                Ok(Output {
                    installed_package,
                    package_folder,
                    paths: None,
                })
            } else {
                Ok(Output {
                    installed_package,
                    package_folder,
                    paths: Some(paths_output),
                })
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
