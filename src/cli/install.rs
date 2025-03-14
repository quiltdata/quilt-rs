use std::path::PathBuf;

use quilt_rs::uri::Namespace;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Debug)]
pub struct Input {
    pub namespace: Option<Namespace>,
    pub paths: Option<Vec<PathBuf>>,
    pub uri: String,
}

#[derive(Debug)]
pub struct Output {
    installed_package: quilt_rs::InstalledPackage,
    paths: Vec<std::path::PathBuf>,
}

#[cfg(test)]
impl Output {
    pub fn get_installed_package(self) -> quilt_rs::InstalledPackage {
        self.installed_package
    }
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut output = vec![format!("{}", self.installed_package)];
        if self.paths.is_empty() {
            output.push("No paths installed".to_string())
        } else {
            for path in &self.paths {
                output.push(format!("Path: {:?}", path));
            }
        }
        write!(f, "{}", output.join("\n"))
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.install(args).await)
}

async fn install_package(
    local_domain: &quilt_rs::LocalDomain,
    uri: &quilt_rs::uri::S3PackageUri,
    namespace: Namespace,
) -> Result<quilt_rs::InstalledPackage, Error> {
    let remote = local_domain.get_remote();
    if let Some(installed_package) = local_domain.get_installed_package(&namespace).await? {
        // TODO: check the actual remote_manifest
        return Ok(installed_package);
    }
    let manifest_uri =
        quilt_rs::io::manifest::resolve_manifest_uri(remote, &uri.catalog, uri).await?;
    Ok(local_domain.install_package(&manifest_uri).await?)
}

async fn install_paths(
    installed_package: &quilt_rs::InstalledPackage,
    paths: &Vec<PathBuf>,
) -> Result<(), Error> {
    installed_package.install_paths(paths).await?;
    Ok(())
}

fn get_entries(
    uri_path: Option<PathBuf>,
    arg_paths: Option<Vec<PathBuf>>,
) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Some(logical_key) = uri_path {
        paths.push(logical_key);
    }
    if let Some(logical_keys) = arg_paths {
        for logical_key in logical_keys {
            paths.push(logical_key);
        }
    }
    paths
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input {
        namespace,
        paths,
        uri,
    }: Input,
) -> Result<Output, Error> {
    let uri: quilt_rs::uri::S3PackageUri = uri.parse()?;
    let path = uri.path.clone();

    let namespace = namespace.unwrap_or(uri.namespace.clone());
    let installed_package = install_package(local_domain, &uri, namespace).await?;
    let paths = get_entries(path, paths);

    if !paths.is_empty() {
        install_paths(&installed_package, &paths).await?;
    }

    Ok(Output {
        installed_package,
        paths,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use test_log::test;

    use quilt_rs::io::storage::LocalStorage;
    use quilt_rs::io::storage::Storage;

    use crate::cli::fixtures::packages::default as pkg;
    use crate::cli::model::create_model_in_temp_dir;

    /// Verifies the installation process in CLI with valid data:
    ///   * lineage is updated with the installed package and tracked paths
    ///   * the working directory contains tracked mutable files
    ///   * `.quilt/objects` contains immutable files from the package
    ///   * `.quilt/installed` contains the installed manifest under the namespace directory
    ///   * `.quilt/packages` contains the cached manifest under the bucket directory
    /// Uses an actual manifest from Quilt without mocks.
    #[test(tokio::test)]
    async fn test_model() -> Result<(), Error> {
        let uri = format!("{}&path={}", pkg::URI, pkg::README_LK_ESCAPED);

        let readme_logical_key = PathBuf::from(pkg::README_LK);
        let timestamp_logical_key = PathBuf::from(pkg::TIMESTAMP_LK);

        let (m, temp_dir) = create_model_in_temp_dir().await?;
        let working_dir = temp_dir.path().join(pkg::NAMESPACE_STR);
        {
            let local_domain = m.get_local_domain();

            let output = model(
                local_domain,
                Input {
                    namespace: None,
                    paths: Some(vec![timestamp_logical_key.clone()]),
                    uri,
                },
            )
            .await?;

            assert_eq!(
                format!("{}", output),
                format!(
                    "Installed package \"{}\"\nPath: \"{}\"\nPath: \"{}\"",
                    pkg::NAMESPACE_STR,
                    pkg::README_LK,
                    pkg::TIMESTAMP_LK,
                )
            );

            let installed_package = output.installed_package;
            assert_eq!(installed_package.namespace, (pkg::NAMESPACE).into());
            assert!(installed_package
                .lineage()
                .await?
                .paths
                .contains_key(&readme_logical_key));

            assert_eq!(
                installed_package.working_folder().await?,
                PathBuf::from(temp_dir.as_ref()).join(pkg::NAMESPACE_STR)
            );
            assert_eq!(
                output.paths,
                vec![readme_logical_key.clone(), timestamp_logical_key.clone()]
            );
        }

        let storage = LocalStorage::new();
        assert!(storage.exists(working_dir.join(&readme_logical_key)).await);
        assert!(
            storage
                .exists(working_dir.join(&timestamp_logical_key))
                .await
        );

        assert!(
            storage
                .exists(temp_dir.path().join(format!(
                    ".quilt/installed/{}/{}",
                    pkg::NAMESPACE_STR,
                    pkg::TOP_HASH
                )))
                .await
        );

        assert!(
            storage
                .exists(
                    temp_dir.path().join(".quilt/packages/data-yaml-spec-tests/a4aed21f807f0474d2761ed924a5875cc10fd0cd84617ef8f7307e4b9daebcc7")
                )
                .await
        );

        // README.md
        assert!(
            storage
                .exists(
                    temp_dir.path().join(".quilt/objects/3e5e75033079a0b5bfaeff79c8f10dbc3f461e283ad8126c333cd74792e62ea7")
                )
                .await
        );
        // timestamp.txt
        assert!(
            storage
                .exists(
                    temp_dir.path().join(".quilt/objects/dc3ea61d9a4aaf7d822eed1de089db83d46aa29f3fbdd99466f7e5e216c91c8a")
                )
                .await
        );

        {
            let local_domain = m.get_local_domain();
            let install_once_more = model(
                local_domain,
                Input {
                    namespace: None,
                    paths: None,
                    uri: pkg::URI.to_string(),
                },
            )
            .await?;

            // No paths installed during this call
            assert_eq!(
                format!("{}", install_once_more),
                format!(
                    "Installed package \"{}\"\nNo paths installed",
                    pkg::NAMESPACE_STR,
                )
            );

            assert_eq!(
                install_once_more.installed_package.namespace,
                pkg::NAMESPACE.into(),
            );

            // However paths are still tracked, because we didn't install a package anew,
            // but re-use installed package from the previous call.
            let paths = install_once_more.installed_package.lineage().await?.paths;
            assert!(paths.contains_key(&readme_logical_key));
            assert!(paths.contains_key(&timestamp_logical_key));
        }

        Ok(())
    }
}
