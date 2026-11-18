//! The record step: the engine's one local write path.
//!
//! A manifest and the content for its rows go in, and a new revision of the
//! package comes out in a [`PackageStore`], as the lineage's pending commit.
//! [`commit`](super::commit()) is a working folder's changes collected into rows,
//! then recorded; a build of an in-memory package is the same manifest and
//! content, recorded with no folder at all. Nothing here reads or writes a
//! working folder, and nothing here touches the network except the workflow
//! check.
//!
//! The local counterpart of [`push_files`](super::push_files()), which takes the
//! same inputs and publishes them to a remote.

use std::collections::BTreeMap;
use std::collections::HashSet;
use std::fmt;
use std::path::Path;
use std::path::PathBuf;

use tracing::debug;
use tracing::info;

use crate::Error;
use crate::LogicalKey;
use crate::Res;
use crate::checksum::hash_file;
use crate::error::PackageOpError;
use crate::error::RemoteCatalogError;
use crate::flow::Committed;
use crate::io::manifest::RowContent;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::remote::entry_view;
use crate::io::remote::validate_workflow;
use crate::io::remote::validate_workflow_against_current_config;
use crate::io::storage::ByteStream;
use crate::io::storage::Storage;
use crate::lineage::CommitState;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::manifest::ManifestHeader;
use crate::manifest::ManifestRow;
use crate::manifest::TopHasher;
use crate::object_hash::ObjectHash;
use crate::paths::DomainPaths;
use crate::workflow::EntryView;
use quilt_uri::Host;
use quilt_uri::Namespace;

/// Where a recorded revision's objects and manifest are written.
///
/// Today the domain's one shared store, `$DOMAIN/.quilt/`
/// ([`DomainPaths`]), is the only implementation: every package's objects
/// share `objects/`, and its manifests are `installed/<owner>/<name>/<hash>`.
/// The record step takes the store as a parameter rather than reaching for
/// `$DOMAIN` itself, so that a store per registry
/// ([quilt-specs#299](https://github.com/quiltdata/quilt-specs/pull/299)) is
/// another implementation of this trait and the same write path.
pub trait PackageStore: Sync {
    /// The folder holding the store's objects, each named by the hex digest
    /// of its hash.
    fn objects_dir(&self) -> PathBuf;

    /// The folder holding the installed manifests of `namespace`, each named
    /// by its top hash.
    fn manifests_dir(&self, namespace: &Namespace) -> PathBuf;

    /// Where the object with `hash` lives in this store.
    fn object_path(&self, hash: &ObjectHash) -> PathBuf {
        self.objects_dir().join(hex::encode(hash.digest()))
    }
}

/// The domain's shared store, `$DOMAIN/.quilt/`.
impl PackageStore for DomainPaths {
    fn objects_dir(&self) -> PathBuf {
        DomainPaths::objects_dir(self)
    }

    fn manifests_dir(&self, namespace: &Namespace) -> PathBuf {
        self.installed_manifests_dir(namespace)
    }
}

/// How [`record`] checks the new revision against a bucket's workflows.
#[derive(Clone, Copy, Debug)]
pub enum WorkflowCheck<'a> {
    /// Refuse, before any manifest is written, a revision that the workflow
    /// pinned in its header rejects, with [`Error::WorkflowValidation`]. For a
    /// commit that is about to be pushed, as `publish` does: the push would
    /// refuse it anyway, and this way no pending commit is left behind. `host`
    /// is the catalog that signs the config reads.
    Gate { host: Option<&'a Host> },
    /// A local commit or build: check the revision against the **current**
    /// workflows config of the lineage's remote bucket and report what is
    /// inconsistent as [`WorkflowWarning`]s, without refusing anything. A
    /// package with no remote gets no check and no warning. A push still
    /// validates as it always has.
    Warn { host: Option<&'a Host> },
}

/// Something a [`WorkflowCheck::Warn`] check found. The revision was recorded
/// anyway.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum WorkflowWarning {
    /// The revision is inconsistent with the bucket's workflows config, so a
    /// push there would refuse it as it stands.
    Inconsistent { bucket: String, reason: String },
    /// The bucket's workflows config or schemas could not be read, so the
    /// revision was not checked.
    Unchecked { bucket: String, reason: String },
}

