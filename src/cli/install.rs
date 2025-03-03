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
    match m.install(args).await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

async fn install_package(
    local_domain: &quilt_rs::LocalDomain,
    uri: &quilt_rs::uri::S3PackageUri,
    namespace: Option<Namespace>,
) -> Result<quilt_rs::InstalledPackage, Error> {
    let remote = local_domain.get_remote();
    let namespace = namespace.unwrap_or(uri.namespace.clone());
    if let Some(installed_package) = local_domain.get_installed_package(&namespace).await? {
        // FIXME: check the actual remote_manifest
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

    use crate::cli::fixtures;
    use crate::cli::model::Model;

    /// Verifies the installation process in CLI with valid data:
    ///   * lineage is updated with the installed package and tracked paths
    ///   * the working directory contains tracked mutable files
    ///   * `.quilt/objects` contains immutable files from the package
    ///   * `.quilt/installed` contains the installed manifest under the namespace directory
    ///   * `.quilt/packages` contains the cached manifest under the bucket directory
    /// Uses an actual manifest from Quilt without mocks.
    #[test(tokio::test)]
    async fn test_model() -> Result<(), Error> {
        let uri = "quilt+s3://data-yaml-spec-tests#package=reference/quilt-rs&path=one/two%20two/three%20three%20three/READ%20ME.md".to_string();

        let readme_logical_key = PathBuf::from("one/two two/three three three/READ ME.md");
        let timestamp_logical_key = PathBuf::from("timestamp.txt");

        let (m, temp_dir) = Model::from_temp_dir()?;
        let working_dir = temp_dir.path().join("reference/quilt-rs");
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
                "Installed package \"reference/quilt-rs\" at {}/reference/quilt-rs\nPath: \"one/two two/three three three/READ ME.md\"\nPath: \"timestamp.txt\"",
                temp_dir.path().display()
            )
        );

            let installed_package = output.installed_package;
            assert_eq!(
                installed_package.namespace,
                ("reference", "quilt-rs").into()
            );
            assert!(installed_package
                .lineage()
                .await?
                .paths
                .contains_key(&readme_logical_key));

            assert_eq!(
                installed_package.working_folder(),
                PathBuf::from(temp_dir.as_ref()).join("reference/quilt-rs")
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
                    fixtures::DEFAULT_NAMESPACE,
                    fixtures::DEFAULT_TOP_HASH
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
                uri: "quilt+s3://data-yaml-spec-tests#package=reference/quilt-rs@a4aed21f807f0474d2761ed924a5875cc10fd0cd84617ef8f7307e4b9daebcc7".to_string(),
            },
        )
        .await?;

            // No paths installed during this call
            assert_eq!(
                format!("{}", install_once_more),
                format!(
                    "Installed package \"reference/quilt-rs\" at {}/reference/quilt-rs\nNo paths installed",
                    temp_dir.path().display()
                )
            );

            assert_eq!(
                install_once_more.installed_package.namespace,
                ("reference", "quilt-rs").into(),
            );

            // However paths are still tracked, because we didn't install a package anew,
            // but re-use installed package from the previous call.
            let paths = install_once_more.installed_package.lineage().await?.paths;
            assert!(paths.contains_key(&readme_logical_key));
            assert!(paths.contains_key(&timestamp_logical_key));
        }

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_valid_command() -> Result<(), Error> {
        let uri = fixtures::DEFAULT_PACKAGE_URI.to_string();

        let (model, temp_dir) = Model::from_temp_dir()?;

        if let Std::Out(output_str) = command(
            model,
            Input {
                namespace: None,
                paths: None,
                uri,
            },
        )
        .await
        {
            assert_eq!(
                output_str,
                format!(
                    "Installed package \"{}\" at {}/{}\nNo paths installed",
                    fixtures::DEFAULT_NAMESPACE,
                    temp_dir.path().display(),
                    fixtures::DEFAULT_NAMESPACE,
                )
            );
        } else {
            return Err(Error::Test("Failed to install".to_string()));
        }

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_invalid_command() -> Result<(), Error> {
        let uri = "quilt+s3://some-nonsense".to_string();

        let (model, _temp_dir) = Model::from_temp_dir()?;

        if let Std::Err(error_str) = command(
            model,
            Input {
                namespace: None,
                paths: None,
                uri,
            },
        )
        .await
        {
            assert_eq!(
                format!("{}", error_str),
                "quilt_rs error: Invalid package URI: S3 package URI must contain a fragment: quilt+s3://some-nonsense".to_string()
            );
        } else {
            return Err(Error::Test("Failed to fail".to_string()));
        }

        Ok(())
    }
}
