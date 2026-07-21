use tokio_stream::StreamExt;
use tracing::debug;
use tracing::info;
use tracing::warn;

use crate::Error;
use crate::Res;
use crate::error::PackageOpError;
use crate::flow;
use crate::io::manifest::RowsStream;
use crate::io::manifest::StreamItem;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::manifest::resolve_tag;
use crate::io::manifest::tag_timestamp;
use crate::io::manifest::upload_manifest;
use crate::io::manifest::upload_row;
use crate::io::remote::HostConfig;
use crate::io::remote::Remote;
use crate::io::remote::entry_view;
use crate::io::remote::validate_workflow_against_current_config;
use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::paths;
use crate::workflow::EntryView;
use quilt_uri::ManifestUri;
use quilt_uri::Namespace;
use quilt_uri::S3PackageHandle;
use quilt_uri::Tag;

async fn use_existing_row_or_upload(
    remote: &impl Remote,
    host_config: &HostConfig,
    package_handle: &S3PackageHandle,
    remote_manifest: &Manifest,
    rows: StreamItem,
) -> StreamItem {
    let mut output = Vec::new();
    for row in rows? {
        let row = row?;
        debug!("⏳ Processing row: {}", row.logical_key.display());
        if let Some(remote_row) = remote_manifest.get_record(&row.logical_key) {
            if remote_row.matches_content(&row) {
                debug!(
                    "✔️ Using existing remote row for: {}",
                    row.logical_key.display()
                );
                let updated_manifest_row = ManifestRow {
                    physical_key: remote_row.physical_key.clone(),
                    ..row.clone()
                };
                output.push(Ok(updated_manifest_row));
            } else {
                debug!(
                    "⏳ Uploading modified row for: {}",
                    row.logical_key.display()
                );
                let uploaded_row =
                    upload_row(remote, host_config, package_handle.clone(), row).await?;
                output.push(Ok(uploaded_row));
            }
        } else {
            debug!("⏳ Uploading new row for: {}", row.logical_key.display());
            let uploaded_row = upload_row(remote, host_config, package_handle.clone(), row).await?;
            output.push(Ok(uploaded_row));
        }
    }
    Ok(output)
}

async fn stream_uploaded_local_rows<'a>(
    remote: &'a impl Remote,
    host_config: &'a HostConfig,
    local_manifest: &'a Manifest,
    remote_manifest: &'a Manifest,
    package_handle: &'a S3PackageHandle,
) -> impl RowsStream + 'a {
    let stream = local_manifest.records_stream().await;
    stream.then(move |rows| {
        use_existing_row_or_upload(remote, host_config, package_handle, remote_manifest, rows)
    })
}

/// Result of a successful push operation.
#[derive(Debug)]
pub struct PushResult {
    pub lineage: PackageLineage,
    /// Whether the pushed revision was certified as "latest".
    /// `false` when the remote's latest tag has moved (someone else pushed)
    /// since we last checked.
    pub certified_latest: bool,
}

/// Push the new package revision to the remote and tags it as "latest".
///
/// Runs the push-side workflow gate against the destination bucket's current
/// config (see `push_package_impl`). Used for a standalone push of a
/// pre-existing commit; the publish flow calls `push_package_impl` directly
/// so it can skip the gate when the commit it just made already validated the
/// identical manifest.
pub async fn push_package(
    lineage: PackageLineage,
    local_manifest: Manifest,
    paths: &paths::DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    namespace: Option<Namespace>,
    host_config: HostConfig,
) -> Res<PushResult> {
    push_package_impl(
        lineage,
        local_manifest,
        paths,
        storage,
        remote,
        namespace,
        host_config,
        true,
    )
    .await
}

