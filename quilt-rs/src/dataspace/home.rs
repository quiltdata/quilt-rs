//! `set_home`: the default dataspace, which replaces today's home (25.2),
//! chosen. The choice is an application preference (25.12), so it is kept
//! in a file of the application's own folder, [`DEFAULT_DATASPACE_FILE`],
//! never in the Domain or with the credentials. An application that keeps
//! its preferences elsewhere can keep this one there too.
//!
//! The folder chosen is made a dataspace where it is not one, with an empty
//! `data.toml`, and nothing in it moves: unlike today's home, the old
//! dataspace keeps its own contract, lock and folders.

use std::path::Path;
use std::path::PathBuf;

use aws_sdk_s3::primitives::ByteStream;
use thiserror::Error;
use tracing::info;

use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::normalize;
use crate::dataspace::edit::ContractEdit;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;

/// The file, in an application's folder, that names its default dataspace.
pub const DEFAULT_DATASPACE_FILE: &str = "default_dataspace.json";

/// What [`DEFAULT_DATASPACE_FILE`] holds.
#[derive(Debug, serde::Deserialize, serde::Serialize)]
struct Stored {
    dataspace: PathBuf,
}

/// The default dataspace, as [`set_home`] chose it.
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct Home {
    /// The dataspace's folder, as an absolute path.
    pub dataspace: PathBuf,
    /// Whether the folder was given an empty `data.toml`, having none.
    pub made: bool,
}

/// Why no default dataspace was chosen.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum HomeError {
    #[error(transparent)]
    Contract(Box<ContractError>),

    #[error(transparent)]
    Failed(Box<crate::Error>),
}

impl From<ContractError> for HomeError {
    fn from(err: ContractError) -> Self {
        Self::Contract(Box::new(err))
    }
}

impl From<crate::Error> for HomeError {
    fn from(err: crate::Error) -> Self {
        Self::Failed(Box::new(err))
    }
}

/// The default dataspace the application whose folder is `app_dir` chose,
/// or `None` where it chose none.
pub async fn default_dataspace(
    storage: &(impl Storage + Sync),
    app_dir: &Path,
) -> crate::Res<Option<PathBuf>> {
    match storage
        .read_bytes(app_dir.join(DEFAULT_DATASPACE_FILE))
        .await
    {
        Ok(bytes) => Ok(Some(serde_json::from_slice::<Stored>(&bytes)?.dataspace)),
        Err(err) if err.is_not_found() => Ok(None),
        Err(err) => Err(err),
    }
}

/// Makes `dataspace` the default dataspace of the application whose folder
/// is `app_dir`. A relative `dataspace` is taken from the current folder.
/// The folder is made, with an empty `data.toml`, where it holds none; one
/// it holds must be TOML. The choice is written to
/// [`DEFAULT_DATASPACE_FILE`] last, by renaming a temporary file over it.
pub async fn set_home(
    storage: &(impl Storage + Sync),
    app_dir: &Path,
    dataspace: &Path,
) -> Result<Home, HomeError> {
    let absolute =
        std::path::absolute(dataspace).map_err(|err| HomeError::Failed(Box::new(err.into())))?;
    // A `data.toml` that is no TOML is refused here; what it declares is
    // each command's to read.
    let edit = ContractEdit::open(storage, &normalize(&absolute)).await?;
    let made = !edit.found();
    if made {
        edit.write(storage).await?;
    }
    let home = Home {
        dataspace: edit.dataspace().to_path_buf(),
        made,
    };
    let stored = serde_json::to_vec(&Stored {
        dataspace: home.dataspace.clone(),
    })
    .map_err(crate::Error::from)?;
    storage.create_dir_all(app_dir).await?;
    storage
        .write_byte_stream_via(
            &app_dir.join(DEFAULT_DATASPACE_FILE),
            format!("{DEFAULT_DATASPACE_FILE}.tmp-{}", uuid::Uuid::new_v4()),
            ByteStream::from(stored),
            None,
        )
        .await?;
    info!("✔️ The default dataspace is {}", home.dataspace.display());
    Ok(home)
}

#[cfg(test)]
mod tests;
