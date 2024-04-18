use std::collections::BTreeMap;

use super::{CommitState, LineagePaths, PackageLineage};
use crate::quilt::lineage::PathState;
use crate::quilt::manifest_handle::RemoteManifest;
use crate::quilt::uri::S3PackageUri;
use crate::quilt::Error;

pub fn path_state() -> PathState {
    PathState {
        timestamp: chrono::DateTime::default(),
        hash: multihash::Multihash::wrap(0xb510, b"pedestrian").unwrap(),
    }
}

fn commit_state_with_hash(hash: &str) -> Option<CommitState> {
    Some(CommitState {
        hash: hash.to_string(),
        ..CommitState::default()
    })
}

fn lineage_paths(keys: &Vec<&str>) -> LineagePaths {
    let mut paths = BTreeMap::new();
    for key in keys {
        paths.insert(key.to_string(), path_state());
    }
    paths
}

pub fn with_paths(keys: &Vec<&str>) -> PackageLineage {
    PackageLineage {
        paths: lineage_paths(keys),
        ..PackageLineage::default()
    }
}

pub fn with_commit() -> PackageLineage {
    PackageLineage {
        commit: Some(CommitState::default()),
        ..PackageLineage::default()
    }
}

pub fn with_remote(uri_str: &str) -> Result<PackageLineage, Error> {
    let uri = S3PackageUri::try_from(uri_str)?;
    let remote_manifest: RemoteManifest = uri.into();
    Ok(PackageLineage {
        remote: remote_manifest,
        ..PackageLineage::default()
    })
}

pub fn with_commit_hash(hash: &str) -> PackageLineage {
    PackageLineage {
        commit: commit_state_with_hash(hash),
        ..PackageLineage::default()
    }
}

pub fn with_commit_hashes(base_hash: &str, latest_hash: &str, commit_hash: &str) -> PackageLineage {
    PackageLineage {
        commit: commit_state_with_hash(commit_hash),
        base_hash: base_hash.to_string(),
        latest_hash: latest_hash.to_string(),
        ..PackageLineage::default()
    }
}