impl fmt::Display for WorkflowWarning {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Inconsistent { bucket, reason } => write!(
                f,
                "this revision does not satisfy the workflows of s3://{bucket}, so pushing it there will be refused until it does: {reason}"
            ),
            Self::Unchecked { bucket, reason } => write!(
                f,
                "this revision was not checked against the workflows of s3://{bucket}: {reason}"
            ),
        }
    }
}

/// What [`record`] did, and what its workflow check found.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Recorded {
    pub committed: Committed,
    /// Empty unless the check was [`WorkflowCheck::Warn`] and found something.
    pub warnings: Vec<WorkflowWarning>,
}

impl Recorded {
    /// The recorded revision's top hash, `None` when nothing was recorded.
    #[must_use]
    pub fn hash(&self) -> Option<&str> {
        self.committed.hash()
    }

    /// `self` with `unchecked`, the warning of a workflow resolution that
    /// could not read the config, unless nothing was recorded or the check
    /// already says the revision went unchecked.
    #[must_use]
    pub fn with_unchecked(mut self, unchecked: Option<WorkflowWarning>) -> Self {
        if let Some(warning) = unchecked
            && self.hash().is_some()
            && !self
                .warnings
                .iter()
                .any(|w| matches!(w, WorkflowWarning::Unchecked { .. }))
        {
            self.warnings.push(warning);
        }
        self
    }
}

/// Record `manifest` as a new revision of `namespace` in `store`, on top of
/// `lineage`'s current revision, and return `lineage` with it as the pending
/// commit.
///
/// Where each row's bytes come from:
///
/// 1. `files`, keyed by logical key: a file anywhere on disk, or the bytes
///    themselves ([`RowContent`]).
/// 2. A `file://` `physical_key` outside the store's objects folder: that
///    file.
///
/// Either way the bytes are copied into the store's objects folder (a clone,
/// where the volume and platform support one), and the copy is hashed with the
/// row's own algorithm before it takes its name. A row whose bytes no longer
/// hash to its `hash`, such as a file that changed after it was hashed, is an
/// error, and nothing is recorded. A row with no hash
/// ([`ObjectHash::default`]) is hashed with the default algorithm. The row's
/// `size` is the copy's, and its `physical_key` is rewritten to the copy, so
/// the revision stays correct when the original is edited or moved.
///
/// A row whose object the store already holds is not copied again. Any other
/// row, such as one that names a file in the store's objects folder or an
/// `s3://` object, is kept as it is, as a commit carries forward the rows it
/// did not change. A key in `files` that the manifest has no row for is
/// refused before anything is copied.
///
/// When the new top hash is the current revision's, nothing is written or
/// checked, and the lineage comes back as it was. Otherwise the manifest,
/// with its `dir_meta`, goes to the store's manifests folder for `namespace`,
/// after the workflow check `workflows` asks for.
///
/// The caller holds the package's lock, and writes the returned lineage.
#[allow(clippy::too_many_arguments, reason = "the parts a record uses")]
pub async fn record(
    store: &impl PackageStore,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    workflows: WorkflowCheck<'_>,
    namespace: &Namespace,
    mut lineage: PackageLineage,
    manifest: Manifest,
    files: &BTreeMap<LogicalKey, RowContent>,
) -> Res<(PackageLineage, Recorded)> {
    let Manifest {
        header,
        rows,
        dir_meta,
    } = manifest;
    let known: HashSet<&LogicalKey> = rows.iter().map(|row| &row.logical_key).collect();
    if let Some(key) = files.keys().find(|key| !known.contains(key)) {
        return Err(Error::PackageOp(PackageOpError::Commit(format!(
            "Content was given for \"{}\", but the manifest has no row for it",
            key.display()
        ))));
    }

    let mut placed = Vec::with_capacity(rows.len());
    for row in rows {
        let content = files.get(&row.logical_key);
        placed.push(place_row(store, storage, row, content).await?);
    }
    placed.sort_by(|a, b| a.logical_key.cmp(&b.logical_key));

    // Before the workflow check, so an unchanged revision is not checked again.
    let new_top_hash = top_hash(&header, &placed)?;
    if lineage.current_hash() == Some(new_top_hash.as_str()) {
        info!("✔️ Nothing to record: the top hash is still {new_top_hash}");
        return Ok((
            lineage,
            Recorded {
                committed: Committed::NothingToCommit,
                warnings: Vec::new(),
            },
        ));
    }

    let warnings =
        check_workflows(remote, workflows, &lineage, namespace, &header, &placed).await?;
    // Returned for the caller to show; logged only for the record.
    for warning in &warnings {
        info!("{namespace}: {warning}");
    }

    let stream = tokio_stream::iter(vec![Ok(placed.into_iter().map(Ok).collect())]);
    let (manifest_path, new_top_hash) = build_manifest_from_rows_stream(
        storage,
        store.manifests_dir(namespace),
        header,
        dir_meta,
        stream,
    )
    .await?;
    info!("✔️ Recorded {} as {new_top_hash}", manifest_path.display());

    let mut prev_hashes = Vec::new();
    if let Some(commit) = lineage.commit.take() {
        prev_hashes.push(commit.hash);
        prev_hashes.extend(commit.prev_hashes);
    }
    let commit = CommitState {
        hash: new_top_hash,
        timestamp: chrono::Utc::now(),
        prev_hashes,
    };
    lineage.commit = Some(commit.clone());
    Ok((
        lineage,
        Recorded {
            committed: Committed::Recorded(commit),
            warnings,
        },
    ))
}

