use std::collections::HashMap;
use std::path::Path;
use std::path::PathBuf;

use tempfile;

use crate::Error;

use super::Storage;

/// A mock implementation of the `Storage` trait.
#[derive(Default)]
pub(crate) struct MockStorage {
    /// A map of paths that are currently stored and their corresponding content
    pub(crate) registry: HashMap<PathBuf, Vec<u8>>,
}

impl MockStorage {
    /// Create the mock storage with these file names "installed"
    pub(crate) fn with_keys(keys: &Vec<&str>) -> MockStorage {
        let mut registry = HashMap::new();
        for key in keys {
            registry.insert(PathBuf::from(key), Vec::new());
        }
        MockStorage { registry }
    }

    /// Create the mock storage with these paths "installed"
    pub(crate) fn with_paths(paths: Vec<PathBuf>) -> MockStorage {
        let mut registry = HashMap::new();
        for path in paths {
            registry.insert(path, Vec::new());
        }
        MockStorage { registry }
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

    async fn remove_dir_all(&self, _path: impl AsRef<Path>) -> Result<(), Error> {
        Ok(()) // No-op
    }

    /// Overwrite the `remove_file` method to do nothing.
    async fn remove_file(&mut self, path: PathBuf) -> Result<(), std::io::Error> {
        self.registry.remove(&path);
        Ok(())
    }

    /// Overwrite the `exists` method to check if the path is in the set of paths.
    async fn exists(&self, path: impl AsRef<std::path::Path>) -> bool {
        self.registry.contains_key(path.as_ref())
    }

    /// Return the current time as the modified timestamp.
    async fn modified_timestamp(
        &self,
        _path: impl AsRef<Path>,
    ) -> Result<chrono::DateTime<chrono::Utc>, Error> {
        Ok(chrono::Utc::now())
    }

    /// Overwrite the `write` method to do nothing.
    async fn write_file(&mut self, path: PathBuf, bytes: &[u8]) -> Result<(), Error> {
        self.registry.insert(path, bytes.to_vec());
        Ok(())
    }

    async fn open_file(&mut self, path: impl AsRef<Path>) -> Result<tokio::fs::File, Error> {
        let temp_dir = tempfile::tempdir()?.into_path();
        let temp_file_path = temp_dir.join(&path);
        let stored_file = self.registry.get(path.as_ref()).unwrap();
        std::fs::create_dir_all(temp_file_path.parent().unwrap())?;
        std::fs::write(&temp_file_path, stored_file)?;
        Ok(tokio::fs::File::open(temp_file_path).await?)
    }

    async fn create_file(&mut self, path: impl AsRef<Path>) -> Result<tokio::fs::File, Error> {
        let temp_dir = tempfile::tempdir()?.into_path();
        self.registry
            .entry(path.as_ref().to_path_buf())
            .or_default();
        // TODO: there is a is_absolute() and is_root() methods
        //       but how to create relative path from absolute?
        let temp_file_path = if path.as_ref().starts_with("/") {
            temp_dir.join(path.as_ref().strip_prefix("/").unwrap())
        } else {
            temp_dir.join(&path)
        };
        std::fs::create_dir_all(temp_file_path.parent().unwrap())?;
        Ok(tokio::fs::File::create(temp_file_path).await?)
    }

    async fn read_to_string(&mut self, path: impl AsRef<Path>) -> Result<String, Error> {
        match self.registry.get(path.as_ref()) {
            Some(vec) => Ok(std::str::from_utf8(vec)?.to_string()),
            None => Err(Error::Io(std::io::Error::new(
                std::io::ErrorKind::NotFound,
                format!("Failed reading to string path {:?}", path.as_ref()),
            ))),
        }
    }
}
