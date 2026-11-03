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
//! own history under `.quilt/named_packages/` is read back only as hashes
//! ([`Remote::published_revisions`](crate::io::remote::Remote::published_revisions)).

use chrono::DateTime;
use chrono::Utc;

use quilt_uri::Namespace;

use crate::Res;
use crate::io::storage::Storage;
use crate::manifest::Manifest;
use crate::manifest::ManifestHeader;
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
    /// The revision's user metadata; `None` when it has none, `null` and `{}`
    /// included.
    pub meta: Option<serde_json::Value>,
    /// The id of the workflow the revision was committed under, if any.
    pub workflow: Option<String>,
}

impl Revision {
    /// The facts `header` records about the revision at `hash`.
    #[must_use]
    pub fn from_header(hash: String, obtained: DateTime<Utc>, header: ManifestHeader) -> Self {
        let meta = header.user_meta.filter(|meta| match meta {
            serde_json::Value::Null => false,
            serde_json::Value::Object(fields) => !fields.is_empty(),
            _ => true,
        });
        Self {
            hash,
            obtained,
            message: header.message,
            meta,
            workflow: header.workflow.and_then(|w| w.id).map(|id| id.id),
        }
    }
}

/// One revision this copy holds, and whether the package's registry lists it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub revision: Revision,
    /// The registry of the lineage's remote lists it: a timestamped pointer
    /// to it exists. Asked, never inferred from the pending commit chain —
    /// see `published-grounding`.
    pub published: bool,
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
        if !is_revision(&entry).await? {
            continue;
        }

        let path = entry.path();
        let manifest = Manifest::from_path(storage, &path).await?;

        revisions.push(Revision::from_header(
            entry.file_name().to_string_lossy().into_owned(),
            entry.metadata().await?.modified()?.into(),
            manifest.header,
        ));
    }

    revisions.sort_by(|left, right| {
        right
            .obtained
            .cmp(&left.obtained)
            .then_with(|| left.hash.cmp(&right.hash))
    });

    Ok(revisions)
}

/// How many revisions of `namespace` this copy has — the length
/// `list_revisions` would return, without parsing any manifest.
pub async fn count_revisions(
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    namespace: &Namespace,
) -> Res<usize> {
    let mut entries = storage
        .read_dir(paths.installed_manifests_dir(namespace))
        .await?;
    let mut count = 0;
    while let Some(entry) = entries.next_entry().await? {
        if is_revision(&entry).await? {
            count += 1;
        }
    }
    Ok(count)
}

/// Every file in the manifests directory is a revision, except a hidden one: a
/// write's stranded `.tmp-<uuid>` (`io/storage/local.rs`) or the OS's own.
async fn is_revision(entry: &tokio::fs::DirEntry) -> Res<bool> {
    Ok(!entry.file_name().to_string_lossy().starts_with('.') && entry.file_type().await?.is_file())
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

    fn header(user_meta: Option<serde_json::Value>) -> ManifestHeader {
        ManifestHeader {
            user_meta,
            ..ManifestHeader::default()
        }
    }

    fn meta_of(header: ManifestHeader) -> Option<serde_json::Value> {
        Revision::from_header("h".to_string(), DateTime::UNIX_EPOCH, header).meta
    }

    #[test]
    fn empty_metadata_is_none() {
        assert_eq!(meta_of(header(None)), None);
        assert_eq!(meta_of(header(Some(serde_json::Value::Null))), None);
        assert_eq!(meta_of(header(Some(serde_json::json!({})))), None);
    }

    #[test]
    fn metadata_and_workflow_are_read() {
        let json = serde_json::json!({
            "version": "v0",
            "message": "m",
            "user_meta": {"plate": 7},
            "workflow": {
                "config": "s3://bucket/.quilt/workflows/config.yml",
                "id": "alpha",
                "schemas": {}
            }
        });
        let stamped: ManifestHeader = serde_json::from_value(json).unwrap();
        let revision = Revision::from_header("h".to_string(), DateTime::UNIX_EPOCH, stamped);
        assert_eq!(revision.meta, Some(serde_json::json!({"plate": 7})));
        assert_eq!(revision.workflow.as_deref(), Some("alpha"));
        assert_eq!(
            meta_of(header(Some(serde_json::json!([1])))),
            Some(serde_json::json!([1]))
        );
    }
}
