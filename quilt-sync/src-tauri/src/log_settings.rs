use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Serialize;

use quilt_rs::logging::LOG_ENV;

use crate::error::Error;

const FILE_NAME: &str = "log_settings.json";

/// The log level chosen on the Settings screen.
///
/// `Default` keeps the built-in filters. Any other level goes through
/// `quilt_rs::logging::directives`, the rule [`LOG_ENV`] follows, and replaces
/// the built-in filters, as [`LOG_ENV`] does. No `Off`: a support request
/// starts with the log.
#[derive(Serialize, Deserialize, Clone, Copy, Debug, Default, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    #[default]
    Default,
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl LogLevel {
    /// The directives this choice stands for, or `None` for the built-in filters.
    pub fn directives(self) -> Option<String> {
        let level = match self {
            Self::Default => return None,
            Self::Trace => "trace",
            Self::Debug => "debug",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        };
        quilt_rs::logging::directives(level).ok().flatten()
    }
}

/// Persisted as `log_settings.json` in `app_local_data_dir`. Read once, when
/// logging starts, so a change applies after a restart.
#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq, Eq)]
pub struct LogSettings {
    #[serde(default)]
    pub level: LogLevel,
}

impl LogSettings {
    fn file_path(data_dir: &Path) -> PathBuf {
        data_dir.join(FILE_NAME)
    }

    /// Load settings from disk. Missing file → defaults.
    ///
    /// Blocking, unlike the other settings files: it is read before the logger
    /// exists, ahead of anything async.
    pub fn load(data_dir: &Path) -> Result<Self, Error> {
        match std::fs::read(Self::file_path(data_dir)) {
            Ok(bytes) => Ok(serde_json::from_slice(&bytes)?),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(err) => Err(Error::from(err)),
        }
    }

    /// Write atomically: temp file + rename.
    pub async fn save(&self, data_dir: &Path) -> Result<(), Error> {
        tokio::fs::create_dir_all(data_dir).await?;
        let path = Self::file_path(data_dir);
        let tmp = path.with_extension("json.tmp");
        let bytes = serde_json::to_vec_pretty(self)?;
        tokio::fs::write(&tmp, &bytes).await?;
        tokio::fs::rename(&tmp, &path).await?;
        Ok(())
    }
}

/// What [`LOG_ENV`] does to the saved choice.
#[derive(Serialize, Clone, Debug, PartialEq, Eq)]
#[serde(tag = "kind", content = "value", rename_all = "camelCase")]
pub enum LogEnv {
    /// Unset or empty: the saved choice applies.
    Unset,
    /// A valid value, which replaces the saved choice.
    Overrides(String),
    /// Not a level or a list of directives, so ignored: the saved choice applies.
    Ignored(String),
}

impl LogEnv {
    pub fn new(value: Option<&str>) -> Self {
        let Some(value) = value else {
            return Self::Unset;
        };
        match quilt_rs::logging::directives(value) {
            Ok(None) => Self::Unset,
            Ok(Some(_)) => Self::Overrides(value.to_string()),
            Err(_) => Self::Ignored(value.to_string()),
        }
    }

    pub fn from_env() -> Self {
        Self::new(std::env::var(LOG_ENV).ok().as_deref())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    #[tokio::test]
    async fn roundtrip() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        let settings = LogSettings {
            level: LogLevel::Trace,
        };
        settings.save(dir.path()).await?;
        assert_eq!(LogSettings::load(dir.path())?, settings);
        let written = tokio::fs::read_to_string(dir.path().join(FILE_NAME)).await?;
        assert!(written.contains(r#""trace""#), "{written}");
        Ok(())
    }

    #[test]
    fn missing_file_is_default() -> Result<(), Error> {
        let dir = TempDir::new().unwrap();
        assert_eq!(LogSettings::load(dir.path())?.level, LogLevel::Default);
        Ok(())
    }

    #[test]
    fn default_keeps_the_built_in_filters() {
        assert_eq!(LogLevel::Default.directives(), None);
    }

    #[test]
    fn a_level_follows_the_shared_rule() {
        assert_eq!(
            LogLevel::Debug.directives().as_deref(),
            Some("quilt=debug,quilt_rs=debug,quilt_uri=debug,quilt_sync=debug,warn")
        );
        assert_eq!(LogLevel::Error.directives().as_deref(), Some("error"));
    }

    #[test]
    fn the_variable_overrides_only_when_valid() {
        assert_eq!(LogEnv::new(None), LogEnv::Unset);
        assert_eq!(LogEnv::new(Some(" ")), LogEnv::Unset);
        assert_eq!(
            LogEnv::new(Some("quilt_rs=trace")),
            LogEnv::Overrides("quilt_rs=trace".to_string())
        );
        assert_eq!(
            LogEnv::new(Some("debgu")),
            LogEnv::Ignored("debgu".to_string())
        );
    }
}
