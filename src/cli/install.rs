use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Debug)]
pub struct Input {
    pub namespace: Option<String>,
    pub paths: Option<Vec<String>>,
    pub uri: quilt_rs::S3PackageURI,
}

#[derive(Debug)]
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
    match m.install(args).await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

async fn install_package(
    local_domain: &quilt_rs::LocalDomain,
    uri: &quilt_rs::S3PackageURI,
    namespace: Option<String>,
) -> Result<(quilt_rs::InstalledPackage, String), Error> {
    let namespace = namespace.unwrap_or(uri.namespace.clone());
    let installed_package = local_domain.get_installed_package(&namespace).await?;
    if let Some(installed_package) = installed_package {
        // FIXME: check the actual remote_manifest
        return Ok((installed_package, namespace));
    }
    let remote_manifest = quilt_rs::RemoteManifest::resolve(uri).await?;
    Ok((
        local_domain.install_package(&remote_manifest).await?,
        namespace,
    ))
}

async fn install_paths(
    installed_package: &quilt_rs::InstalledPackage,
    paths: Vec<String>,
) -> Result<Vec<String>, Error> {
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
        namespace,
        paths,
        uri,
    }: Input,
) -> Result<Output, Error> {
    let (installed_package, namespace) = install_package(local_domain, &uri, namespace).await?;
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

#[cfg(test)]
mod tests {
    #[tokio::test]
    #[ignore]
    async fn install() -> Result<(), String> {
        unreachable!()
    }
}
