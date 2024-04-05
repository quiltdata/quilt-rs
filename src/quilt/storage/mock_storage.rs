use std::collections::HashMap;
use std::collections::HashSet;
use std::path::Path;
use std::path::PathBuf;

use crate::Error;

use super::Storage;

/// A mock implementation of the `Storage` trait.
#[derive(Default)]
pub(crate) struct MockStorage {
    /// A set of paths that are currently stored.
    pub(crate) paths: HashSet<PathBuf>,

    /// A map of paths that are currently stored and their corresponding content
    pub(crate) registry: HashMap<PathBuf, Vec<u8>>,
}

impl MockStorage {
    /// Install a list of paths into the mock storage.
    pub(crate) fn install_paths(&mut self, new_paths: HashSet<PathBuf>) {
        // TODO: install to the registry
        self.paths.extend(new_paths);
    }
}

impl Storage for MockStorage {
    async fn copy(
        &mut self,
        from: impl AsRef<Path>,
        to: impl AsRef<Path>,
    ) -> Result<u64, std::io::Error> {
        let file = self.registry.get(from.as_ref()).unwrap();
        self.registry
            .insert(to.as_ref().to_path_buf(), file.clone());
        Ok(0)
    }

    async fn create_dir_all(&self, _path: impl AsRef<Path>) -> Result<(), std::io::Error> {
        Ok(()) // No-op
    }

    async fn remove_dir_all(&self, _path: impl AsRef<Path>) -> Result<(), std::io::Error> {
        Ok(()) // No-op
    }

    /// Overwrite the `remove_file` method to do nothing.
    async fn remove_file(&mut self, path: PathBuf) -> Result<(), std::io::Error> {
        // TODO: remove from the registry
        self.paths.remove(&path);
        Ok(())
    }

    /// Overwrite the `exists` method to check if the path is in the set of paths.
    async fn exists(&self, path: impl AsRef<std::path::Path>) -> bool {
        // TODO: contains_key in the registry
        self.paths.contains(path.as_ref()) || self.registry.contains_key(path.as_ref())
    }

    /// Return the current time as the modified timestamp.
    async fn modified_timestamp(
        &self,
        _path: impl AsRef<Path>,
    ) -> Result<chrono::DateTime<chrono::Utc>, Error> {
        Ok(chrono::Utc::now())
    }

    /// Overwrite the `write` method to do nothing.
    async fn write(&mut self, path: PathBuf, bytes: &[u8]) -> Result<(), Error> {
        self.registry.insert(path, bytes.to_vec());
        Ok(())
    }
}
