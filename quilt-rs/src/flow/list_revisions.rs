//! Which revisions of a package a copy has, newest first.
//!
//! The manifests under `.quilt/installed/<namespace>/` are the authority for
//! this, and the lineage is not: `PackageLineage::commit` is a *pending-push*
//! record — [`push`](super::push) clears it on success and
//! [`reset_to_latest`](super::reset_to_latest) clears it outright — so it
//! answers "what have I committed that is not pushed" rather than "what
//! revisions exist".
//!
//! Order therefore comes from the manifest file's mtime, which means *when this
//! copy obtained the revision*: the commit time for a revision committed here,
//! the fetch time for one pulled from a remote. Nothing on disk records when a
//! revision was **made** — the manifest format carries neither a timestamp nor
//! a parent pointer (see
//! [`tag_timestamp`](crate::io::manifest::tag_timestamp)), and the registry's
//! own history under `.quilt/named_packages/` cannot be read back because
//! [`Remote`](crate::io::remote::Remote) has no list operation.

use chrono::DateTime;
use chrono::Utc;

use quilt_uri::Namespace;

use crate::Res;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use crate::paths::DomainPaths;

/// One revision this copy holds a manifest for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Revision {
    /// Top hash of the revision's manifest.
    pub hash: String,
    /// When *this copy* obtained the revision. See the module docs: this is not
    /// when the revision was made, and for a pulled revision it is the fetch
    /// time.
    pub obtained: DateTime<Utc>,
    /// The revision's commit message, absent if it was committed without one.
    pub message: Option<String>,
}

/// List the revisions of `namespace` this copy has, newest `obtained` first.
///
/// Hash breaks a genuine mtime tie so the order is stable across calls.
pub async fn list_revisions(
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    namespace: &Namespace,
) -> Res<Vec<Revision>> {
    let manifest_dir = paths.installed_manifests_dir(namespace);
    let mut entries = storage.read_dir(&manifest_dir).await?;
    let mut revisions = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        if !entry.file_type().await?.is_file() {
            continue;
        }

        let path = entry.path();
        let manifest = Manifest::from_path(storage, &path).await?;

        revisions.push(Revision {
            hash: entry.file_name().to_string_lossy().into_owned(),
            obtained: entry.metadata().await?.modified()?.into(),
            message: manifest.header.message,
        });
    }

    revisions.sort_by(|left, right| {
        right
            .obtained
            .cmp(&left.obtained)
            .then_with(|| left.hash.cmp(&right.hash))
    });

    Ok(revisions)
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::flow::create;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::DomainLineage;

    /// A created package's manifest is enumerated, with its header message.
    ///
    /// Ordering is exercised end-to-end against real commit sequences in
    /// `quilt-cli`'s `history` tests, which drive several revisions through
    /// `commit` — more setup than belongs here.
    #[test(tokio::test)]
    async fn test_list_revisions_reads_the_installed_manifest() -> Res {
        let (lineage, _temp_dir) = DomainLineage::from_temp_dir()?;
        let (paths, _temp_dir2) = DomainPaths::from_temp_dir()?;
        let storage = MockStorage::default();
        let namespace: Namespace = ("demo", "sales").into();

        create(
            lineage,
            &paths,
            &storage,
            namespace.clone(),
            None,
            Some("initial import".to_string()),
        )
        .await?;

        let listed = list_revisions(&paths, &storage, &namespace).await?;
        assert_eq!(listed.len(), 1);
        assert_eq!(listed[0].message.as_deref(), Some("initial import"));

        Ok(())
    }
}
