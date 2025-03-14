use std::path::Path;
use std::path::PathBuf;

#[cfg(test)]
use crate::Res;

#[cfg(test)]
use tempfile::TempDir;

use serde::Deserialize;
use serde::Serialize;

/// Wrapper for working directory path with proper serialization/deserialization
#[derive(Clone, Debug, PartialEq, Eq, Default)]
pub struct Home {
    inner: PathBuf,
}

impl Home {
    pub fn new(path: PathBuf) -> Self {
        Home { inner: path }
    }

    pub fn join(&self, path: impl AsRef<Path>) -> PathBuf {
        self.inner.join(path)
    }

    #[cfg(test)]
    pub fn from_temp_dir() -> Res<(Self, TempDir)> {
        let temp_dir = TempDir::new()?;
        Ok((Home::from(temp_dir.path()), temp_dir))
    }
}

impl AsRef<PathBuf> for Home {
    fn as_ref(&self) -> &PathBuf {
        &self.inner
    }
}

impl<P: AsRef<Path>> From<P> for Home {
    fn from(path: P) -> Self {
        Home {
            inner: path.as_ref().to_path_buf(),
        }
    }
}

impl Serialize for Home {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.inner.to_string_lossy())
    }
}

impl<'de> Deserialize<'de> for Home {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let path_str = String::deserialize(deserializer)?;
        Ok(Home {
            inner: PathBuf::from(path_str),
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
        assert_eq!(home.inner, path);

        let path_str = "/tmp/home";
        let home = Home::from(path_str);
        assert_eq!(home.inner, PathBuf::from(path_str));
    }
}
