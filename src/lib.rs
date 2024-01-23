mod quilt4;
mod s3_utils;

pub mod data_yaml;
pub mod quilt;

pub use quilt4::{
    client::{Client, GetClient},
    domain::Domain,
    entry::Entry4,
    manifest::Manifest4,
    namespace::Namespace,
    row4::Row4,
    string_map::StringMap,
    table::Table,
    upath::UPath,
    uri::UriParser,
    uri::UriQuilt,
};

pub use quilt::{InstalledPackage, LocalDomain, Manifest, RemoteManifest, S3PackageURI};

use temp_dir::TempDir;

pub async fn install_temporarily(
    bucket: &str,
    namespace: &str,
    hash: &str,
) -> Result<InstalledPackage, String> {
    let temp_folder = TempDir::new().unwrap();
    let loc = LocalDomain::new(temp_folder.path().to_path_buf());
    let remote_manifest = RemoteManifest {
        bucket: bucket.to_string(),
        namespace: namespace.to_string(),
        hash: hash.to_string(),
    };
    tracing::info!("remote_manifest: {:?}", remote_manifest);

    let result = loc.install_package(&remote_manifest).await;
    tracing::info!("result: {:?}", result);
    result
}

pub async fn installed_packages(dir: Option<String>) -> Result<Vec<InstalledPackage>, String> {
    let path_buf = match dir {
        Some(dir) => std::path::PathBuf::from(dir),
        None => std::env::current_dir().unwrap(),
    };
    let local_domain = LocalDomain::new(path_buf);
    println!("local_domain: {:?}", local_domain);
    let installed_packages = local_domain
        .list_installed_packages()
        .await
        .expect("Failed to list installed packages");
    Ok(installed_packages)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_installed_packages_in_cwd() {
        let result = installed_packages(None).await;
        assert!(result.is_ok());
        let packages = result.unwrap();
        let count = packages.len();
        assert_eq!(count, 0);
    }

    #[tokio::test]
    async fn test_installed_packages_in_test_domain() {
        let dir = utils::TEST_DOMAIN.to_string();
        let result = installed_packages(Some(dir.to_string())).await;
        assert!(result.is_ok());
        let packages = result.unwrap();
        println!("packages[{}]: {:?}", utils::TEST_DOMAIN, packages);
        let count = packages.len();
        assert!(count == 0); // TODO: add data.json to fix this
    }
}