/// `row` with its bytes in the store, or as it is when they need no copy.
async fn place_row(
    store: &impl PackageStore,
    storage: &(impl Storage + Sync),
    row: ManifestRow,
    content: Option<&RowContent>,
) -> Res<ManifestRow> {
    if let Some(content) = content {
        return store_content(store, storage, row, content).await;
    }
    let path = url::Url::parse(&row.physical_key)
        .ok()
        .filter(|url| url.scheme() == "file")
        .and_then(|url| url.to_file_path().ok());
    match path {
        Some(path) if path.parent() != Some(store.objects_dir().as_path()) => {
            store_content(store, storage, row, &RowContent::Path(path)).await
        }
        // In the store already, an `s3://` object, or a row carried forward
        // as the revision it came from had it.
        _ => Ok(row),
    }
}

/// Copy `content` into the store under the name of its hash, checked against
/// `row`'s, and return `row` pointing at the copy.
async fn store_content(
    store: &impl PackageStore,
    storage: &(impl Storage + Sync),
    row: ManifestRow,
    content: &RowContent,
) -> Res<ManifestRow> {
    let unhashed = row.hash == ObjectHash::default();
    if !unhashed {
        let dest = store.object_path(&row.hash);
        if storage.is_file(&dest).await {
            debug!("✔️ The store already holds {}", dest.display());
            let size = storage.open_file(&dest).await?.metadata().await?.len();
            return with_object(row, &dest, size);
        }
    }

    let objects_dir = store.objects_dir();
    storage.create_dir_all(&objects_dir).await?;
    // Hidden, so a gc leaves a copy in flight, or one a kill stranded, alone.
    let temp = objects_dir.join(format!(".tmp-{}", uuid::Uuid::new_v4()));
    let stored = async {
        match content {
            RowContent::Path(path) => {
                storage.copy(path, &temp).await?;
            }
            RowContent::Bytes(bytes) => {
                storage
                    .write_byte_stream(&temp, ByteStream::from(bytes.clone()))
                    .await?;
            }
        }
        let algorithm = if unhashed {
            HostConfig::default().checksums.algorithm_code()
        } else {
            row.hash.algorithm()
        };
        let (hash, size) = hash_file(storage, &temp, algorithm).await?;
        if !unhashed && hash != row.hash {
            return Err(Error::PackageOp(PackageOpError::Commit(format!(
                "\"{}\" no longer hashes to {}: its content changed after it was hashed",
                row.logical_key.display(),
                row.hash
            ))));
        }
        let dest = store.object_path(&hash);
        if storage.is_file(&dest).await {
            storage.remove_file(&temp).await?;
        } else {
            storage.rename(&temp, &dest).await?;
        }
        with_object(ManifestRow { hash, ..row }, &dest, size)
    }
    .await;
    if stored.is_err() && storage.exists(&temp).await {
        let _ = storage.remove_file(&temp).await;
    }
    stored
}

fn with_object(row: ManifestRow, object: &Path, size: u64) -> Res<ManifestRow> {
    Ok(ManifestRow {
        physical_key: crate::paths::file_url(object)?,
        size,
        ..row
    })
}

