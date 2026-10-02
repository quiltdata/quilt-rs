//! Remove old revisions of one package: their installed manifests, and the
//! objects only they used.
//!
//! The proof that an object is free is gc's: every installed manifest of
//! every package is read, and an object none of the kept ones names is one no
//! package can reach. The removal runs it for one package's objects only, and
//! never touches a revision the package still needs ([`protection`]).

use std::collections::BTreeMap;
use std::collections::BTreeSet;

use tracing::debug;
use tracing::info;

use super::HistoryEntry;
use super::gc::format_bytes;
use super::gc::list_files;
use super::gc::lock_every_package;
use super::gc::objects_in_use;
use super::gc::row_objects;
use crate::Error;
use crate::Res;
use crate::io::storage::Storage;
use crate::lineage::DomainLineage;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::paths::DomainPaths;
use quilt_uri::Namespace;

/// Why a revision is kept. Ordered as a row says them: `current · not pushed`,
/// `latest · base`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kept {
    /// The working files are at it.
    Current,
    /// The believed remote tip, which classify and pull compare against.
    Latest,
    /// The merge base, which classify and pull compare against.
    Base,
    /// On the pending commit chain, which push and undo read.
    NotPushed,
    /// The registry does not list it, so this copy is the only place it is.
    Unpublished,
}

impl std::fmt::Display for Kept {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Current => "current",
            Self::Latest => "latest",
            Self::Base => "base",
            Self::NotPushed => "not pushed",
            Self::Unpublished => "unpublished",
        })
    }
}

/// The revisions of `history` that may not be removed, each with every
/// reason it is kept, in [`Kept`]'s order. A revision absent from the map is
/// removable.
///
/// Publication is `history`'s, which is the registry's listing. A revision on
/// the pending chain is unpublished by definition, so it says `not pushed`
/// and not both.
#[must_use]
pub fn protection(
    lineage: &PackageLineage,
    history: &[HistoryEntry],
) -> BTreeMap<String, Vec<Kept>> {
    let chain: BTreeSet<&str> = lineage
        .commit
        .iter()
        .flat_map(|commit| std::iter::once(&commit.hash).chain(&commit.prev_hashes))
        .map(String::as_str)
        .collect();
    history
        .iter()
        .filter_map(|entry| {
            let hash = entry.revision.hash.as_str();
            let pending = chain.contains(hash);
            let reasons: Vec<Kept> = [
                (lineage.current_hash() == Some(hash), Kept::Current),
                (lineage.latest_hash == hash, Kept::Latest),
                (lineage.base_hash == hash, Kept::Base),
                (pending, Kept::NotPushed),
                (!pending && !entry.published, Kept::Unpublished),
            ]
            .into_iter()
            .filter_map(|(kept, reason)| kept.then_some(reason))
            .collect();
            (!reasons.is_empty()).then(|| (hash.to_string(), reasons))
        })
        .collect()
}

/// Which objects each of one package's revisions uses, and what removing a
/// set of them frees: the bytes of the objects only that set uses, with no
/// other revision of the package and no other package using them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RevisionUsage {
    /// Each object on disk that only this package's revisions use, with its
    /// file's length and the revisions using it.
    owned: BTreeMap<String, (u64, BTreeSet<String>)>,
}

impl RevisionUsage {
    /// Read every manifest of `namespace`, and ask gc's in-use read about
    /// their objects over every other package's. Fails on a manifest it cannot
    /// read, as gc does, rather than count its objects as free.
    pub async fn read(
        paths: &DomainPaths,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
    ) -> Res<Self> {
        let mut users: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
        let manifests = paths.installed_manifests_dir(namespace);
        for (hash, path, _) in list_files(storage, &manifests).await? {
            let manifest = Manifest::from_path(storage, &path).await?;
            for row in &manifest.rows {
                for name in row_objects(&row.hash, &row.physical_key) {
                    users.entry(name).or_default().insert(hash.clone());
                }
            }
        }

        // Only an object with a file can free anything.
        let mut owned: BTreeMap<String, (u64, BTreeSet<String>)> = BTreeMap::new();
        for (name, _, len) in list_files(storage, &paths.objects_dir()).await? {
            if let Some(revisions) = users.remove(&name) {
                owned.insert(name, (len, revisions));
            }
        }

        let candidates = owned.keys().cloned().collect();
        for name in objects_in_use(paths, storage, candidates, Some(namespace)).await? {
            owned.remove(&name);
        }
        Ok(Self { owned })
    }

