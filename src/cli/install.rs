use std::path::PathBuf;

use quilt_rs::quilt::uri::Namespace;

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
    package_dir: std::path::PathBuf,
    paths: Vec<std::path::PathBuf>,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut output = vec![format!(
            "Package {:?} installed to {:?}",
            self.installed_package, self.package_dir,
        )];
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
    uri: &quilt_rs::S3PackageUri,
    namespace: Option<Namespace>,
) -> Result<quilt_rs::InstalledPackage, Error> {
    let remote = quilt_rs::s3_utils::RemoteS3::new();
    let namespace = namespace.unwrap_or(uri.namespace.clone());
    let installed_package = local_domain.get_installed_package(namespace).await?;
    if let Some(installed_package) = installed_package {
        // FIXME: check the actual remote_manifest
        return Ok(installed_package);
    }
    let remote_manifest = quilt_rs::RemoteManifest::resolve(&remote, uri).await?;
    Ok(local_domain.install_package(&remote_manifest).await?)
}

async fn install_paths(
    installed_package: &quilt_rs::InstalledPackage,
    paths: &Vec<PathBuf>,
) -> Result<(), Error> {
    installed_package.install_paths(paths).await?;
    Ok(())
}

fn get_entries(
    root: &std::path::Path,
    uri_path: Option<PathBuf>,
    arg_paths: Option<Vec<PathBuf>>,
) -> Vec<std::path::PathBuf> {
    let mut paths = Vec::new();
    if let Some(logical_key) = uri_path {
        paths.push(root.to_path_buf().join(logical_key));
    }
    if arg_paths.is_some() {
        let logical_keys = arg_paths.unwrap();
        for logical_key in logical_keys {
            paths.push(root.to_path_buf().join(logical_key));
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
    let uri: quilt_rs::S3PackageUri = uri.parse()?;
    let installed_package = install_package(local_domain, &uri, namespace).await?;
    let package_dir = installed_package.working_folder();
    let paths = get_entries(&package_dir, uri.path, paths);

    if !paths.is_empty() {
        install_paths(&installed_package, &paths).await?;
    }

    Ok(Output {
        installed_package,
        package_dir,
        paths,
    })
}

#[cfg(test)]
mod tests {
    #[tokio::test]
    #[ignore]
    async fn install() -> Result<(), String> {
        unreachable!()
    }
}
