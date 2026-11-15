//! The shared, non-secret `settings.json` that every Quilt client reads: today
//! it holds one key, `default_catalog`.
//!
//! The file lives in the shared folder described by quilt-specs#46: `com.quiltdata.auth`
//! under [`dirs::data_local_dir`], or the folder named by `QUILT_AUTH_DIR`.
//! It holds no secret, so it is a plain file, and the folder is 0700 on Unix.
//! Every change takes `settings.json.lock` first and is written through a
//! temp file and a rename.
//!
//! The file is shared with keys this crate does not know (the specs put the
//! per-bucket ambient profiles there next), so a change rewrites only its own
//! key and keeps the rest. A file that cannot be read as a JSON object is an
//! error and is never replaced.
//!
//! A missing file, or a missing key, means "no default catalog". Nothing in this
//! crate consults the default on its own: a caller that wants it as a fallback
//! asks [`effective_catalog`], which keeps a package's own catalog first.

use std::io::ErrorKind;
use std::path::Path;
use std::path::PathBuf;
use std::str::FromStr;

use serde_json::Map;
use serde_json::Value;

use crate::Res;
use crate::error::FsError;
use crate::io::storage::LockGuard;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;
use quilt_uri::Host;

/// Names the shared folder, replacing `<data_local_dir>/com.quiltdata.auth`.
pub const SHARED_DIR_ENV: &str = "QUILT_AUTH_DIR";
/// The shared folder's name under the local data directory.
pub const SHARED_DIR_NAME: &str = "com.quiltdata.auth";
/// The settings file, in the shared folder.
pub const SETTINGS_FILE: &str = "settings.json";
/// The lock every change to [`SETTINGS_FILE`] holds.
pub const SETTINGS_LOCK: &str = "settings.json.lock";
/// The key that holds the default catalog, as a bare DNS host.
pub const DEFAULT_CATALOG_KEY: &str = "default_catalog";

/// The shared folder: `override_dir` if it is set and not empty, else
/// `com.quiltdata.auth` under `data_local_dir`. `None` when neither exists.
///
/// The pure half of [`shared_dir`], so a caller or a test can name both inputs.
#[must_use]
pub fn shared_dir_from(
    override_dir: Option<PathBuf>,
    data_local_dir: Option<PathBuf>,
) -> Option<PathBuf> {
    override_dir
        .filter(|dir| !dir.as_os_str().is_empty())
        .or_else(|| data_local_dir.map(|dir| dir.join(SHARED_DIR_NAME)))
}

/// The shared folder on this computer, from `QUILT_AUTH_DIR` and the local data directory.
#[must_use]
pub fn shared_dir() -> Option<PathBuf> {
    shared_dir_from(
        std::env::var_os(SHARED_DIR_ENV).map(PathBuf::from),
        dirs::data_local_dir(),
    )
}

/// What a caller knows about the package an operation is for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PackageOrigin {
    /// There is no package: the operation is not about one.
    None,
    /// The package's origin names this catalog.
    Catalog(Host),
    /// The package has a remote but no catalog, so it is read and written on
    /// this computer's AWS credentials. That is a choice, not a gap to fill.
    Ambient,
}

/// The catalog an operation uses, first match wins:
///
/// 1. the one the caller named (`explicit`);
/// 2. the one in the package's origin ([`PackageOrigin::Catalog`]);
/// 3. a package with no catalog ([`PackageOrigin::Ambient`]) uses none;
/// 4. only with no package at all, the stored default.
#[must_use]
pub fn effective_catalog(
    explicit: Option<Host>,
    package: PackageOrigin,
    default: Option<Host>,
) -> Option<Host> {
    match (explicit, package) {
        (Some(host), _) | (None, PackageOrigin::Catalog(host)) => Some(host),
        (None, PackageOrigin::Ambient) => None,
        (None, PackageOrigin::None) => default,
    }
}

/// The shared `settings.json`, in one folder.
#[derive(Clone, Debug)]
pub struct SharedSettings<S: Storage> {
    storage: S,
    dir: PathBuf,
}

impl<S: Storage + Sync> SharedSettings<S> {
    /// The settings in `dir`. Nothing is read or created until a call needs it.
    pub fn new(storage: S, dir: impl Into<PathBuf>) -> Self {
        SharedSettings {
            storage,
            dir: dir.into(),
        }
    }

    fn file(&self) -> PathBuf {
        self.dir.join(SETTINGS_FILE)
    }

    fn lock_file(&self) -> PathBuf {
        self.dir.join(SETTINGS_LOCK)
    }

    /// The stored default catalog; `None` with no file or no key.
    ///
    /// Takes no lock: a writer replaces the file by rename, so a reader sees the
    /// old file or the new one.
    pub async fn default_catalog(&self) -> Res<Option<Host>> {
        let map = self.read().await?;
        host_in(&map, &self.file())
    }

    /// Stores `host` as the default catalog, keeping every other key.
    pub async fn set_default_catalog(&self, host: &Host) -> Res {
        let _lock = self.lock().await?;
        let mut map = self.read().await?;
        map.insert(
            DEFAULT_CATALOG_KEY.to_string(),
            Value::String(host.to_string()),
        );
        self.write(&map).await
    }

    /// Removes the default catalog, keeping every other key. Returns whether
    /// there was one to remove. With no file, creates nothing.
    pub async fn clear_default_catalog(&self) -> Res<bool> {
        if !self.storage.exists(self.file()).await {
            return Ok(false);
        }
        let _lock = self.lock().await?;
        let mut map = self.read().await?;
        if map.remove(DEFAULT_CATALOG_KEY).is_none() {
            return Ok(false);
        }
        self.write(&map).await?;
        Ok(true)
    }

    async fn lock(&self) -> Res<LockGuard> {
        self.storage.create_private_dir(&self.dir).await?;
        self.storage.lock_exclusive(self.lock_file()).await
    }

    async fn read(&self) -> Res<Map<String, Value>> {
        let path = self.file();
        if !self.storage.exists(&path).await {
            return Ok(Map::new());
        }
        let bytes = self.storage.read_bytes(&path).await?;
        match serde_json::from_slice(&bytes) {
            Ok(Value::Object(map)) => Ok(map),
            Ok(_) => Err(unreadable(&path, "not a JSON object")),
            Err(err) => Err(unreadable(&path, &err.to_string())),
        }
    }

    async fn write(&self, map: &Map<String, Value>) -> Res {
        let path = self.file();
        let temp = self.dir.join(format!("{SETTINGS_FILE}.tmp"));
        let bytes = serde_json::to_vec_pretty(map)?;
        // The lock is held, so no other writer uses the temp file.
        self.storage.write_byte_stream(&temp, bytes.into()).await?;
        self.storage.rename(&temp, &path).await
    }
}

fn host_in(map: &Map<String, Value>, path: &Path) -> Res<Option<Host>> {
    match map.get(DEFAULT_CATALOG_KEY) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(name)) => Host::from_str(name).map(Some).map_err(|err| {
            unreadable(path, &format!("`{DEFAULT_CATALOG_KEY}` is {name:?}: {err}"))
        }),
        Some(_) => Err(unreadable(
            path,
            &format!("`{DEFAULT_CATALOG_KEY}` is not a string"),
        )),
    }
}

fn unreadable(path: &Path, why: &str) -> crate::Error {
    FsError::Read {
        path: path.to_path_buf(),
        source: std::io::Error::new(ErrorKind::InvalidData, why),
    }
    .into()
}

#[cfg(test)]
mod tests;