    /// The objects removing `revisions` frees, with their lengths.
    fn freed(&self, revisions: &BTreeSet<String>) -> impl Iterator<Item = (&str, u64)> {
        self.owned
            .iter()
            .filter(|(_, (_, users))| users.is_subset(revisions))
            .map(|(name, (len, _))| (name.as_str(), *len))
    }

    /// The bytes removing `revisions` frees, measured for the set: an object
    /// shared only among them counts once they all go.
    #[must_use]
    pub fn frees(&self, revisions: &BTreeSet<String>) -> u64 {
        self.freed(revisions).map(|(_, len)| len).sum()
    }
}

/// What a [`remove_revisions`] did.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemovalReport {
    pub namespace: Namespace,
    /// Installed manifests deleted.
    pub revisions: usize,
    /// Object files deleted.
    pub objects: usize,
    /// The deleted objects' lengths, summed.
    pub bytes: u64,
    /// A package that was busy, so the objects were kept; gc frees them later.
    pub kept_for: Option<Namespace>,
    /// The kept objects' lengths, summed.
    pub kept_bytes: u64,
}

/// One sentence for every surface that reports a removal: "Removed 4 old
/// revisions of user/plate-07 · freed 630.2 kB".
impl std::fmt::Display for RemovalReport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let what = if self.revisions == 1 {
            "1 old revision".to_string()
        } else {
            format!("{} old revisions", self.revisions)
        };
        write!(f, "Removed {what} of {} \u{b7} ", self.namespace)?;
        match &self.kept_for {
            Some(busy) => write!(
                f,
                "kept {} while {busy} is busy",
                format_bytes(self.kept_bytes)
            ),
            None if self.bytes == 0 => write!(f, "freed no space"),
            None => write!(f, "freed {}", format_bytes(self.bytes)),
        }
    }
}

