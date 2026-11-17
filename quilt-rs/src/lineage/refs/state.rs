//!
//! Where a domain stands in the move of its lineage from `data.json` to refs
//! and back, read from its files, so a move or a rollback that was killed
//! resumes from the step it reached.

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "the cut-over and the rollback read it")
)]

use std::path::PathBuf;

use serde::Deserialize;

use super::format::MigrationState;
use super::store::read_config;
use super::store::read_if_exists;
use crate::Res;
use crate::io::storage::Storage;
use crate::paths::DomainPaths;

/// What `data.json` becomes once the refs hold the lineage. Its home and its
/// packages are both null, so a build without refs fails to parse it whether
/// it reads the home or the packages, and cannot repair it by a home change,
/// as it would a file missing only its home.
pub(crate) const STUB: &[u8] = br#"{
  "home": null,
  "packages": null,
  "lineage": "refs",
  "note": "This domain's lineage is in .quilt/refs/ and .quilt/domain.json. A quilt that fails to read this file predates them: upgrade it."
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
    /// the domain config, so `source` holds the lineage.
    StubOnly { source: PathBuf },
    /// A move is writing the refs from `source`.
    Migrating { source: PathBuf },
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
    // A later move freezes its copy apart, keeping the first move's.
    let source = async || {
        let migrating = paths.lineage_migrating();
        if storage.exists(&migrating).await {
            migrating
        } else {
            paths.lineage_pre_refs()
        }
    };
    Ok(match migration {
        Some(MigrationState::Done) => DomainState::Refs,
        Some(MigrationState::InProgress) => DomainState::Migrating {
            source: source().await,
        },
        Some(MigrationState::RollingBack { archive }) => {
            let archive = paths.refs_archive(&archive);
            DomainState::RollingBack {
                archived: storage.exists(&archive).await,
                published: lineage().await?.is_some_and(|bytes| !is_stub(&bytes)),
                archive,
            }
        }
        None if lineage().await?.is_some_and(|bytes| is_stub(&bytes)) => DomainState::StubOnly {
            source: source().await,
        },
        None => DomainState::Legacy,
    })
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

    const ARCHIVE: &str = "refs.rolled-back-x";

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
            DomainState::StubOnly { source } => DomainState::StubOnly {
                source: name(source),
            },
            DomainState::Migrating { source } => DomainState::Migrating {
                source: name(source),
            },
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

    fn stub_only(source: &str) -> DomainState {
        DomainState::StubOnly {
            source: source.into(),
        }
    }

    fn migrating(source: &str) -> DomainState {
        DomainState::Migrating {
            source: source.into(),
        }
    }

    fn rolled_back(archived: bool, published: bool) -> DomainState {
        DomainState::RollingBack {
            archive: ARCHIVE.into(),
            archived,
            published,
        }
    }

    /// Each on-disk state the resume rules name, and which frozen copy a
    /// resumed move reads: what `data.json` and the domain config hold,
    /// whether a later move froze its own copy beside the first's, and
    /// whether the archive exists.
    #[test(tokio::test)]
    async fn each_state_a_move_or_rollback_leaves_is_told_apart() -> Res {
        let stub = || Some(STUB.to_vec());
        let in_progress = || Some(config(Some(json!("in_progress"))));
        let done = || Some(config(Some(json!("done"))));
        let rolling = || Some(config(rolling_back()));
        for (lineage, domain_config, second_move, archive, expected) in [
            (None, None, false, false, DomainState::Legacy),
            (Some(legacy()), None, false, false, DomainState::Legacy),
            (
                Some(legacy()),
                Some(config(None)),
                false,
                false,
                DomainState::Legacy,
            ),
            (stub(), None, false, false, stub_only("data.json.pre-refs")),
            (
                stub(),
                Some(config(None)),
                false,
                false,
                stub_only("data.json.pre-refs"),
            ),
            (stub(), None, true, false, stub_only("data.json.migrating")),
            (
                stub(),
                in_progress(),
                false,
                false,
                migrating("data.json.pre-refs"),
            ),
            (
                stub(),
                in_progress(),
                true,
                false,
                migrating("data.json.migrating"),
            ),
            (stub(), done(), false, false, DomainState::Refs),
            // A preview domain, stood up on refs with no move.
            (None, done(), false, false, DomainState::Refs),
            (stub(), rolling(), false, false, rolled_back(false, false)),
            (stub(), rolling(), false, true, rolled_back(true, false)),
            (
                Some(legacy()),
                rolling(),
                false,
                true,
                rolled_back(true, true),
            ),
        ] {
            let root = tempfile::TempDir::new()?;
            let paths = DomainPaths::new(root.path().to_path_buf());
            std::fs::create_dir_all(paths.dot_quilt_dir())?;
            if let Some(bytes) = &lineage {
                std::fs::write(paths.lineage(), bytes)?;
                if is_stub(bytes) {
                    std::fs::write(paths.lineage_pre_refs(), legacy())?;
                }
            }
            if second_move {
                std::fs::write(paths.lineage_migrating(), legacy())?;
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

    #[test]
    fn the_stub_is_told_by_its_marker() {
        assert!(is_stub(STUB));
        assert!(is_stub(br#"{"lineage":"refs","note":"another"}"#));
        assert!(!is_stub(&legacy()));
        assert!(!is_stub(br#"{"lineage":"data.json"}"#));
        assert!(!is_stub(b"{"));
    }

    /// The stub fails to parse as a lineage, not merely for lacking a home,
    /// so the `data.json` store neither reads it nor repairs it.
    #[test(tokio::test)]
    async fn a_build_on_data_json_cannot_read_or_rewrite_the_stub() -> Res {
        let root = tempfile::TempDir::new()?;
        let paths = DomainPaths::new(root.path().to_path_buf());
        std::fs::create_dir_all(paths.dot_quilt_dir())?;
        std::fs::write(paths.lineage(), STUB)?;
        let io = DomainLineageIo::new(paths.lineage());
        let storage = LocalStorage::new();

        let parsed = DomainLineage::from_slice(STUB);
        assert!(unparsed(&parsed), "{parsed:?}");
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