async fn check_workflows(
    remote: &impl Remote,
    workflows: WorkflowCheck<'_>,
    lineage: &PackageLineage,
    namespace: &Namespace,
    header: &ManifestHeader,
    rows: &[ManifestRow],
) -> Res<Vec<WorkflowWarning>> {
    let entries: Vec<EntryView> = rows.iter().map(entry_view).collect();
    let name = namespace.to_string();
    match workflows {
        // Vacuously passes for an ungoverned bucket: the header carries no
        // workflow.
        WorkflowCheck::Gate { host } => {
            validate_workflow(
                remote,
                host,
                &name,
                header.message(),
                header.user_meta.as_ref(),
                header.workflow.as_ref(),
                &entries,
            )
            .await?;
            Ok(Vec::new())
        }
        WorkflowCheck::Warn { host } => {
            let Some(bucket) = lineage
                .remote_uri
                .as_ref()
                .map(|remote| remote.bucket.as_str())
                .filter(|bucket| !bucket.is_empty())
            else {
                return Ok(Vec::new());
            };
            let checked = validate_workflow_against_current_config(
                remote,
                host,
                bucket,
                &name,
                header.message(),
                header.user_meta.as_ref(),
                header.workflow.as_ref(),
                &entries,
            )
            .await;
            let bucket = bucket.to_string();
            Ok(match checked {
                Ok(()) => Vec::new(),
                Err(Error::WorkflowValidation(err)) => vec![WorkflowWarning::Inconsistent {
                    bucket,
                    reason: err.to_string(),
                }],
                Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(reason))) => {
                    vec![WorkflowWarning::Inconsistent { bucket, reason }]
                }
                Err(err) => vec![WorkflowWarning::Unchecked {
                    bucket,
                    reason: err.to_string(),
                }],
            })
        }
    }
}