/// Delete `hashes`' installed manifests of `namespace`, then the objects only
/// they used.
///
/// The caller holds `namespace`'s lock and has checked that none of `hashes`
/// is protected. This try-locks every other package; if one is busy, the
/// manifests still go, every object they alone used is kept, and the report
/// names the package. An unreadable manifest anywhere deletes nothing.
pub async fn remove_revisions(
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    lineage: &DomainLineage,
    namespace: &Namespace,
    hashes: &BTreeSet<String>,
) -> Res<RemovalReport> {
    info!("⏳ Removing {} revisions of {namespace}", hashes.len());
    // Held to the end, so no other package files an object meanwhile.
    let (_held, kept_for) =
        match lock_every_package(paths, storage, lineage, Some(namespace)).await {
            Ok(held) => (held, None),
            Err(Error::PackageBusy(busy)) => (Vec::new(), Some(busy)),
            Err(err) => return Err(err),
        };

    let usage = RevisionUsage::read(paths, storage, namespace).await?;
    let freed: Vec<(String, u64)> = usage
        .freed(hashes)
        .map(|(name, len)| (name.to_string(), len))
        .collect();

    let mut report = RemovalReport {
        namespace: namespace.clone(),
        revisions: 0,
        objects: 0,
        bytes: 0,
        kept_for,
        kept_bytes: 0,
    };
    // Manifests first: a failure between the two leaves objects no manifest
    // uses, which gc frees, and never a manifest whose object is gone.
    for hash in hashes {
        debug!("🗑️ Removing revision {hash} of {namespace}");
        storage
            .remove_file(paths.installed_manifest(namespace, hash))
            .await?;
        report.revisions += 1;
    }
    for (name, len) in freed {
        if report.kept_for.is_some() {
            report.kept_bytes += len;
            continue;
        }
        debug!("🗑️ Removing object {name}");
        storage.remove_file(paths.objects_dir().join(&name)).await?;
        report.objects += 1;
        report.bytes += len;
    }

    info!("✔️ Removed revisions: {report:?}");
    Ok(report)
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::flow::Revision;
    use crate::io::storage::LocalStorage;
    use crate::lineage::CommitState;
    use crate::manifest::ManifestRow;
    use crate::package_lock;

    const PLATE: (&str, &str) = ("user", "plate-07");
    const OTHER: (&str, &str) = ("lab", "assays");

    /// Install revision `hash` of `namespace`, using the named objects by
    /// `file://` keys.
    fn revision(paths: &DomainPaths, namespace: (&str, &str), hash: &str, objects: &[&str]) -> Res {
        let rows = objects
            .iter()
            .map(|name| ManifestRow {
                logical_key: (*name).into(),
                physical_key: url::Url::from_file_path(paths.objects_dir().join(name))
                    .expect("absolute")
                    .to_string(),
                ..ManifestRow::default()
            })
            .collect();
        let manifest = Manifest {
            rows,
            ..Manifest::default()
        };
        let path = paths.installed_manifest(&namespace.into(), hash);
        std::fs::create_dir_all(path.parent().expect("parent"))?;
        std::fs::write(path, manifest.to_jsonlines())?;
        Ok(())
    }

    /// An object file of `len` bytes.
    fn object(paths: &DomainPaths, name: &str, len: usize) -> Res {
        std::fs::create_dir_all(paths.objects_dir())?;
        std::fs::write(paths.objects_dir().join(name), "x".repeat(len))?;
        Ok(())
    }

    fn set(hashes: &[&str]) -> BTreeSet<String> {
        hashes.iter().map(|h| (*h).to_string()).collect()
    }

    fn exists(paths: &DomainPaths, name: &str) -> bool {
        paths.objects_dir().join(name).exists()
    }

    fn installed(paths: &DomainPaths, namespace: (&str, &str), hash: &str) -> bool {
        paths.installed_manifest(&namespace.into(), hash).exists()
    }

    /// `plate-07`'s `r1` uses x and s, `r2` s and y, `r3` y; `lab/assays`
    /// uses x too. Sizes 1, 10, 100.
    fn fixture() -> Res<(DomainPaths, tempfile::TempDir)> {
        let (paths, dir) = DomainPaths::from_temp_dir()?;
        object(&paths, "x", 1)?;
        object(&paths, "s", 10)?;
        object(&paths, "y", 100)?;
        revision(&paths, PLATE, "r1", &["x", "s"])?;
        revision(&paths, PLATE, "r2", &["s", "y"])?;
        revision(&paths, PLATE, "r3", &["y"])?;
        revision(&paths, OTHER, "q1", &["x"])?;
        Ok((paths, dir))
    }

    #[test(tokio::test)]
    async fn a_set_frees_only_what_no_other_revision_or_package_uses() -> Res {
        let (paths, _dir) = fixture()?;
        let usage = RevisionUsage::read(&paths, &LocalStorage::new(), &PLATE.into()).await?;

        assert_eq!(usage.frees(&set(&["r1"])), 0, "x is lab's, s is r2's");
        assert_eq!(usage.frees(&set(&["r2"])), 0, "s is r1's, y is r3's");
        assert_eq!(usage.frees(&set(&["r1", "r2"])), 10, "s alone");
        assert_eq!(usage.frees(&set(&["r2", "r3"])), 100, "y alone");
        assert_eq!(usage.frees(&set(&["r1", "r2", "r3"])), 110);
        assert_eq!(usage.frees(&set(&[])), 0);
        Ok(())
    }

    /// A sparse copy never downloaded some objects: they free nothing.
    #[test(tokio::test)]
    async fn an_object_never_downloaded_frees_nothing() -> Res {
        let (paths, _dir) = DomainPaths::from_temp_dir()?;
        object(&paths, "here", 5)?;
        revision(&paths, PLATE, "r1", &["here", "not-here"])?;
        let usage = RevisionUsage::read(&paths, &LocalStorage::new(), &PLATE.into()).await?;

        assert_eq!(usage.frees(&set(&["r1"])), 5);
        Ok(())
    }

    #[test(tokio::test)]
    async fn removing_deletes_the_manifests_then_what_only_they_used() -> Res {
        let (paths, _dir) = fixture()?;
        let report = remove_revisions(
            &paths,
            &LocalStorage::new(),
            &DomainLineage::default(),
            &PLATE.into(),
            &set(&["r2", "r3"]),
        )
        .await?;

        assert!(!installed(&paths, PLATE, "r2"));
        assert!(!installed(&paths, PLATE, "r3"));
        assert!(installed(&paths, PLATE, "r1"));
        assert!(!exists(&paths, "y"));
        assert!(exists(&paths, "s"), "r1 still uses it");
        assert!(exists(&paths, "x"));
        assert_eq!(
            report,
            RemovalReport {
                namespace: PLATE.into(),
                revisions: 2,
                objects: 1,
                bytes: 100,
                kept_for: None,
                kept_bytes: 0,
            }
        );
        Ok(())
    }

    /// Another package syncing may be about to file one of these objects
    /// under a manifest of its own: keep them all, still drop the manifests.
    #[test(tokio::test)]
    async fn a_busy_other_package_keeps_the_objects() -> Res {
        let (paths, _dir) = fixture()?;
        let storage = LocalStorage::new();
        let _held = package_lock::lock(&storage, &paths, &OTHER.into()).await?;

        let report = remove_revisions(
            &paths,
            &storage,
            &DomainLineage::default(),
            &PLATE.into(),
            &set(&["r2", "r3"]),
        )
        .await?;

        assert!(!installed(&paths, PLATE, "r2"));
        assert!(exists(&paths, "y"));
        assert_eq!(report.objects, 0);
        assert_eq!(report.kept_for, Some(OTHER.into()));
        assert_eq!(report.kept_bytes, 100);
        Ok(())
    }

    /// The caller holds its own package's lock, so that one is not busy.
    #[test(tokio::test)]
    async fn the_callers_own_lock_is_not_a_busy_package() -> Res {
        let (paths, _dir) = fixture()?;
        let storage = LocalStorage::new();
        let _own = package_lock::lock(&storage, &paths, &PLATE.into()).await?;

        let report = remove_revisions(
            &paths,
            &storage,
            &DomainLineage::default(),
            &PLATE.into(),
            &set(&["r3"]),
        )
        .await?;

        assert_eq!(report.kept_for, None);
        Ok(())
    }

    #[test(tokio::test)]
    async fn an_unreadable_manifest_deletes_nothing() -> Res {
        let (paths, _dir) = fixture()?;
        std::fs::write(paths.installed_manifest(&OTHER.into(), "broken"), "nope")?;

        let result = remove_revisions(
            &paths,
            &LocalStorage::new(),
            &DomainLineage::default(),
            &PLATE.into(),
            &set(&["r2", "r3"]),
        )
        .await;

        assert!(result.is_err());
        assert!(installed(&paths, PLATE, "r2"));
        assert!(installed(&paths, PLATE, "r3"));
        assert!(exists(&paths, "y"));
        Ok(())
    }

    fn entry(hash: &str, published: bool) -> HistoryEntry {
        HistoryEntry {
            revision: Revision {
                hash: hash.to_string(),
                obtained: chrono::DateTime::UNIX_EPOCH,
                message: None,
            },
            published,
        }
    }

    #[test]
    fn current_base_latest_the_chain_and_unpublished_are_kept() {
        let lineage = PackageLineage {
            commit: Some(CommitState {
                hash: "mine".into(),
                prev_hashes: vec!["mine-before".into()],
                ..CommitState::default()
            }),
            base_hash: "agreed".into(),
            latest_hash: "agreed".into(),
            ..PackageLineage::default()
        };
        let history = [
            entry("mine", false),
            entry("mine-before", false),
            entry("agreed", true),
            entry("old", true),
            entry("never-sent", false),
        ];

        let kept = protection(&lineage, &history);

        assert_eq!(
            kept,
            BTreeMap::from([
                ("mine".into(), vec![Kept::Current, Kept::NotPushed]),
                ("mine-before".into(), vec![Kept::NotPushed]),
                ("agreed".into(), vec![Kept::Latest, Kept::Base]),
                ("never-sent".into(), vec![Kept::Unpublished]),
            ])
        );
    }

    #[test]
    fn the_report_says_what_went_and_what_it_freed() {
        let report = RemovalReport {
            namespace: PLATE.into(),
            revisions: 4,
            objects: 6,
            bytes: 630_200,
            kept_for: None,
            kept_bytes: 0,
        };
        assert_eq!(
            report.to_string(),
            "Removed 4 old revisions of user/plate-07 \u{b7} freed 630.2 kB"
        );
        let one = RemovalReport {
            revisions: 1,
            objects: 0,
            bytes: 0,
            ..report.clone()
        };
        assert_eq!(
            one.to_string(),
            "Removed 1 old revision of user/plate-07 \u{b7} freed no space"
        );
        let kept = RemovalReport {
            objects: 0,
            bytes: 0,
            kept_for: Some(OTHER.into()),
            kept_bytes: 6_900_000,
            ..report
        };
        assert_eq!(
            kept.to_string(),
            "Removed 4 old revisions of user/plate-07 \u{b7} kept 6.9 MB while lab/assays is busy"
        );
    }
}
