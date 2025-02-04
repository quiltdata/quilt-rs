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
    let remote = quilt_rs::io::remote::RemoteS3::new();
    let namespace = namespace.unwrap_or(uri.namespace.clone());
    if let Some(installed_package) = local_domain.get_installed_package(&namespace).await? {
        // FIXME: check the actual remote_manifest
        return Ok(installed_package);
    }
    let manifest_uri = quilt_rs::io::manifest::resolve_manifest_uri(&remote, uri).await?;
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
    if arg_paths.is_some() {
        let logical_keys = arg_paths.unwrap();
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
    use tempfile::TempDir;

    use quilt_rs::io::storage::LocalStorage;
    use quilt_rs::io::storage::Storage;

    use crate::cli::model::Model;

    /// Verifies the installation process in CLI with valid data:
    ///   * lineage is updated with the installed package and tracked paths
    ///   * the working directory contains tracked mutable files
    ///   * `.quilt/objects` contains immutable files from the package
    ///   * `.quilt/installed` contains the installed manifest under the namespace directory
    ///   * `.quilt/packages` contains the cached manifest under the bucket directory
    /// Uses an actual manifest from Quilt without mocks.
    #[tokio::test]
    async fn test_model() -> Result<(), Error> {
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore@44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c&path=READ%20ME.md".to_string();

        let readme_logical_key = PathBuf::from("READ ME.md");
        let timestamp_logical_key = PathBuf::from("timestamp.txt");

        let temp_dir = TempDir::default();
        let local_path = PathBuf::from(temp_dir.as_ref());
        let local_domain = quilt_rs::LocalDomain::new(local_path);

        let output = model(
            &local_domain,
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
                "Installed package \"spec/quiltcore\" at {}/spec/quiltcore\nPath: \"READ ME.md\"\nPath: \"timestamp.txt\"",
                temp_dir.display()
            )
        );

        let installed_package = output.installed_package;
        assert_eq!(installed_package.namespace, ("spec", "quiltcore").into());
        assert!(installed_package
            .lineage()
            .await?
            .paths
            .contains_key(&readme_logical_key));

        let working_dir = temp_dir.join("spec/quiltcore");
        assert_eq!(
            installed_package.working_folder(),
            PathBuf::from(temp_dir.as_ref()).join("spec/quiltcore")
        );
        assert_eq!(
            output.paths,
            vec![readme_logical_key.clone(), timestamp_logical_key.clone()]
        );

        let storage = LocalStorage::new();
        assert!(storage.exists(working_dir.join(readme_logical_key)).await);
        assert!(
            storage
                .exists(working_dir.join(timestamp_logical_key))
                .await
        );

        assert!(
            storage
                .exists(temp_dir.join(".quilt/installed/spec/quiltcore/44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c"))
                .await
        );

        assert!(
            storage
                .exists(
                    temp_dir.join(".quilt/packages/udp-spec/44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c")
                )
                .await
        );

        // README.md
        assert!(
            storage
                .exists(
                    temp_dir.join(".quilt/objects/e1181788c8a77224d98bb3a2de256bfea1d2f128019d5d378406522c03b5db07")
                )
                .await
        );
        // timestamp.txt
        assert!(
            storage
                .exists(
                    temp_dir.join(".quilt/objects/1f580d4f3e2545b95054993a6d66e802dc81140a9a42702c8aa088f00091cab2")
                )
                .await
        );

        let install_once_more = model(
            &local_domain,
            Input {
                namespace: None,
                paths: None,
                uri: "quilt+s3://udp-spec#package=spec/quiltcore@44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c".to_string(),
            },
        )
        .await?;

        // No paths installed during this call
        assert_eq!(
            format!("{}", install_once_more),
            format!(
                "Installed package \"spec/quiltcore\" at {}/spec/quiltcore\nNo paths installed",
                temp_dir.display()
            )
        );

        assert_eq!(
            installed_package.namespace,
            install_once_more.installed_package.namespace
        );

        // However paths are still tracked, because we didn't install a package anew,
        // but re-use installed package from the previous call.
        assert_eq!(
            installed_package.lineage().await?.paths,
            install_once_more.installed_package.lineage().await?.paths
        );

        Ok(())
    }

    #[tokio::test]
    async fn test_valid_command() -> Result<(), Error> {
        let uri = "quilt+s3://udp-spec#package=spec/quiltcore@44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c".to_string();

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
                    "Installed package \"spec/quiltcore\" at {}/spec/quiltcore\nNo paths installed",
                    temp_dir.path().display()
                )
            );
        } else {
            return Err(Error::Test("Failed to install".to_string()));
        }

        Ok(())
    }

    #[tokio::test]
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