fn top_hash(header: &ManifestHeader, rows: &[ManifestRow]) -> Res<String> {
    let mut top_hasher = TopHasher::new();
    top_hasher.append_header(header)?;
    for row in rows {
        top_hasher.append(row)?;
    }
    Ok(top_hasher.finalize())
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use bytes::Bytes;

    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::LocalStorage;
    use crate::object_hash::MULTIHASH_SHA256_CHUNKED;
    use quilt_uri::ManifestUri;
    use quilt_uri::S3Uri;

    fn namespace() -> Namespace {
        ("acme", "built").into()
    }

    fn key(name: &str) -> LogicalKey {
        LogicalKey::new(name).expect("a valid key")
    }

    /// A row for `name` carrying `hash`, with no physical key of its own.
    fn row(name: &str, hash: ObjectHash) -> ManifestRow {
        ManifestRow {
            logical_key: key(name),
            hash,
            ..ManifestRow::default()
        }
    }

    fn revision(message: &str, rows: Vec<ManifestRow>) -> Manifest {
        Manifest {
            header: ManifestHeader {
                message: crate::manifest::HeaderMessage::Text(message.to_string()),
                ..ManifestHeader::default()
            },
            rows,
            ..Manifest::default()
        }
    }

    /// A file outside the store holding `bytes`, and its sha2-256-chunked hash.
    async fn file_anywhere(dir: &Path, name: &str, bytes: &[u8]) -> Res<(PathBuf, ObjectHash)> {
        let path = dir.join(name);
        std::fs::write(&path, bytes)?;
        let (hash, _) = hash_file(&LocalStorage::new(), &path, MULTIHASH_SHA256_CHUNKED).await?;
        Ok((path, hash))
    }

    fn manifest_names(dir: &Path) -> Vec<String> {
        std::fs::read_dir(dir).map_or_else(
            |_| Vec::new(),
            |entries| {
                entries
                    .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
                    .collect()
            },
        )
    }

    /// The store the record step writes to is the caller's: here one rooted
    /// outside `$DOMAIN`, standing in for a registry's own store.
    struct OtherStore(PathBuf);

    impl PackageStore for OtherStore {
        fn objects_dir(&self) -> PathBuf {
            self.0.join("objects")
        }

        fn manifests_dir(&self, namespace: &Namespace) -> PathBuf {
            self.0.join("manifests").join(namespace.to_string())
        }
    }

    /// A build: an in-memory package with bytes in memory, a file elsewhere
    /// on disk and an S3 object, recorded with no working folder. Each local
    /// row's bytes are copied into the store under their hash, and the row
    /// points at the copy; the S3 row is kept; the manifest is the pending
    /// commit of a package with no base.
    #[test(tokio::test)]
    async fn a_build_records_its_rows_into_the_store() -> Res {
        let (paths, _domain) = DomainPaths::from_temp_dir()?;
        let elsewhere = tempfile::tempdir()?;
        let storage = LocalStorage::new();
        let (source, source_hash) = file_anywhere(elsewhere.path(), "b.txt", b"from disk").await?;
        let s3_row = ManifestRow {
            physical_key: "s3://bucket/c.txt?versionId=1".to_string(),
            size: 3,
            ..row("c.txt", source_hash.clone())
        };
        let manifest = revision(
            "built",
            vec![
                s3_row.clone(),
                row("b.txt", source_hash.clone()),
                row("a.txt", ObjectHash::default()),
            ],
        );
        let files = BTreeMap::from([
            (
                key("a.txt"),
                RowContent::Bytes(Bytes::from_static(b"in memory")),
            ),
            (key("b.txt"), RowContent::Path(source.clone())),
        ]);

        let (lineage, recorded) = record(
            &paths,
            &storage,
            &MockRemote::default(),
            WorkflowCheck::Warn { host: None },
            &namespace(),
            PackageLineage::default(),
            manifest,
            &files,
        )
        .await?;

        assert_eq!(recorded.warnings, []);
        let hash = recorded.hash().expect("a revision").to_string();
        let commit = lineage.commit.as_ref().expect("a pending commit");
        assert_eq!(commit.hash, hash);
        assert_eq!(commit.prev_hashes, Vec::<String>::new());
        assert_eq!(lineage.base_hash, "");
        assert!(lineage.paths.is_empty(), "a build tracks no working copy");

        let written =
            Manifest::from_path(&storage, &paths.installed_manifest(&namespace(), &hash)).await?;
        let keys: Vec<_> = written
            .rows
            .iter()
            .map(|r| r.logical_key.to_string())
            .collect();
        assert_eq!(keys, ["a.txt", "b.txt", "c.txt"]);
        for (row, bytes) in written.rows[..2]
            .iter()
            .zip([&b"in memory"[..], b"from disk"])
        {
            let object = paths.object_path(&row.hash);
            assert_eq!(row.physical_key, crate::paths::file_url(&object)?);
            assert_eq!(std::fs::read(&object)?, bytes);
            assert_eq!(row.size, bytes.len() as u64);
        }
        assert_ne!(
            written.rows[0].hash,
            ObjectHash::default(),
            "an unhashed row is hashed"
        );
        assert_eq!(written.rows[1].hash, source_hash);
        let kept = &written.rows[2];
        assert_eq!(
            (&kept.physical_key, &kept.hash, kept.size),
            (&s3_row.physical_key, &s3_row.hash, s3_row.size),
            "an S3 row keeps its object"
        );
        assert_eq!(
            std::fs::read(&source)?,
            b"from disk",
            "the source is left alone"
        );
        Ok(())
    }

    /// The store is a parameter: a record into another store writes its
    /// objects and manifest there, and nothing into `$DOMAIN`.
    #[test(tokio::test)]
    async fn a_record_writes_to_the_store_it_is_given() -> Res {
        let (paths, _domain) = DomainPaths::from_temp_dir()?;
        let other = tempfile::tempdir()?;
        let store = OtherStore(other.path().to_path_buf());
        let storage = LocalStorage::new();
        let files = BTreeMap::from([(key("a.txt"), RowContent::Bytes(Bytes::from_static(b"x")))]);

        let (_, recorded) = record(
            &store,
            &storage,
            &MockRemote::default(),
            WorkflowCheck::Warn { host: None },
            &namespace(),
            PackageLineage::default(),
            revision("other", vec![row("a.txt", ObjectHash::default())]),
            &files,
        )
        .await?;

        let hash = recorded.hash().expect("a revision");
        let written =
            Manifest::from_path(&storage, &store.manifests_dir(&namespace()).join(hash)).await?;
        assert!(
            storage
                .is_file(store.object_path(&written.rows[0].hash))
                .await
        );
        assert!(
            !storage
                .exists(paths.installed_manifests_dir(&namespace()))
                .await
        );
        assert!(!storage.exists(DomainPaths::objects_dir(&paths)).await);
        Ok(())
    }

    /// A file whose bytes changed after the caller hashed them fails the
    /// record, rather than storing them under another hash's name: no
    /// manifest is written, the lineage is untouched, and no copy is left.
    #[test(tokio::test)]
    async fn a_file_changed_after_it_was_hashed_records_nothing() -> Res {
        let (paths, _domain) = DomainPaths::from_temp_dir()?;
        let elsewhere = tempfile::tempdir()?;
        let storage = LocalStorage::new();
        let (source, old_hash) = file_anywhere(elsewhere.path(), "a.txt", b"before").await?;
        std::fs::write(&source, b"after")?;

        let err = record(
            &paths,
            &storage,
            &MockRemote::default(),
            WorkflowCheck::Warn { host: None },
            &namespace(),
            PackageLineage::default(),
            revision("changed", vec![row("a.txt", old_hash)]),
            &BTreeMap::from([(key("a.txt"), RowContent::Path(source))]),
        )
        .await
        .expect_err("the bytes no longer match the row's hash");

        assert!(
            err.to_string().contains("changed after it was hashed"),
            "{err}"
        );
        assert_eq!(
            manifest_names(&paths.installed_manifests_dir(&namespace())),
            Vec::<String>::new()
        );
        assert_eq!(
            manifest_names(&DomainPaths::objects_dir(&paths)),
            Vec::<String>::new()
        );
        Ok(())
    }

    /// Content for a key the manifest has no row for is refused before
    /// anything is copied.
    #[test(tokio::test)]
    async fn content_for_a_key_with_no_row_is_refused() -> Res {
        let (paths, _domain) = DomainPaths::from_temp_dir()?;
        let result = record(
            &paths,
            &LocalStorage::new(),
            &MockRemote::default(),
            WorkflowCheck::Warn { host: None },
            &namespace(),
            PackageLineage::default(),
            revision("stray", Vec::new()),
            &BTreeMap::from([(key("stray"), RowContent::Bytes(Bytes::from_static(b"x")))]),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(
            manifest_names(&DomainPaths::objects_dir(&paths)),
            Vec::<String>::new()
        );
        Ok(())
    }

    /// A second record goes on top of the pending commit, which it lists as
    /// its previous revision; recording the head again records nothing.
    #[test(tokio::test)]
    async fn records_stack_on_the_pending_commit() -> Res {
        let (paths, _domain) = DomainPaths::from_temp_dir()?;
        let storage = LocalStorage::new();
        let remote = MockRemote::default();
        let build = |message: &'static str, lineage| {
            let (paths, storage, remote) = (&paths, &storage, &remote);
            async move {
                record(
                    paths,
                    storage,
                    remote,
                    WorkflowCheck::Warn { host: None },
                    &namespace(),
                    lineage,
                    revision(message, Vec::new()),
                    &BTreeMap::new(),
                )
                .await
            }
        };

        let (lineage, first) = build("A", PackageLineage::default()).await?;
        let first = first.hash().expect("A").to_string();
        let (lineage, second) = build("B", lineage).await?;
        let second = second.hash().expect("B").to_string();
        let commit = lineage.commit.clone().expect("B is pending");
        assert_eq!(commit.hash, second);
        assert_eq!(commit.prev_hashes, [first]);

        let (again, nothing) = build("B", lineage.clone()).await?;
        assert_eq!(nothing.committed, Committed::NothingToCommit);
        assert_eq!(again, lineage);
        Ok(())
    }

    /// A bucket whose workflows config requires a workflow, and a lineage
    /// with that bucket as its remote, or none.
    async fn governed(remote_bucket: bool) -> Res<(MockRemote, PackageLineage)> {
        let remote = MockRemote::default();
        let config: S3Uri = "s3://governed/.quilt/workflows/config.yml".parse()?;
        remote
            .put_object(
                None,
                &config,
                b"version: \"1\"\nis_workflow_required: true\nworkflows:\n  gate:\n    name: Gate\n"
                    .to_vec(),
            )
            .await?;
        let lineage = PackageLineage {
            remote_uri: remote_bucket.then(|| ManifestUri {
                origin: None,
                bucket: "governed".to_string(),
                namespace: namespace(),
                hash: String::new(),
            }),
            ..PackageLineage::default()
        };
        Ok((remote, lineage))
    }

    async fn record_unworkflowed(
        paths: &DomainPaths,
        remote: &MockRemote,
        workflows: WorkflowCheck<'_>,
        lineage: PackageLineage,
    ) -> Res<(PackageLineage, Recorded)> {
        record(
            paths,
            &LocalStorage::new(),
            remote,
            workflows,
            &namespace(),
            lineage,
            revision("no workflow", Vec::new()),
            &BTreeMap::new(),
        )
        .await
    }

    /// A local commit or build of a package with a remote is checked against
    /// the remote bucket's current workflows, and an inconsistent revision is
    /// recorded with a warning rather than refused.
    #[test(tokio::test)]
    async fn an_inconsistent_revision_is_recorded_with_a_warning() -> Res {
        let (paths, _domain) = DomainPaths::from_temp_dir()?;
        let (remote, lineage) = governed(true).await?;
        let (lineage, recorded) =
            record_unworkflowed(&paths, &remote, WorkflowCheck::Warn { host: None }, lineage)
                .await?;
        assert!(lineage.commit.is_some());
        match recorded.warnings.as_slice() {
            [WorkflowWarning::Inconsistent { bucket, reason }] => {
                assert_eq!(bucket, "governed");
                assert!(reason.contains("workflow is required"), "{reason}");
            }
            other => panic!("expected one inconsistency, got {other:?}"),
        }
        Ok(())
    }

    /// A local-only package gets no check and no warning, even where a
    /// bucket it is not tied to would refuse it.
    #[test(tokio::test)]
    async fn a_local_only_package_gets_no_workflow_check() -> Res {
        let (paths, _domain) = DomainPaths::from_temp_dir()?;
        let (remote, lineage) = governed(false).await?;
        let (_, recorded) =
            record_unworkflowed(&paths, &remote, WorkflowCheck::Warn { host: None }, lineage)
                .await?;
        assert!(recorded.hash().is_some());
        assert_eq!(recorded.warnings, []);
        Ok(())
    }

    /// A config that cannot be read does not block the record either: the
    /// warning says the revision was not checked.
    #[test(tokio::test)]
    async fn an_unreadable_config_is_a_warning_too() -> Res {
        let (paths, _domain) = DomainPaths::from_temp_dir()?;
        let (remote, lineage) = governed(true).await?;
        remote
            .put_object(
                None,
                &"s3://governed/.quilt/workflows/config.yml".parse()?,
                b"version: [not, a, config".to_vec(),
            )
            .await?;
        let (_, recorded) =
            record_unworkflowed(&paths, &remote, WorkflowCheck::Warn { host: None }, lineage)
                .await?;
        assert!(recorded.hash().is_some());
        assert!(
            matches!(
                recorded.warnings.as_slice(),
                [WorkflowWarning::Unchecked { bucket, .. }] if bucket == "governed"
            ),
            "{:?}",
            recorded.warnings
        );
        Ok(())
    }

    /// The gate a publish uses refuses what its header's workflow rejects,
    /// and writes no manifest. Here the header names a workflow whose rules
    /// require a message the revision lacks.
    #[test(tokio::test)]
    async fn the_gate_refuses_and_records_nothing() -> Res {
        let (paths, _domain) = DomainPaths::from_temp_dir()?;
        let remote = MockRemote::default();
        let config: S3Uri = "s3://governed/.quilt/workflows/config.yml".parse()?;
        remote
            .put_object(
                None,
                &config,
                b"version: \"1\"\nworkflows:\n  gate:\n    name: Gate\n    is_message_required: true\n"
                    .to_vec(),
            )
            .await?;
        let mut manifest = revision("", Vec::new());
        manifest.header.message = crate::manifest::HeaderMessage::default();
        manifest.header.workflow = Some(crate::manifest::Workflow {
            config,
            id: Some(crate::manifest::WorkflowId {
                id: "gate".to_string(),
                schemas: BTreeMap::new(),
            }),
        });
        let result = record(
            &paths,
            &LocalStorage::new(),
            &remote,
            WorkflowCheck::Gate { host: None },
            &namespace(),
            PackageLineage::default(),
            manifest,
            &BTreeMap::new(),
        )
        .await;
        assert!(
            matches!(result, Err(Error::WorkflowValidation(_))),
            "{result:?}"
        );
        assert_eq!(
            manifest_names(&paths.installed_manifests_dir(&namespace())),
            Vec::<String>::new()
        );
        Ok(())
    }
}
