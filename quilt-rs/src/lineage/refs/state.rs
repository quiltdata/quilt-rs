//!
//! Where a domain stands in the move of its lineage from `data.json` to refs
//! and back, read from its files, so a move or a rollback that was killed
//! resumes from the step it reached.

use std::path::PathBuf;

use serde::Deserialize;
use uuid::Uuid;

use super::format::MigrationState;
use super::store::read_config;
use super::store::read_if_exists;
use crate::Res;
use crate::error::LineageError;
use crate::io::storage::Storage;
use crate::paths::DomainPaths;

/// What `data.json` becomes once the refs hold the lineage. A build without
/// refs fails to parse it whether it reads the home or the packages, so no
/// home change repairs it, and quotes the first field it fails on, so
/// `packages` comes first and holds the instruction.
pub(crate) const STUB: &[u8] = br#"{
  "packages": "This domain's lineage has moved to .quilt/refs/ and this quilt is too old to read it: upgrade quilt, or run `quilt lineage rollback` with a newer quilt to move it back.",
  "home": null,
  "lineage": "refs"
}
"#;

/// Whether `bytes` are the stub, told by its marker, whatever its layout.
pub(crate) fn is_stub(bytes: &[u8]) -> bool {
    #[derive(Deserialize)]
    struct Marker {
        lineage: Option<String>,
    }
    serde_json::from_slice::<Marker>(bytes)
        .is_ok_and(|marker| marker.lineage.as_deref() == Some("refs"))
}

/// Where the domain's lineage is.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum DomainState {
    /// In `data.json`, or nowhere yet: no move has put the stub in place.
    Legacy,
    /// A move was killed after putting the stub in place and before writing
    /// the domain config, so its frozen copy holds the lineage.
    StubOnly,
    /// A move is writing the refs from its frozen copy.
    Migrating,
    /// In the refs.
    Refs,
    /// Going back to `data.json`.
    RollingBack {
        /// Where the refs go.
        archive: PathBuf,
        /// The refs are there.
        archived: bool,
        /// `data.json` has replaced the stub.
        published: bool,
    },
}

/// Reads the domain's state without a lock: a mover re-reads it under the
/// migrate lock. A domain config that does not parse is an error naming it;
/// one with no migration state counts as none.
pub(crate) async fn detect(
    storage: &(impl Storage + Sync),
    paths: &DomainPaths,
) -> Res<DomainState> {
    let migration = read_config(storage, paths)
        .await?
        .and_then(|config| config.migration);
    let lineage_path = paths.lineage();
    let lineage = || read_if_exists(storage, &lineage_path);
    Ok(match migration {
        Some(MigrationState::Done) => DomainState::Refs,
        Some(MigrationState::InProgress) => DomainState::Migrating,
        Some(MigrationState::RollingBack { archive }) => {
            let archive = archive_path(paths, &archive)?;
            DomainState::RollingBack {
                archived: storage.exists(&archive).await,
                published: lineage().await?.is_some_and(|bytes| !is_stub(&bytes)),
                archive,
            }
        }
        None if lineage().await?.is_some_and(|bytes| is_stub(&bytes)) => DomainState::StubOnly,
        None => DomainState::Legacy,
    })
}

const ARCHIVE_PREFIX: &str = "refs.rolled-back-";

/// The name a rollback gives its archive.
pub(crate) fn archive_name(uuid: Uuid) -> String {
    format!("{ARCHIVE_PREFIX}{uuid}")
}

/// `.quilt/<name>`, for a name a rollback gives its archive: any other, such
/// as `refs`, `..` or an absolute path, is a damaged domain config.
fn archive_path(paths: &DomainPaths, name: &str) -> Res<PathBuf> {
    let named = name
        .strip_prefix(ARCHIVE_PREFIX)
        .and_then(|uuid| Uuid::try_parse(uuid).ok())
        .is_some_and(|uuid| archive_name(uuid) == name);
    if !named {
        return Err(LineageError::FileParse {
            path: paths.domain_config(),
            source: serde::de::Error::custom(format!("{name:?} is not a rollback's archive")),
        }
        .into());
    }
    Ok(paths.refs_archive(name))
}

#[cfg(test)]
mod tests {
    use serde_json::json;
    use test_log::test;

    use super::*;
    use crate::Error;
    use crate::error::LineageError;
    use crate::io::storage::LocalStorage;
    use crate::lineage::DomainLineage;
    use crate::lineage::DomainLineageIo;

    const ARCHIVE: &str = "refs.rolled-back-67e55044-10b1-426f-9247-bb680e5fe0c8";

    fn legacy() -> Vec<u8> {
        serde_json::to_vec(&json!({"home": "/home", "packages": {}})).unwrap()
    }

    fn config(migration: Option<serde_json::Value>) -> Vec<u8> {
        let mut config = json!({"format": 1, "home": "/home"});
        if let Some(migration) = migration {
            config["migration"] = migration;
        }
        serde_json::to_vec(&config).unwrap()
    }

    fn rolling_back() -> Option<serde_json::Value> {
        Some(json!({"rolling_back": {"archive": ARCHIVE}}))
    }

    fn unparsed(result: &Res<impl std::fmt::Debug>) -> bool {
        matches!(result, Err(Error::Lineage(LineageError::Parse(_))))
    }

