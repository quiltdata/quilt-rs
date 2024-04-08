use std::collections::BTreeMap;

use super::{CommitState, LineagePaths, PackageLineage};
use crate::quilt::lineage::PathState;

pub fn lineage_paths(keys: &Vec<&str>) -> LineagePaths {
    let mut paths = BTreeMap::new();
    for key in keys {
        paths.insert(key.to_string(), PathState::default());
    }
    paths
}

pub fn lineage_with_paths(keys: &Vec<&str>) -> PackageLineage {
    PackageLineage {
        paths: lineage_paths(keys),
        ..PackageLineage::default()
    }
}

pub fn lineage_with_commit_hash(hash: &str) -> PackageLineage {
    PackageLineage {
        commit: Some(CommitState {
            hash: hash.to_string(),
            ..CommitState::default()
        }),
        ..PackageLineage::default()
    }
}
