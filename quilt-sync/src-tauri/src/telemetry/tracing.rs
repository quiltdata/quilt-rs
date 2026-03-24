use std::path::{Path, PathBuf};

use tempfile::TempDir;
use tracing_appender::rolling;
use tracing_subscriber::prelude::*;

use crate::telemetry::prelude::*;
use crate::Result;

pub enum LogsDir {
    Permanent(PathBuf),
    Temporary(TempDir),
}

impl LogsDir {
    pub fn path(&self) -> &Path {
        match self {
            LogsDir::Permanent(path) => path,
            LogsDir::Temporary(temp_dir) => temp_dir.path(),
        }
    }

    #[cfg(test)]
    pub fn create() -> Result<Self> {
        Ok(LogsDir::Temporary(tempfile::TempDir::new()?))
    }
}

fn get_logs_dir(base_path: &Path) -> Result<LogsDir> {
    let logs_dir = base_path.join("logs");

    if let Err(err) = std::fs::create_dir_all(&logs_dir) {
        if err.kind() != std::io::ErrorKind::AlreadyExists {
            return Ok(LogsDir::Temporary(tempfile::tempdir()?));
        }
    }

    Ok(LogsDir::Permanent(logs_dir))
}

pub fn init_file_logging(base_path: &Path) -> Result<LogsDir> {
    let logs_dir = get_logs_dir(base_path)?;
    init_tracing(&logs_dir);
    Ok(logs_dir)
}

fn init_tracing(logs_dir: &LogsDir) {
    let path = logs_dir.path();
    if let Ok(file_appender) = rolling::RollingFileAppender::builder()
        .rotation(rolling::Rotation::DAILY)
        .filename_prefix("quilt-sync")
        .filename_suffix("log")
        .max_log_files(10)
        .build(path)
    {
        tracing_subscriber::registry()
            .with(tracing_subscriber::EnvFilter::from_default_env())
            .with(tracing_subscriber::fmt::layer().with_writer(file_appender))
            .with(sentry::integrations::tracing::layer())
            .init();
    }

    if let LogsDir::Temporary(_) = logs_dir {
        error!(
            "Failed to create permanent logs directory, using temporary directory: {}",
            logs_dir.path().display()
        );
    }
}