    /// The files a state names, by their name in `.quilt/`.
    fn named(state: DomainState) -> DomainState {
        let name = |path: PathBuf| PathBuf::from(path.file_name().expect("a file"));
        match state {
            DomainState::RollingBack {
                archive,
                archived,
                published,
            } => DomainState::RollingBack {
                archive: name(archive),
                archived,
                published,
            },
            state => state,
        }
    }

    fn rolled_back(archived: bool, published: bool) -> DomainState {
        DomainState::RollingBack {
            archive: ARCHIVE.into(),
            archived,
            published,
        }
    }

    /// Each on-disk state the resume rules name: what `data.json` and the
    /// domain config hold, and whether the archive exists.
    #[test(tokio::test)]
    async fn each_state_a_move_or_rollback_leaves_is_told_apart() -> Res {
        let stub = || Some(STUB.to_vec());
        let in_progress = || Some(config(Some(json!("in_progress"))));
        let done = || Some(config(Some(json!("done"))));
        let rolling = || Some(config(rolling_back()));
        for (lineage, domain_config, archive, expected) in [
            (None, None, false, DomainState::Legacy),
            (Some(legacy()), None, false, DomainState::Legacy),
            (
                Some(legacy()),
                Some(config(None)),
                false,
                DomainState::Legacy,
            ),
            (stub(), None, false, DomainState::StubOnly),
            (stub(), Some(config(None)), false, DomainState::StubOnly),
            (stub(), in_progress(), false, DomainState::Migrating),
            (stub(), done(), false, DomainState::Refs),
            // A preview domain, stood up on refs with no move.
            (None, done(), false, DomainState::Refs),
            (stub(), rolling(), false, rolled_back(false, false)),
            (stub(), rolling(), true, rolled_back(true, false)),
            (Some(legacy()), rolling(), true, rolled_back(true, true)),
        ] {
            let root = tempfile::TempDir::new()?;
            let paths = DomainPaths::new(root.path().to_path_buf());
            std::fs::create_dir_all(paths.dot_quilt_dir())?;
            if let Some(bytes) = &lineage {
                std::fs::write(paths.lineage(), bytes)?;
            }
            if let Some(bytes) = &domain_config {
                std::fs::write(paths.domain_config(), bytes)?;
            }
            if archive {
                std::fs::create_dir(paths.refs_archive(ARCHIVE))?;
            }

            let found = detect(&LocalStorage::new(), &paths).await?;
            assert_eq!(named(found), expected);
        }
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_domain_config_that_does_not_parse_is_an_error_naming_it() -> Res {
        let root = tempfile::TempDir::new()?;
        let paths = DomainPaths::new(root.path().to_path_buf());
        std::fs::create_dir_all(paths.dot_quilt_dir())?;
        std::fs::write(paths.domain_config(), b"{")?;

        let found = detect(&LocalStorage::new(), &paths).await;

        assert!(
            matches!(
                &found,
                Err(Error::Lineage(LineageError::FileParse { path, .. })) if *path == paths.domain_config()
            ),
            "{found:?}"
        );
        Ok(())
    }

    /// The stub the preview builds wrote.
    const PREVIEW_STUB: &[u8] = br#"{
  "home": null,
  "packages": null,
  "lineage": "refs",
  "note": "This domain's lineage is in .quilt/refs/ and .quilt/domain.json. A quilt that fails to read this file predates them: upgrade it."
}
"#;

    #[test]
    fn the_stub_is_told_by_its_marker() {
        assert!(is_stub(STUB));
        assert!(is_stub(PREVIEW_STUB));
        assert!(is_stub(br#"{"lineage":"refs","note":"another"}"#));
        assert!(!is_stub(&legacy()));
        assert!(!is_stub(br#"{"lineage":"data.json"}"#));
        assert!(!is_stub(b"{"));
    }

    /// The stub fails to parse as a lineage, not merely for lacking a home,
    /// so the `data.json` store neither reads it nor repairs it, and the
    /// error a whole-file read gives tells the user what to do.
    #[test(tokio::test)]
    async fn a_build_on_data_json_cannot_read_or_rewrite_the_stub() -> Res {
        let root = tempfile::TempDir::new()?;
        let paths = DomainPaths::new(root.path().to_path_buf());
        std::fs::create_dir_all(paths.dot_quilt_dir())?;
        std::fs::write(paths.lineage(), STUB)?;
        let io = DomainLineageIo::data_json(paths.lineage());
        let storage = LocalStorage::new();

        let parsed = DomainLineage::from_slice(STUB);
        assert!(unparsed(&parsed), "{parsed:?}");
        let told = parsed.unwrap_err().to_string();
        assert!(
            told.contains("upgrade quilt, or run `quilt lineage rollback`"),
            "{told}"
        );
        let home = io.home(&storage).await;
        assert!(unparsed(&home), "{home:?}");
        let packages = io.read_packages(&storage).await;
        assert!(unparsed(&packages), "{packages:?}");
        let set = io.set_home(&storage, root.path().join("home")).await;
        assert!(unparsed(&set), "{set:?}");
        assert_eq!(std::fs::read(paths.lineage())?, STUB);
        Ok(())
    }
}
