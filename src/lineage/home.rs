use std::path::Path;
use std::path::PathBuf;

#[cfg(test)]
use tempfile::TempDir;

use serde::Deserialize;
use serde::Serialize;

use crate::Error;
use crate::Res;

/// Wrapper for working directory path with proper serialization/deserialization
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Home {
    inner: Option<PathBuf>,
}

impl Home {
    pub fn new(path: PathBuf) -> Self {
        Home { inner: Some(path) }
    }

    pub fn get(&self) -> Res<&PathBuf> {
        self.inner.as_ref().ok_or(Error::LineageHome)
    }

    pub fn is_some(&self) -> bool {
        self.inner.is_some()
    }

    pub fn is_none(&self) -> bool {
        self.inner.is_none()
    }

    pub fn join(&self, path: impl AsRef<std::path::Path>) -> Result<PathBuf, Error> {
        match &self.inner {
            Some(dir) => Ok(dir.join(path)),
            None => Err(Error::LineageHome),
        }
    }

    #[cfg(test)]
    pub fn from_temp_dir() -> Res<(Self, TempDir)> {
        let temp_dir = TempDir::new()?;
        Ok((Home::from(temp_dir.path()), temp_dir))
    }
}

impl<P: AsRef<Path>> From<P> for Home {
    fn from(path: P) -> Self {
        Home {
            inner: Some(path.as_ref().to_path_buf()),
        }
    }
}

impl Serialize for Home {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        match &self.inner {
            Some(path) => serializer.serialize_str(&path.to_string_lossy()),
            None => serializer.serialize_none(),
        }
    }
}

impl<'de> Deserialize<'de> for Home {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let path_str = String::deserialize(deserializer)?;
        Ok(Home {
            inner: Some(PathBuf::from(path_str)),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_home_from_path() {
        let path = PathBuf::from("/tmp/home");
        let home = Home::from(&path);
        assert_eq!(home.inner, Some(path));

        let path_str = "/tmp/home";
        let home = Home::from(path_str);
        assert_eq!(home.inner, Some(PathBuf::from(path_str)));
    }
}
