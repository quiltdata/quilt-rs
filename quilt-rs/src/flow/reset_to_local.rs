use std::collections::BTreeMap;
use std::io::ErrorKind;
use std::path::PathBuf;

use tracing::{debug, info};

use crate::Error;
use crate::Res;
use crate::error::PackageOpError;
use crate::io::storage::Storage;
use crate::lineage::{CommitState, PackageLineage, PathState};
use crate::manifest::Manifest;
use crate::paths::DomainPaths;
use quilt_uri::Namespace;

/// Discard the newest local commit and restore the previous local manifest.
///
/// Local commits keep their complete parent chain in [`CommitState::prev_hashes`]
/// and retain both manifests and immutable objects on disk. Reset therefore
/// needs no remote: it removes the current working copy, copies the previous
/// revision's objects back, and moves the lineage pointer to that revision.
pub async fn reset_to_local(
    mut lineage: PackageLineage,
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    package_home: PathBuf,
    namespace: Namespace,
) -> Res<(PackageLineage, CommitState)> {
    let current = lineage.commit.clone().ok_or_else(|| {
        Error::PackageOp(PackageOpError::Reset(
            "package has no local commit to reset".to_string(),
        ))
    })?;
    let previous_hash = current.prev_hashes.first().cloned().ok_or_else(|| {
        Error::PackageOp(PackageOpError::Reset(
            "no previous local revision exists".to_string(),
        ))
    })?;

    let previous_manifest_path = paths.installed_manifest(&namespace, &previous_hash);
    let previous_manifest = Manifest::from_path(storage, &previous_manifest_path).await?;

    info!(
        "⏳ Resetting {} from {} to local revision {}",
        namespace, current.hash, previous_hash
    );

    for path in lineage.paths.keys() {
        let working_path = package_home.join(path);
        if let Err(err) = storage.remove_file(&working_path).await
            && err.kind() != ErrorKind::NotFound
        {
            return Err(Error::Io(err));
        }
    }

    let mut restored_paths = BTreeMap::new();
    for row in &previous_manifest.rows {
        let object_path = paths.object(row.hash.digest());
        let working_path = package_home.join(&row.logical_key);
        if let Some(parent) = working_path.parent() {
            storage.create_dir_all(parent).await?;
        }
        debug!(
            "⏳ Restoring {} from {}",
            row.logical_key.display(),
            object_path.display()
        );
        storage.copy(&object_path, &working_path).await?;
        restored_paths.insert(
            row.logical_key.clone(),
            PathState {
                timestamp: storage.modified_timestamp(&working_path).await?,
                hash: row.hash.clone().into(),
            },
        );
    }

    let commit = CommitState {
        timestamp: storage.modified_timestamp(&previous_manifest_path).await?,
        hash: previous_hash,
        prev_hashes: current.prev_hashes.into_iter().skip(1).collect(),
    };
    lineage.paths = restored_paths;
    lineage.commit = Some(commit.clone());

    info!("✔️ Reset {} to local revision {}", namespace, commit.hash);
    Ok((lineage, commit))
}
