use std::collections::HashSet;
use std::path::{Path, PathBuf};

use crate::Error;

use super::Storage;

/// A mock implementation of the `Storage` trait.
#[derive(Default)]
pub(crate) struct MockStorage {
    /// A set of paths that are currently stored.
    pub(crate) paths: HashSet<PathBuf>,
}

impl MockStorage {
    /// Install a list of paths into the mock storage.
    pub(crate) fn install_paths(&mut self, new_paths: HashSet<PathBuf>) {
        self.paths.extend(new_paths);
    }
}

impl Storage for MockStorage {
    async fn copy(
        &self,
        _from: impl AsRef<Path>,
        _to: impl AsRef<Path>,
    ) -> Result<u64, std::io::Error> {
        unimplemented!("Right now, the mock storage does not support copying files.")
    }

    async fn create_dir_all(&self, _path: impl AsRef<Path>) -> Result<(), std::io::Error> {
        Ok(()) // No-op
    }

    /// Overwrite the `remove_file` method to do nothing.
    async fn remove_file(&mut self, _path: PathBuf) -> Result<(), std::io::Error> {
        self.paths.remove(&_path);
        Ok(())
    }

    /// Overwrite the `exists` method to check if the path is in the set of paths.
    async fn exists(&self, path: impl AsRef<std::path::Path>) -> bool {
        self.paths.contains(path.as_ref())
    }

    /// Return the current time as the modified timestamp.
    async fn modified_timestamp(
        &self,
        _path: impl AsRef<Path>,
    ) -> Result<chrono::DateTime<chrono::Utc>, Error> {
        Ok(chrono::Utc::now())
    }
}
