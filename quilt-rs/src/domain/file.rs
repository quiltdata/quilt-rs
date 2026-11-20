//! `domain.json` at the Domain's root: the default dataspace, the dataspaces
//! on autosync, and how far the import of a domain an older build kept has
//! got.
//!
//! The name is the one the refs store gives its own `.quilt/domain.json`,
//! which held a single home. The contents are told apart by `format`: this
//! file is format 2, and a build that reads only format 1 refuses it.

use std::path::Path;
use std::path::PathBuf;

use aws_sdk_s3::primitives::ByteStream;
use serde::Deserialize;
use serde::Serialize;

use super::DomainRoot;
use crate::Res;
use crate::dataspace::contract::dataspace_path;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;

/// The file's name at the Domain's root.
pub const DOMAIN_FILE: &str = "domain.json";

/// The format this build reads and writes. Format 1 is the refs store's.
const FORMAT: u32 = 2;

/// What `domain.json` holds.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct DomainFile {
    /// The dataspace a call uses when it names none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub default_dataspace: Option<PathBuf>,
    /// The dataspaces on autosync, in the order autosync was turned on, each
    /// named as its contract names it.
    #[serde(default)]
    pub autosync: Vec<PathBuf>,
    /// The import this Domain was filled by, where there was one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub import: Option<ImportRecord>,
}

/// The import of an older build's domain into this one.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ImportRecord {
    /// The folder imported from, which holds `.quilt/`.
    pub from: PathBuf,
    /// `false` while an import has not finished, as when it was interrupted.
    pub done: bool,
}

#[derive(Serialize, Deserialize)]
struct Raw {
    format: u32,
    #[serde(flatten)]
    file: DomainFile,
}

#[derive(Deserialize)]
struct Format {
    format: u32,
}

impl DomainRoot {
    /// `domain.json`, or `None` where there is none.
    pub async fn read_file(&self, storage: &(impl Storage + Sync)) -> Res<Option<DomainFile>> {
        let path = self.domain_file();
        let bytes = match storage.read_bytes(&path).await {
            Ok(bytes) => bytes,
            Err(err) if err.is_not_found() => return Ok(None),
            Err(err) => return Err(err),
        };
        // The format first, so the refs store's file is refused as one.
        let Format { format } = serde_json::from_slice(&bytes)?;
        if format != FORMAT {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidData,
                format!(
                    "{} is format {format}; this build reads format {FORMAT}",
                    path.display()
                ),
            )
            .into());
        }
        let raw: Raw = serde_json::from_slice(&bytes)?;
        Ok(Some(raw.file))
    }

    /// Writes `domain.json` whole, by renaming a temporary file over it.
    pub(crate) async fn write_file(
        &self,
        storage: &(impl Storage + Sync),
        file: &DomainFile,
    ) -> Res {
        let raw = Raw {
            format: FORMAT,
            file: file.clone(),
        };
        let temp = format!("{DOMAIN_FILE}.tmp-{}", uuid::Uuid::new_v4());
        storage
            .write_byte_stream_via(
                self.domain_file(),
                temp,
                ByteStream::from(serde_json::to_vec_pretty(&raw)?),
                None,
            )
            .await
    }

    /// The dataspaces on autosync, in the order autosync was turned on.
    pub async fn autosync(&self, storage: &(impl Storage + Sync)) -> Res<Vec<PathBuf>> {
        Ok(self
            .read_file(storage)
            .await?
            .map_or_default(|file| file.autosync))
    }

    /// Turns autosync on or off for the dataspace at `dataspace`, named as
    /// [`Contract::dataspace`](crate::dataspace::contract::Contract::dataspace)
    /// names it. Turning it on again keeps its place. Done holding
    /// `domain.lock`, so two writers never lose each other's change.
    pub async fn set_autosync(
        &self,
        storage: &(impl Storage + Sync),
        dataspace: &Path,
        on: bool,
    ) -> Res {
        let dataspace = dataspace_path(dataspace)?;
        let _held = storage.lock_exclusive(self.domain_lock()).await?;
        let mut file = self.read_file(storage).await?.unwrap_or_default();
        let listed = file.autosync.contains(&dataspace);
        if on == listed {
            return Ok(());
        }
        if on {
            file.autosync.push(dataspace);
        } else {
            file.autosync.retain(|listed| *listed != dataspace);
        }
        self.write_file(storage, &file).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::io::storage::LocalStorage;

    #[test(tokio::test)]
    async fn dataspaces_stay_on_autosync_in_the_order_turned_on() -> Res {
        let dir = tempfile::tempdir()?;
        let storage = LocalStorage::new();
        let domain = DomainRoot::new(dir.path().join("domain"));
        let [first, second, third] = ["first", "second", "third"].map(|name| dir.path().join(name));
        for dataspace in [&first, &second, &third] {
            domain.set_autosync(&storage, dataspace, true).await?;
        }
        domain.set_autosync(&storage, &first, true).await?;
        domain.set_autosync(&storage, &second, false).await?;

        let reopened = DomainRoot::new(dir.path().join("domain"));
        assert_eq!(reopened.autosync(&storage).await?, [first, third]);
        Ok(())
    }

    /// One dataspace spelled with `.` or `..` and without is one entry, named
    /// as its contract names it, so turning it off by either spelling turns it
    /// off.
    #[test(tokio::test)]
    async fn a_dataspace_is_named_as_its_contract_names_it() -> Res {
        let dir = tempfile::tempdir()?;
        let storage = LocalStorage::new();
        let domain = DomainRoot::new(dir.path().join("domain"));
        let dataspace = dir.path().join("rnaseq");
        let roundabout = dir.path().join("other").join("..").join(".").join("rnaseq");

        domain.set_autosync(&storage, &roundabout, true).await?;
        domain.set_autosync(&storage, &dataspace, true).await?;
        assert_eq!(
            domain.autosync(&storage).await?,
            std::slice::from_ref(&dataspace)
        );

        domain.set_autosync(&storage, &roundabout, false).await?;
        assert_eq!(domain.autosync(&storage).await?, Vec::<PathBuf>::new());
        Ok(())
    }

    /// Turning autosync on keeps what else the file holds.
    #[test(tokio::test)]
    async fn turning_autosync_on_keeps_the_rest_of_the_file() -> Res {
        let dir = tempfile::tempdir()?;
        let storage = LocalStorage::new();
        let domain = DomainRoot::new(dir.path());
        let file = DomainFile {
            default_dataspace: Some(dir.path().join("home")),
            autosync: Vec::new(),
            import: Some(ImportRecord {
                from: dir.path().join("old"),
                done: true,
            }),
        };
        domain.write_file(&storage, &file).await?;

        domain
            .set_autosync(&storage, &dir.path().join("home"), true)
            .await?;

        assert_eq!(
            domain.read_file(&storage).await?,
            Some(DomainFile {
                autosync: vec![dir.path().join("home")],
                ..file
            })
        );
        Ok(())
    }

    /// The refs store's `domain.json`, which holds one home, is not taken for
    /// this one.
    #[test(tokio::test)]
    async fn the_refs_stores_domain_json_is_refused() -> Res {
        let dir = tempfile::tempdir()?;
        let storage = LocalStorage::new();
        let domain = DomainRoot::new(dir.path());
        std::fs::write(
            domain.domain_file(),
            br#"{"format": 1, "home": "/home/me/QuiltSync", "migration": "done"}"#,
        )?;

        let read = domain.read_file(&storage).await;

        assert!(
            read.as_ref()
                .is_err_and(|err| err.to_string().contains("format 1")),
            "{read:?}"
        );
        Ok(())
    }
}
