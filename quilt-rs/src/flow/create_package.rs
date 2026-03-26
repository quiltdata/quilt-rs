use std::path::Path;
use std::path::PathBuf;

use tracing::debug;
use tracing::info;

use crate::io::storage::Storage;
use crate::lineage::DomainLineage;
use crate::lineage::PackageLineage;
use crate::lineage::RemotePackage;
use crate::paths::DomainPaths;
use crate::quiltignore;
use crate::uri::S3PackageUri;
use crate::Error;
use crate::Res;

async fn copy_source_dir(
    storage: &(impl Storage + Sync),
    source: &Path,
    destination: &Path,
) -> Res {
    let quiltignore = quiltignore::load(source)?;
    let mut queue = vec![source.to_path_buf()];

    while let Some(dir) = queue.pop() {
        let mut entries = storage.read_dir(&dir).await?;
        while let Some(entry) = entries.next_entry().await? {
            let path = entry.path();
            let relative = path.strip_prefix(source)?;
            let file_type = entry.file_type().await?;

            if let Some(gi) = quiltignore.as_ref() {
                if quiltignore::is_ignored(gi, relative, file_type.is_dir()) {
                    continue;
                }
            }

            let dest_path = destination.join(relative);
            if file_type.is_dir() {
                storage.create_dir_all(&dest_path).await?;
                queue.push(path);
            } else if file_type.is_file() {
                if let Some(parent) = dest_path.parent() {
                    storage.create_dir_all(parent).await?;
                }
                storage.copy(path, &dest_path).await?;
            }
        }
    }

    Ok(())
}

pub async fn create_package(
    lineage: DomainLineage,
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    uri: &S3PackageUri,
    source: Option<&PathBuf>,
) -> Res<DomainLineage> {
    info!("⏳ Creating package {}", uri.display());

    if lineage.packages.contains_key(&uri.namespace) {
        return Err(Error::PackageAlreadyInstalled(uri.namespace.clone()));
    }

    let home = lineage.home.clone();
    paths
        .scaffold_for_installing(storage, &home, &uri.namespace)
        .await?;
    paths.scaffold_for_caching(storage, &uri.bucket).await?;

    let package_home = home.join(uri.namespace.to_string());
    if let Some(source) = source {
        debug!(
            "⏳ Copying source directory {} into {}",
            source.display(),
            package_home.display()
        );
        copy_source_dir(storage, source, &package_home).await?;
    }

    let mut lineage = lineage;
    lineage.packages.insert(
        uri.namespace.clone(),
        PackageLineage::from_package(RemotePackage {
            origin: uri.catalog.clone(),
            bucket: uri.bucket.clone(),
            namespace: uri.namespace.clone(),
        }),
    );

    Ok(lineage)
}