/// Push the new package revision to the remote and tag it as "latest".
///
/// `enforce_workflow` runs the push-side workflow gate against the
/// destination bucket's **current** config. The publish flow passes `false`
/// on the commit-then-push path: the commit gate has already validated the
/// byte-identical manifest against the freshly-resolved (hence current)
/// workflow in the same operation, so a second fetch + validate is pure
/// redundancy. Standalone push (and the publish push-only branch, where no
/// commit ran this operation) passes `true`.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn push_package_impl(
    mut lineage: PackageLineage,
    local_manifest: Manifest,
    paths: &paths::DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    namespace: Option<Namespace>,
    host_config: HostConfig,
    enforce_workflow: bool,
) -> Res<PushResult> {
    // NB: `.take()` moves commit out of lineage before we validate remote.
    // Safe because the caller reads lineage from disk and discards this copy on error.
    // If reusing this function elsewhere, ensure the commit isn't lost on early return.
    let Some(commit) = lineage.commit.take() else {
        info!("No changes to push");
        return Ok(PushResult {
            lineage,
            certified_latest: true,
        });
    };

    let remote_uri = lineage.remote()?.clone();
    let is_first_push = lineage.base_hash.is_empty();

    debug!("⏳ Fetching remote manifest");
    let remote_manifest = if remote_uri.hash.is_empty() {
        debug!("✔️ First push — no remote manifest, using empty default");
        Manifest::default()
    } else {
        flow::browse(paths, storage, remote, &remote_uri).await?
    };
    debug!("✔️ Remote manifest ready");

    // ## copy data
    // Copy each of the _modified_ paths from their local_key to remote_key,
    // keeping track of the resulting versionIds
    //
    // TODO: FAIL if the remote bucket does NOT support versioning (as it would be destructive)

    let namespace = namespace.unwrap_or(remote_uri.namespace.clone());

    debug!("⏳ Creating manifest URI");
    let manifest_uri = ManifestUri {
        namespace,
        ..remote_uri.clone()
    };
    debug!("✔️ Created manifest URI: {}", manifest_uri.display());

    // The workflow quality gate, re-run at push against the destination
    // bucket's *current* config: a revision committed by an older or
    // different client may not satisfy the bucket's workflow as it stands
    // now, and the header's version-pinned config URI may be stale or
    // deleted, so we re-load `.quilt/workflows/config.yml` from the bucket
    // rather than trusting the header. Runs after the no-commit early return
    // above, so a no-op push stays a no-op. Skipped only when the caller
    // (publish) just validated the byte-identical manifest at commit.
    //
    // Validate the entry order that `records_stream` will serialize and
    // upload, not `local_manifest.rows`' storage order. `records_stream`
    // yields rows sorted by logical key, while `.rows` preserves
    // insertion/file order, so a manifest installed from another client can be
    // unsorted. An order-sensitive entries_schema (a Draft-7 tuple-form
    // `items`) must see the bytes we upload — and agree with the commit gate,
    // which validates its own sorted view. This sort mirrors
    // `Manifest::records_stream`'s ordering.
    if enforce_workflow {
        let mut sorted_rows: Vec<&ManifestRow> = local_manifest.rows.iter().collect();
        sorted_rows.sort_by(|a, b| a.logical_key.cmp(&b.logical_key));
        let entries: Vec<EntryView> = sorted_rows.iter().copied().map(entry_view).collect();
        validate_workflow_against_current_config(
            remote,
            &host_config.host,
            &manifest_uri.bucket,
            &manifest_uri.namespace.to_string(),
            local_manifest.header.message.as_deref(),
            local_manifest.header.user_meta.as_ref(),
            local_manifest.header.workflow.as_ref(),
            &entries,
        )
        .await?;
    }

    debug!("⏳ Building and uploading manifest");
    let package_handle = S3PackageHandle::from(&manifest_uri);
    let stream = Box::pin(
        stream_uploaded_local_rows(
            remote,
            &host_config,
            &local_manifest,
            &remote_manifest,
            &package_handle,
        )
        .await,
    );
    let dest_dir = paths.cached_manifests_dir(&manifest_uri.bucket);
    let (cache_path, top_hash) =
        build_manifest_from_rows_stream(storage, dest_dir, local_manifest.header.clone(), stream)
            .await?;
    debug!(
        "✔️ Built manifest with hash {} at {}",
        top_hash,
        cache_path.display()
    );

    let new_manifest_uri = ManifestUri {
        hash: top_hash,
        ..remote_uri.clone()
    };

    debug!(
        "⏳ Uploading manifest to remote {}",
        new_manifest_uri.display()
    );
    upload_manifest(storage, remote, &new_manifest_uri, &cache_path).await?;
    debug!("✔️ Manifest uploaded");

    debug!("⏳ Adding timestamp tag {}", commit.timestamp);
    tag_timestamp(remote, &new_manifest_uri, commit.timestamp).await?;
    debug!("✔️ Timestamp tag added");

    debug!("⏳ Checking remote's latest manifest hash");
    lineage.latest_hash =
        match resolve_tag(remote, &new_manifest_uri.origin, manifest_uri, Tag::Latest).await {
            Ok(uri) => uri.hash,
            Err(e) if e.is_not_found() => {
                debug!("✔️ No existing latest tag — first push for this package");
                String::new()
            }
            Err(e) => return Err(e),
        };
    debug!("✔️ Latest hash is: {}", lineage.latest_hash);

    lineage.remote_uri = Some(new_manifest_uri.clone());

    // Update base_hash after a successful push. Only needed for first push
    // where base_hash is "" — without this, the package would appear as Diverged
    // instead of UpToDate after push.
    // On subsequent pushes, certify_latest() below handles the update via
    // update_latest(), which sets both base_hash and latest_hash.
    if lineage.base_hash.is_empty() {
        lineage.base_hash = new_manifest_uri.hash.clone();
    }

    if new_manifest_uri.hash != commit.hash {
        debug!("❌ Hash mismatch, copying cached to installed");
        // Otherwise, lineage will be pointing to the wrong/inexisting hash
        paths::copy_cached_to_installed(paths, storage, &new_manifest_uri).await?;
        Err(Error::PackageOp(PackageOpError::Push(
            "Latest local hash is not equal to pushed manifest commit".to_string(),
        )))?;
    }

    // Certify latest when:
    // - first push (base_hash was empty): always certify, even if the remote already
    //   has a different "latest" — the user explicitly pushed this version;
    // - tracking (base_hash == latest_hash): we're up-to-date with remote;
    // - no existing latest: remote has never had a "latest" tag.
    if is_first_push || lineage.base_hash == lineage.latest_hash || lineage.latest_hash.is_empty() {
        debug!("⏳ Certifying new latest (first push, tracking, or no existing latest)");
        let lineage = flow::certify_latest(lineage, remote, new_manifest_uri).await?;
        return Ok(PushResult {
            lineage,
            certified_latest: true,
        });
    }

    warn!("Pushed but did not certify latest: remote has newer changes");
    info!("✔️ Successfully pushed package (without certifying latest)");
    Ok(PushResult {
        lineage,
        certified_latest: false,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use std::path::PathBuf;

    use aws_sdk_s3::primitives::ByteStream;

    use crate::fixtures;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::CommitState;
    use crate::lineage::PackageLineage;
    use crate::manifest::ManifestHeader;
    use crate::manifest::Workflow;
    use crate::manifest::WorkflowId;
    use crate::workflow::RuleViolation;
    use crate::workflow::WorkflowValidationError;
    use quilt_uri::S3Uri;
    use serde_json::Value;
    use serde_json::json;
    use std::collections::BTreeMap;

    #[test(tokio::test)]
    async fn test_no_push_if_no_commit() -> Res {
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        let result = push_package(
            PackageLineage::default(),
            Manifest::default(),
            &paths::DomainPaths::default(),
            &storage,
            &remote,
            None,
            HostConfig::default(),
        )
        .await?;
        assert_eq!(result.lineage, PackageLineage::default());
        assert!(result.certified_latest);
        Ok(())
    }

    /// A manifest header carrying a workflow whose config requires an `owner`
    /// in the package metadata. Shared by the two push-gate tests; the caller
    /// seeds the matching config into the remote.
    fn governed_gate_workflow() -> Workflow {
        Workflow {
            config: "s3://b/.quilt/workflows/config.yml"
                .parse()
                .expect("valid config uri"),
            id: Some(WorkflowId {
                id: "gate".to_string(),
                schemas: BTreeMap::new(),
            }),
        }
    }

    fn first_push_governed_lineage() -> PackageLineage {
        PackageLineage {
            commit: Some(CommitState {
                timestamp: chrono::Utc::now(),
                // Deliberately not the built hash: the valid-path test relies on
                // the push reaching the post-upload hash check, which proves the
                // gate let it through.
                hash: "deadbeef".to_string(),
                prev_hashes: Vec::new(),
            }),
            remote_uri: Some(ManifestUri {
                bucket: "b".to_string(),
                namespace: ("foo", "bar").into(),
                hash: String::new(),
                origin: None,
            }),
            ..PackageLineage::default()
        }
    }

    fn governed_manifest() -> Manifest {
        Manifest {
            header: ManifestHeader {
                message: Some("msg".to_string()),
                user_meta: None,
                workflow: Some(governed_gate_workflow()),
                ..ManifestHeader::default()
            },
            ..Manifest::default()
        }
    }

    /// The push-side workflow gate: a revision whose workflow requires `owner`
    /// (but whose metadata lacks it) is rejected before any bytes reach the
    /// remote.
    #[test(tokio::test)]
    async fn test_push_rejected_by_workflow_uploads_nothing() -> Res {
        let paths = paths::DomainPaths::new(PathBuf::from("/foo"));
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/workflows/config.yml")?,
                b"version: \"1\"\nworkflows:\n  gate:\n    name: Gate\n    metadata_schema: meta\nschemas:\n  meta:\n    url: s3://b/schemas/meta.json\n".to_vec(),
            )
            .await?;
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/schemas/meta.json")?,
                br#"{"type": "object", "required": ["owner"]}"#.to_vec(),
            )
            .await?;

        let err = push_package(
            first_push_governed_lineage(),
            governed_manifest(),
            &paths,
            &storage,
            &remote,
            None,
            HostConfig::default(),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(
                err,
                Error::WorkflowValidation(WorkflowValidationError::Rejected(_))
            ),
            "expected a workflow rejection, got: {err:?}"
        );
        assert!(
            !storage.exists(&paths.cached_manifests_dir("b")).await,
            "a rejected push must not build or upload a manifest"
        );
        Ok(())
    }

    /// A governed manifest that *satisfies* its workflow passes the gate: the
    /// push proceeds to build and upload the manifest, reaching the post-upload
    /// hash check (which trips only because this fixture's commit hash is a
    /// placeholder). The point is that the gate did not reject it.
    #[test(tokio::test)]
    async fn test_push_governed_but_valid_passes_the_gate() -> Res {
        let paths = paths::DomainPaths::new(PathBuf::from("/foo"));
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        // A permissive `gate`: no schema, no required message — the empty
        // manifest satisfies it.
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/workflows/config.yml")?,
                b"version: \"1\"\nworkflows:\n  gate:\n    name: Gate\n".to_vec(),
            )
            .await?;

        let err = push_package(
            first_push_governed_lineage(),
            governed_manifest(),
            &paths,
            &storage,
            &remote,
            None,
            HostConfig::default(),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(
                &err,
                Error::PackageOp(PackageOpError::Push(msg))
                    if msg.contains("not equal to pushed manifest commit")
            ),
            "expected the gate to pass and the push to reach the hash check, got: {err:?}"
        );
        assert!(
            storage.exists(&paths.cached_manifests_dir("b")).await,
            "a passing gate must let the push build the manifest"
        );
        Ok(())
    }

    /// Seed the mock remote with a `gate` workflow whose `entries_schema`
    /// requires each entry's metadata to carry an `approved` key. Shared by
    /// the two wrapped-meta gate tests below.
    async fn seed_entries_schema_gate(remote: &MockRemote) -> Res {
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/workflows/config.yml")?,
                b"version: \"1\"\nworkflows:\n  gate:\n    name: Gate\n    entries_schema: entries\nschemas:\n  entries:\n    url: s3://b/schemas/entries.json\n".to_vec(),
            )
            .await?;
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/schemas/entries.json")?,
                br#"{"type": "array", "items": {"type": "object", "properties": {"meta": {"type": "object", "required": ["approved"]}}}}"#.to_vec(),
            )
            .await?;
        Ok(())
    }

    /// Seed a `gate` workflow whose `entries_schema` is an order-sensitive
    /// Draft-7 tuple-form `items`: entry 0 must be `a.txt`, entry 1 `b.txt`.
    /// It validates only when the entries are inspected in sorted (`a`, `b`)
    /// order — the order `records_stream` serializes and uploads.
    async fn seed_ordered_entries_schema_gate(remote: &MockRemote) -> Res {
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/workflows/config.yml")?,
                b"version: \"1\"\nworkflows:\n  gate:\n    name: Gate\n    entries_schema: entries\nschemas:\n  entries:\n    url: s3://b/schemas/entries.json\n".to_vec(),
            )
            .await?;
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/schemas/entries.json")?,
                br#"{"type": "array", "items": [{"properties": {"logical_key": {"const": "a.txt"}}}, {"properties": {"logical_key": {"const": "b.txt"}}}]}"#.to_vec(),
            )
            .await?;
        Ok(())
    }

    /// A row whose wire `meta` carries the wrapped `{"user_meta": {...}}`
    /// form, as manifest rows do on the wire.
    fn wrapped_meta_row(user_meta: &Value) -> ManifestRow {
        ManifestRow {
            logical_key: PathBuf::from("a.txt"),
            physical_key: "file:///b/a/r0".to_string(),
            meta: Some(json!({ "user_meta": user_meta })),
            ..ManifestRow::default()
        }
    }

    /// The entries gate validates each entry's *unwrapped* user metadata,
    /// matching quilt3 (`PackageEntry.meta` returns
    /// `self._meta.get('user_meta', {})`): a row whose wrapped metadata lacks
    /// the required `approved` key is rejected before any bytes reach the
    /// remote.
    #[test(tokio::test)]
    async fn test_push_entries_schema_rejects_wrapped_meta_violating_unwrapped() -> Res {
        let paths = paths::DomainPaths::new(PathBuf::from("/foo"));
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        seed_entries_schema_gate(&remote).await?;

        let mut manifest = governed_manifest();
        manifest
            .insert_record(wrapped_meta_row(&json!({ "note": "x" })))
            .await?;

        let err = push_package(
            first_push_governed_lineage(),
            manifest,
            &paths,
            &storage,
            &remote,
            None,
            HostConfig::default(),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(
                &err,
                Error::WorkflowValidation(WorkflowValidationError::Rejected(violations))
                    if matches!(&violations[..], [RuleViolation::EntriesInvalid(_)])
            ),
            "expected an entries_schema rejection, got: {err:?}"
        );
        assert!(
            !storage.exists(&paths.cached_manifests_dir("b")).await,
            "a rejected push must not build or upload a manifest"
        );
        Ok(())
    }

    /// The happy-path companion: the same wrapped shape whose *unwrapped*
    /// content satisfies the schema passes the gate. Had the gate validated
    /// the wrapped wire value instead, this entry would be rejected (the
    /// wrapper object's only key is `user_meta`, not `approved`) — so this
    /// test pins the unwrapping.
    #[test(tokio::test)]
    async fn test_push_entries_schema_passes_wrapped_meta_satisfying_unwrapped() -> Res {
        let paths = paths::DomainPaths::new(PathBuf::from("/foo"));
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        seed_entries_schema_gate(&remote).await?;

        let file_content = b"approved bytes\n";
        remote
            .storage
            .write_byte_stream(
                &PathBuf::from("/b/a/r0"),
                ByteStream::from_static(file_content),
            )
            .await?;

        let mut manifest = governed_manifest();
        manifest
            .insert_record(ManifestRow {
                size: file_content.len() as u64,
                ..wrapped_meta_row(&json!({ "approved": true }))
            })
            .await?;

        // The fixture's commit hash is a placeholder, so a push that clears
        // the gate proceeds all the way to the post-upload hash check — which
        // proves the gate let it through.
        let err = push_package(
            first_push_governed_lineage(),
            manifest,
            &paths,
            &storage,
            &remote,
            None,
            HostConfig::default(),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(
                &err,
                Error::PackageOp(PackageOpError::Push(msg))
                    if msg.contains("not equal to pushed manifest commit")
            ),
            "expected the gate to pass and the push to reach the hash check, got: {err:?}"
        );
        assert!(
            storage.exists(&paths.cached_manifests_dir("b")).await,
            "a passing gate must let the push build the manifest"
        );
        Ok(())
    }

    /// The push gate must validate the entry order it *uploads*, not
    /// `Manifest::rows`' storage order. `records_stream` serializes rows
    /// sorted by logical key, but `.rows` preserves insertion/file order, so a
    /// manifest installed from another client can be unsorted. With an
    /// order-sensitive tuple-form `entries_schema`, validating storage order
    /// would reject a package whose uploaded (sorted) order is valid — and
    /// disagree with the commit gate. Here the rows are inserted unsorted
    /// (`b.txt` then `a.txt`) and the schema passes only in sorted order.
    #[test(tokio::test)]
    async fn test_push_entries_schema_validates_uploaded_sorted_order() -> Res {
        let paths = paths::DomainPaths::new(PathBuf::from("/foo"));
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        seed_ordered_entries_schema_gate(&remote).await?;

        let file_content = b"bytes\n";
        for name in ["ra", "rb"] {
            remote
                .storage
                .write_byte_stream(
                    &PathBuf::from(format!("/b/a/{name}")),
                    ByteStream::from_static(file_content),
                )
                .await?;
        }

        // Insert in unsorted storage order: b.txt then a.txt. `records_stream`
        // uploads them sorted (a.txt, b.txt), which is what the gate must see.
        let mut manifest = governed_manifest();
        manifest
            .insert_record(ManifestRow {
                logical_key: PathBuf::from("b.txt"),
                physical_key: "file:///b/a/rb".to_string(),
                size: file_content.len() as u64,
                ..ManifestRow::default()
            })
            .await?;
        manifest
            .insert_record(ManifestRow {
                logical_key: PathBuf::from("a.txt"),
                physical_key: "file:///b/a/ra".to_string(),
                size: file_content.len() as u64,
                ..ManifestRow::default()
            })
            .await?;

        // The fixture's commit hash is a placeholder, so a push that clears the
        // gate proceeds all the way to the post-upload hash check — proving the
        // gate accepted the sorted order it uploads. Before the fix the gate
        // validated storage order (b, a) and rejected with EntriesInvalid.
        let err = push_package(
            first_push_governed_lineage(),
            manifest,
            &paths,
            &storage,
            &remote,
            None,
            HostConfig::default(),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(
                &err,
                Error::PackageOp(PackageOpError::Push(msg))
                    if msg.contains("not equal to pushed manifest commit")
            ),
            "expected the gate to accept the sorted upload order and reach the hash check, got: {err:?}"
        );
        assert!(
            storage.exists(&paths.cached_manifests_dir("b")).await,
            "a passing gate must let the push build the manifest"
        );
        Ok(())
    }

    /// A manifest whose header carries a message but **no** workflow record —
    /// the shape an ungoverned commit (or a commit made against a bucket that
    /// had no config at the time) produces.
    fn header_none_manifest() -> Manifest {
        Manifest {
            header: ManifestHeader {
                message: Some("msg".to_string()),
                user_meta: None,
                workflow: None,
                ..ManifestHeader::default()
            },
            ..Manifest::default()
        }
    }

    /// (a) A header carrying **no** workflow, pushed to a bucket whose current
    /// config requires one, is rejected before any bytes reach the remote —
    /// the destination's `is_workflow_required` is enforced against the live
    /// config, not bypassed by the header's self-declared (absent) workflow.
    #[test(tokio::test)]
    async fn test_push_header_none_to_required_bucket_is_rejected() -> Res {
        let paths = paths::DomainPaths::new(PathBuf::from("/foo"));
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        // `is_workflow_required` defaults to true; a `gate` workflow is
        // declared but the header selected none.
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/workflows/config.yml")?,
                b"version: \"1\"\nworkflows:\n  gate:\n    name: Gate\n".to_vec(),
            )
            .await?;

        let err = push_package(
            first_push_governed_lineage(),
            header_none_manifest(),
            &paths,
            &storage,
            &remote,
            None,
            HostConfig::default(),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(
                &err,
                Error::WorkflowValidation(WorkflowValidationError::Rejected(violations))
                    if matches!(&violations[..], [RuleViolation::WorkflowRequired])
            ),
            "expected a WorkflowRequired rejection, got: {err:?}"
        );
        assert!(
            !storage.exists(&paths.cached_manifests_dir("b")).await,
            "a rejected push must not build or upload a manifest"
        );
        Ok(())
    }

    /// (b) The push gate consults the destination bucket's **current** config,
    /// not the header's version-pinned `workflow.config`. Here the header's
    /// pinned config (a stale object) still declares `gate`, but the current
    /// `.quilt/workflows/config.yml` does not — so the stamped id is unknown
    /// and push hard-errors, uploading nothing.
    #[test(tokio::test)]
    async fn test_push_stamped_id_missing_from_current_config_errors() -> Res {
        let paths = paths::DomainPaths::new(PathBuf::from("/foo"));
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        // The header's pinned config still knows `gate` (would vacuously pass
        // the old header-trust gate)...
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/stale-config.yml")?,
                b"version: \"1\"\nworkflows:\n  gate:\n    name: Gate\n".to_vec(),
            )
            .await?;
        // ...but the bucket's *current* config no longer declares it.
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/workflows/config.yml")?,
                b"version: \"1\"\nis_workflow_required: false\nworkflows:\n  other:\n    name: Other\n".to_vec(),
            )
            .await?;

        let mut manifest = governed_manifest();
        manifest.header.workflow = Some(Workflow {
            config: "s3://b/.quilt/stale-config.yml"
                .parse()
                .expect("valid config uri"),
            id: Some(WorkflowId {
                id: "gate".to_string(),
                schemas: BTreeMap::new(),
            }),
        });

        let err = push_package(
            first_push_governed_lineage(),
            manifest,
            &paths,
            &storage,
            &remote,
            None,
            HostConfig::default(),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(&err, Error::RemoteCatalog(_)),
            "expected a RemoteCatalog error naming the missing workflow, got: {err:?}"
        );
        assert!(
            err.to_string().contains("gate"),
            "error should name the missing workflow id, got: {err}"
        );
        assert!(
            !storage.exists(&paths.cached_manifests_dir("b")).await,
            "a rejected push must not build or upload a manifest"
        );
        Ok(())
    }

    /// (c) The push gate re-validates against the bucket's config **as it is
    /// now**: the same manifest that passes under a permissive config is
    /// rejected once the config is tightened, with no other change.
    #[test(tokio::test)]
    async fn test_push_revalidates_against_mutated_current_config() -> Res {
        let paths = paths::DomainPaths::new(PathBuf::from("/foo"));
        let config_uri = S3Uri::try_from("s3://b/.quilt/workflows/config.yml")?;

        // Under a permissive config, the governed manifest clears the gate and
        // reaches the post-upload hash check (proving the gate let it through).
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        remote
            .put_object(
                &None,
                &config_uri,
                b"version: \"1\"\nworkflows:\n  gate:\n    name: Gate\n".to_vec(),
            )
            .await?;
        let err = push_package(
            first_push_governed_lineage(),
            governed_manifest(),
            &paths,
            &storage,
            &remote,
            None,
            HostConfig::default(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(
                &err,
                Error::PackageOp(PackageOpError::Push(msg))
                    if msg.contains("not equal to pushed manifest commit")
            ),
            "expected the permissive config to pass the gate, got: {err:?}"
        );

        // Tighten the *current* config to require an `owner` the manifest lacks.
        // The identical manifest is now rejected.
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        remote
            .put_object(
                &None,
                &config_uri,
                b"version: \"1\"\nworkflows:\n  gate:\n    name: Gate\n    metadata_schema: meta\nschemas:\n  meta:\n    url: s3://b/schemas/meta.json\n".to_vec(),
            )
            .await?;
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/schemas/meta.json")?,
                br#"{"type": "object", "required": ["owner"]}"#.to_vec(),
            )
            .await?;
        let err = push_package(
            first_push_governed_lineage(),
            governed_manifest(),
            &paths,
            &storage,
            &remote,
            None,
            HostConfig::default(),
        )
        .await
        .unwrap_err();
        assert!(
            matches!(
                &err,
                Error::WorkflowValidation(WorkflowValidationError::Rejected(_))
            ),
            "expected the tightened current config to reject the same manifest, got: {err:?}"
        );
        assert!(
            !storage.exists(&paths.cached_manifests_dir("b")).await,
            "a rejected push must not build or upload a manifest"
        );
        Ok(())
    }

    /// (e) An ungoverned bucket (no config, header carries no workflow) is
    /// untouched by the gate: the push proceeds to build and upload the
    /// manifest, reaching the post-upload hash check.
    #[test(tokio::test)]
    async fn test_push_ungoverned_bucket_passes_the_gate() -> Res {
        let paths = paths::DomainPaths::new(PathBuf::from("/foo"));
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        // No config seeded for bucket `b`.

        let err = push_package(
            first_push_governed_lineage(),
            header_none_manifest(),
            &paths,
            &storage,
            &remote,
            None,
            HostConfig::default(),
        )
        .await
        .unwrap_err();

        assert!(
            matches!(
                &err,
                Error::PackageOp(PackageOpError::Push(msg))
                    if msg.contains("not equal to pushed manifest commit")
            ),
            "expected the ungoverned push to pass the gate and reach the hash check, got: {err:?}"
        );
        assert!(
            storage.exists(&paths.cached_manifests_dir("b")).await,
            "an ungoverned push must build the manifest"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_no_entries_push() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("a", "c").into(),
            hash: "__FOO__".to_string(),
            origin: None,
        };
        let lineage = PackageLineage {
            commit: Some(CommitState {
                timestamp: chrono::Utc::now(),
                hash: fixtures::top_hash::EMPTY_NULL_TOP_HASH.to_string(),
                prev_hashes: Vec::new(),
            }),
            remote_uri: Some(manifest_uri),
            ..PackageLineage::default()
        };
        let paths = paths::DomainPaths::new(PathBuf::from("/foo"));
        let manifest_key = paths
            .cached_manifests_dir("b")
            .join(fixtures::top_hash::EMPTY_NULL_TOP_HASH);
        let storage = MockStorage::default();
        storage
            .write_byte_stream(manifest_key, ByteStream::from_static(b"foo"))
            .await?;

        let remote = MockRemote::default();
        let dummy_manifest = r#"{"version": "v0"}"#;
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/packages/__FOO__")?,
                dummy_manifest.as_bytes().to_vec(),
            )
            .await?;
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/named_packages/a/c/latest")?,
                b"abcdef".to_vec(),
            )
            .await?;
        let mut manifest = Manifest::default();
        manifest.header.user_meta = Some(serde_json::Value::Null);
        let result = push_package(
            lineage,
            manifest,
            &paths,
            &storage,
            &remote,
            None,
            HostConfig::default(),
        )
        .await?;
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("a", "c").into(),
            hash: fixtures::top_hash::EMPTY_NULL_TOP_HASH.to_string(),
            origin: None,
        };
        // First push with an existing remote "latest": certify_latest is called,
        // so both base_hash and latest_hash point to the pushed hash.
        assert!(result.certified_latest);
        assert_eq!(
            result.lineage,
            PackageLineage {
                remote_uri: Some(manifest_uri.clone()),
                base_hash: manifest_uri.hash.clone(),
                latest_hash: manifest_uri.hash,
                ..PackageLineage::default()
            }
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_push_virtual_manifest() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "hash-we-later-rewrite-with-push".to_string(),
            origin: None,
        };
        let lineage = PackageLineage {
            commit: Some(CommitState {
                timestamp: chrono::Utc::now(),
                hash: fixtures::manifest::TOP_HASH.to_string(),
                prev_hashes: Vec::new(),
            }),
            remote_uri: Some(manifest_uri),
            ..PackageLineage::default()
        };
        let storage = MockStorage::default();
        let remote = MockRemote::default();
        let dummy_manifest = r#"{"version": "v0"}"#;
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/packages/hash-we-later-rewrite-with-push")?,
                dummy_manifest.as_bytes().to_vec(),
            )
            .await?;
        remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/.quilt/named_packages/f/a/latest")?,
                b"latest-hash-abcdef".to_vec(),
            )
            .await?;

        let mut manifest = Manifest::default();
        manifest.header.message = Some("Initial".to_string());
        manifest.header.user_meta = None;

        let file_content = b"Thu Feb 29 19:07:56 PST 2024\n";

        for i in 0..10 {
            let file_path = PathBuf::from(format!("/b/a/r{i}"));
            remote
                .storage
                .write_byte_stream(&file_path, ByteStream::from_static(file_content))
                .await?;

            manifest
                .insert_record(ManifestRow {
                    logical_key: PathBuf::from(format!("e0-{i}.txt")),
                    physical_key: format!("file://{}", file_path.display()),
                    hash: crate::object_hash::Sha256ChunkedHash::try_from(
                        "/UMjH1bsbrMLBKdd9cqGGvtjhWzawhz1BfrxgngUhVI=",
                    )?
                    .into(),
                    size: file_content.len() as u64,
                    meta: Some(serde_json::Value::Null),
                })
                .await?;
        }

        let result = push_package(
            lineage,
            manifest,
            &paths::DomainPaths::default(),
            &storage,
            &remote,
            None,
            HostConfig::default(),
        )
        .await?;
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: fixtures::manifest::TOP_HASH.to_string(),
            origin: None,
        };
        // First push with an existing remote "latest": certify_latest is called,
        // so both base_hash and latest_hash point to the pushed hash.
        assert!(result.certified_latest);
        assert_eq!(
            result.lineage,
            PackageLineage {
                remote_uri: Some(manifest_uri.clone()),
                base_hash: manifest_uri.hash.clone(),
                latest_hash: manifest_uri.hash,
                ..PackageLineage::default()
            }
        );
        Ok(())
    }
}
