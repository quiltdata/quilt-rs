//! The shared, non-secret `settings.json` that every Quilt client reads: today
//! it holds one key, `default_catalog`.
//!
//! The file lives in the shared folder of quilt-specs#46: `com.quiltdata.auth`
//! under [`dirs::data_local_dir`], or `QUILT_AUTH_DIR`. Every change takes
//! `settings.json.lock` first and keeps the keys it does not know. A file that
//! is not a JSON object is an error and is never replaced.
//!
//! A missing file or key means no default catalog, until
//! [`SharedSettings::default_catalog_or_adopt`] finds quilt3's. A cleared
//! default is stored as `null`, which stops that. [`effective_catalog`] keeps
//! a package's catalog ahead of the default.

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

const QUILT3_APP_NAME: &str = "Quilt";
const QUILT3_APP_AUTHOR: &str = "QuiltData";
const QUILT3_CONFIG_FILE: &str = "config.yml";

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

/// Where quilt3 keeps its `config.yml`, best match first: its
/// `platformdirs.user_data_dir("Quilt", "QuiltData")`.
///
/// That is `%LOCALAPPDATA%\QuiltData\Quilt` on Windows and `Quilt` under the
/// local data directory elsewhere. Newer platformdirs also honors an absolute
/// `XDG_DATA_HOME` on macOS, so it is tried first.
#[must_use]
pub fn quilt3_config_files() -> Vec<PathBuf> {
    quilt3_config_files_from(
        cfg!(windows),
        std::env::var_os("XDG_DATA_HOME").map(PathBuf::from),
        dirs::data_local_dir(),
    )
}

/// The pure half of [`quilt3_config_files`].
#[must_use]
pub fn quilt3_config_files_from(
    windows: bool,
    xdg_data_home: Option<PathBuf>,
    data_local_dir: Option<PathBuf>,
) -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if windows {
        dirs.extend(data_local_dir.map(|dir| dir.join(QUILT3_APP_AUTHOR).join(QUILT3_APP_NAME)));
    } else {
        dirs.extend(
            xdg_data_home
                .filter(|dir| dir.is_absolute())
                .map(|dir| dir.join(QUILT3_APP_NAME)),
        );
        dirs.extend(data_local_dir.map(|dir| dir.join(QUILT3_APP_NAME)));
    }
    let mut files: Vec<PathBuf> = dirs
        .into_iter()
        .map(|dir| dir.join(QUILT3_CONFIG_FILE))
        .collect();
    files.dedup();
    files
}

#[derive(serde::Deserialize)]
struct Quilt3Config {
    navigator_url: Option<String>,
}

/// The catalog host in quilt3's `config.yml`: the host of its `navigator_url`.
/// `None` for a file that is not YAML, has no `navigator_url`, or has one that
/// is empty or holds no host.
#[must_use]
pub fn catalog_in_quilt3_config(config: &[u8]) -> Option<Host> {
    let config: Quilt3Config = serde_saphyr::from_str(std::str::from_utf8(config).ok()?).ok()?;
    let url = url::Url::parse(config.navigator_url?.trim()).ok()?;
    Host::from_str(url.host_str()?).ok()
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

    /// The stored default catalog, or, when the file has no `default_catalog`
    /// key at all, the one in quilt3's `config.yml` (the first of
    /// `quilt3_config_files` that exists), which is then stored under the lock
    /// so this file is the one source. `config.yml` is only read. A missing,
    /// unreadable or hostless `config.yml` is no default, not an error.
    pub async fn default_catalog_or_adopt(
        &self,
        quilt3_config_files: &[PathBuf],
    ) -> Res<Option<Host>> {
        let map = self.read().await?;
        if map.contains_key(DEFAULT_CATALOG_KEY) {
            return host_in(&map, &self.file());
        }
        let Some(host) = self.quilt3_catalog(quilt3_config_files).await else {
            return Ok(None);
        };
        let _lock = self.lock().await?;
        let mut map = self.read().await?;
        if map.contains_key(DEFAULT_CATALOG_KEY) {
            return host_in(&map, &self.file());
        }
        map.insert(
            DEFAULT_CATALOG_KEY.to_string(),
            Value::String(host.to_string()),
        );
        self.write(&map).await?;
        Ok(Some(host))
    }

    async fn quilt3_catalog(&self, files: &[PathBuf]) -> Option<Host> {
        for file in files {
            if self.storage.exists(file).await {
                let bytes = self.storage.read_bytes(file).await.ok()?;
                return catalog_in_quilt3_config(&bytes);
            }
        }
        None
    }

    /// Clears the default catalog, keeping every other key, by storing `null`
    /// so quilt3's is not adopted again. Returns whether a host was stored.
    pub async fn clear_default_catalog(&self) -> Res<bool> {
        let _lock = self.lock().await?;
        let mut map = self.read().await?;
        let had_host = matches!(map.get(DEFAULT_CATALOG_KEY), Some(value) if !value.is_null());
        if !matches!(map.get(DEFAULT_CATALOG_KEY), Some(Value::Null)) {
            map.insert(DEFAULT_CATALOG_KEY.to_string(), Value::Null);
            self.write(&map).await?;
        }
        Ok(had_host)
    }

    async fn lock(&self) -> Res<LockGuard> {
        self.storage.create_private_dir(&self.dir).await?;
        self.storage.lock_exclusive(self.lock_file()).await
    }

    async fn read(&self) -> Res<Map<String, Value>> {
        let path = self.file();
        let bytes = match self.storage.read_bytes(&path).await {
            Ok(bytes) => bytes,
            Err(err) if is_absent(&err) => return Ok(Map::new()),
            Err(err) => return Err(err),
        };
        match serde_json::from_slice(&bytes) {
            Ok(Value::Object(map)) => Ok(map),
            Ok(_) => Err(unreadable(&path, "not a JSON object")),
            Err(err) => Err(unreadable(&path, &err.to_string())),
        }
    }

    async fn write(&self, map: &Map<String, Value>) -> Res {
        let bytes = serde_json::to_vec_pretty(map)?;
        self.storage
            .write_byte_stream(self.file(), bytes.into())
            .await
    }
}

/// Whether a read failed because there is no file, and not for any other reason.
fn is_absent(err: &crate::Error) -> bool {
    let mut cause: Option<&(dyn std::error::Error + 'static)> = Some(err);
    err.is_not_found()
        || std::iter::from_fn(|| {
            let current = cause?;
            cause = current.source();
            Some(current)
        })
        .any(|e| matches!(e.downcast_ref::<std::io::Error>(), Some(io) if io.kind() == ErrorKind::NotFound))
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
