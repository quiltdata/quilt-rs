//! The remote I/O seam for workflows: fetch and parse `.quilt/workflows/config.yml`
//! and schema documents, version-resolve declared schema URLs, resolve the
//! workflow to stamp into a manifest header, and run the workflow gate against a
//! candidate revision. The pure config model, gate, and header-stamp types live
//! in the `crate::workflow` module; this module turns config + S3 into the plain
//! inputs that crate consumes.

use std::collections::BTreeMap;

use serde_json::Value;
use serde_yaml::Value as YamlValue;
use tokio::io::AsyncReadExt;

use crate::Error;
use crate::Res;
use crate::error::RemoteCatalogError;
use crate::io::remote::Remote;
use crate::manifest::ManifestRow;
use crate::workflow::EntryView;
use crate::workflow::PackageCandidate;
use crate::workflow::WORKFLOWS_CONFIG_KEY;
use crate::workflow::Workflow;
use crate::workflow::WorkflowId;
use crate::workflow::WorkflowIntent;
use crate::workflow::WorkflowRules;
use crate::workflow::WorkflowsConfig;
use crate::workflow::validate_package;
use quilt_uri::Host;
use quilt_uri::S3Uri;

/// Version-resolve the declared URL of a schema a workflow references against
/// the remote. Pairs the pure [`WorkflowsConfig::declared_schema_url`] lookup
/// with the single remote call it needs; `workflow_id` is used only for error
/// context.
async fn resolve_schema_url<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    config: &WorkflowsConfig,
    workflow_id: &str,
    schema_id: &str,
) -> Res<S3Uri> {
    let declared = config.declared_schema_url(workflow_id, schema_id)?;
    remote.resolve_url(host, &declared).await
}

/// Resolve every schema a workflow declares into the content-addressed
/// `{schema_id -> version-pinned url}` map that becomes the stamp's `schemas`
/// object.
///
/// quilt3 records one entry per schema the workflow references - both
/// `metadata_schema` and `entries_schema`
/// (`WorkflowConfig.get_workflow_validator` populates `loaded_schemas_by_id`
/// from both). We mirror that exactly: resolve each declared key, version-pin
/// its url, and key by schema id. A `BTreeMap` deduplicates when two keys name
/// one schema id (as quilt3's dict does) and keeps the order deterministic. An
/// empty map means the workflow declares no schema - the stamp then omits the
/// `schemas` field.
async fn resolve_declared_schemas<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    config: &WorkflowsConfig,
    workflow_id: &str,
) -> Res<BTreeMap<String, S3Uri>> {
    let mut schemas = BTreeMap::new();
    for key in ["metadata_schema", "entries_schema"] {
        if let Some(schema_id) = config.schema_id(workflow_id, key)? {
            let url = resolve_schema_url(remote, host, config, workflow_id, &schema_id).await?;
            schemas.insert(schema_id, url);
        }
    }
    Ok(schemas)
}

/// Resolve a named workflow id against `config`, attaching every schema the
/// workflow declares (empty map when it declares none). `config_uri` is the
/// (possibly version-pinned) config address the resulting stamp records.
async fn resolve_named<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    config: &WorkflowsConfig,
    config_uri: S3Uri,
    id: String,
) -> Res<Option<Workflow>> {
    let schemas = resolve_declared_schemas(remote, host, config, &id).await?;
    Ok(Some(Workflow {
        config: config_uri,
        id: Some(WorkflowId { id, schemas }),
    }))
}

pub(crate) async fn fetch_workflows_config<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    uri: &S3Uri,
) -> Res<(S3Uri, Option<WorkflowsConfig>)> {
    if !remote.exists(host, uri).await? {
        return Ok((uri.clone(), None));
    }
    match remote.get_object_stream(host, uri).await {
        Ok(stream) => {
            let mut bytes = Vec::new();
            stream
                .body
                .into_async_read()
                .read_to_end(&mut bytes)
                .await?;
            let config = serde_yaml::from_slice::<Option<YamlValue>>(&bytes)?
                .map(|yaml| WorkflowsConfig::from_yaml(&yaml))
                .transpose()?;
            Ok((stream.uri, config))
        }
        Err(err) => Err(err),
    }
}

/// Fetch and parse the `.quilt/workflows/config.yml` for an arbitrary bucket,
/// independent of any package's already-set remote.
///
/// Builds the config address from `bucket` alone, so the pre-set-remote UI can
/// preview a bucket's declared workflows before the choice is committed.
/// Returns `Ok(None)` when the bucket has no config.
pub async fn fetch_workflows_config_for_bucket<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    bucket: &str,
) -> Res<Option<WorkflowsConfig>> {
    let uri = S3Uri {
        key: WORKFLOWS_CONFIG_KEY.to_string(),
        bucket: bucket.to_string(),
        version: None,
    };
    let (_, config) = fetch_workflows_config(remote, host, &uri).await?;
    Ok(config)
}

/// Fetch a schema document from the remote and parse it as JSON.
async fn fetch_schema_doc<R: Remote>(remote: &R, host: &Option<Host>, uri: &S3Uri) -> Res<Value> {
    let stream = remote.get_object_stream(host, uri).await?;
    let mut bytes = Vec::new();
    stream
        .body
        .into_async_read()
        .read_to_end(&mut bytes)
        .await?;
    Ok(serde_json::from_slice(&bytes)?)
}

/// Assemble the pure-validator [`WorkflowRules`] for a named workflow: read the
/// `handle_pattern` / `is_message_required` flags straight from the parsed
/// config (no fetch), and fetch the `metadata_schema` / `entries_schema`
/// documents referenced by the workflow as `serde_json::Value`.
///
/// This is the I/O boundary between the remote layer and the pure gate in
/// `crate::workflow`: it turns config + S3 into the plain inputs
/// [`crate::workflow::validate_package`] consumes.
pub async fn fetch_workflow_rules<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    config: &WorkflowsConfig,
    workflow_id: &str,
) -> Res<WorkflowRules> {
    let metadata_schema =
        fetch_schema_for_key(remote, host, config, workflow_id, "metadata_schema").await?;
    let entries_schema =
        fetch_schema_for_key(remote, host, config, workflow_id, "entries_schema").await?;

    Ok(WorkflowRules {
        handle_pattern: config.handle_pattern(workflow_id),
        is_message_required: config.is_message_required(workflow_id),
        metadata_schema,
        entries_schema,
    })
}

/// Fetch the schema document a workflow declares under `key`, or `None` when
/// the workflow declares no such schema.
async fn fetch_schema_for_key<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    config: &WorkflowsConfig,
    workflow_id: &str,
    key: &str,
) -> Res<Option<Value>> {
    match config.schema_id(workflow_id, key)? {
        Some(schema_id) => {
            let uri = resolve_schema_url(remote, host, config, workflow_id, &schema_id).await?;
            Ok(Some(fetch_schema_doc(remote, host, &uri).await?))
        }
        None => Ok(None),
    }
}

/// Project a manifest row to the [`EntryView`] the workflow gate inspects.
///
/// `meta` is the row's **unwrapped** user metadata — the value under the row's
/// `user_meta` key — matching quilt3, whose entry projection uses
/// `PackageEntry.meta` (`self._meta.get('user_meta', {})`). The row's own
/// `meta` wire value is the wrapped form `{"user_meta": {...}}`, so we peel one
/// level here; an absent `user_meta` maps to `None` and the `{}`-default is
/// applied by `project_entries` in the pure gate (matching quilt3's default).
///
/// Shared by the commit and push flows so both project rows identically.
///
/// TODO: the gates materialize every manifest row and project all entries into
/// one JSON array for `entries_schema` validation, so memory scales with
/// manifest size. When large manifests arrive (streamed formats such as
/// Parquet/Iceberg over [`crate::io::manifest::RowsStream`]), validate lazily
/// instead: classify the
/// schema up front and stream-validate the dominant subset (a single per-item
/// `items` schema, count/tuple/`contains` accumulators), materializing only
/// for array-level combinators that genuinely need the whole entry set.
pub(crate) fn entry_view(row: &ManifestRow) -> EntryView<'_> {
    EntryView {
        // Lossy: a non-UTF-8 logical key projects with U+FFFD replacement
        // characters, so the entries_schema validates an approximation of the
        // real key instead of a fabricated empty string. A valid key borrows.
        logical_key: row.logical_key.to_string_lossy(),
        size: row.size,
        meta: row.meta.as_ref().and_then(|meta| meta.get("user_meta")),
    }
}

/// Run the workflow quality gate against a candidate revision, using the
/// workflow already recorded in its manifest header.
///
/// This is the enforcement counterpart to [`resolve_workflow`]: resolution
/// stamps a workflow into the header, enforcement re-reads that workflow's
/// config and schema documents and checks the candidate against them. It is
/// the single I/O + gate seam shared by the commit and push flows.
///
/// A vacuously-valid revision is left untouched:
///
/// - a header with no workflow (`workflow` is `None`) — an ungoverned bucket;
/// - a header whose workflow's config has since disappeared from the bucket.
///
/// Otherwise the workflow's rules are fetched (schema documents included) and
/// [`validate_package`] decides. A rule failure surfaces as
/// [`crate::Error::WorkflowValidation`] — a distinct typed error the sync
/// watcher classifies as a conflict (pause the namespace), not a transient
/// (retry) — while a failed *fetch* stays an `Error::S3` and remains transient.
pub(crate) async fn validate_workflow<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    name: &str,
    message: Option<&str>,
    user_meta: Option<&Value>,
    workflow: Option<&Workflow>,
    entries: &[EntryView<'_>],
) -> Res<()> {
    let config = match workflow {
        Some(workflow) => {
            fetch_workflows_config(remote, host, &workflow.config)
                .await?
                .1
        }
        None => None,
    };
    validate_workflow_with_config(
        remote,
        host,
        name,
        message,
        user_meta,
        workflow,
        config.as_ref(),
        entries,
    )
    .await
}

/// The workflow gate given a config the caller has already fetched and parsed,
/// skipping the `.quilt/workflows/config.yml` fetch [`validate_workflow`] would
/// otherwise perform via the header's pinned config URI.
///
/// `config` must be the parsed config that produced `workflow` — the same
/// object the pinned URI in `workflow.config` addresses — so reusing it is
/// semantically identical to re-fetching. Pass `None` for a bucket with no
/// config (a vacuously-valid revision, left untouched); a `None` `workflow` is
/// likewise vacuously valid. Otherwise identical to [`validate_workflow`]: same
/// rules, same [`validate_package`] decision.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn validate_workflow_with_config<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    name: &str,
    message: Option<&str>,
    user_meta: Option<&Value>,
    workflow: Option<&Workflow>,
    config: Option<&WorkflowsConfig>,
    entries: &[EntryView<'_>],
) -> Res<()> {
    let Some(workflow) = workflow else {
        return Ok(());
    };
    let Some(config) = config else {
        return Ok(());
    };
    let rules = match &workflow.id {
        Some(workflow_id) => {
            Some(fetch_workflow_rules(remote, host, config, &workflow_id.id).await?)
        }
        None => None,
    };
    let candidate = PackageCandidate {
        name,
        message,
        user_meta,
        entries,
    };
    validate_package(rules.as_ref(), config.is_workflow_required, &candidate)?;
    Ok(())
}

/// Run the workflow quality gate at push time against the destination
/// bucket's **current** workflows config, ignoring the version-pinned config
/// URI recorded in the manifest header.
///
/// This is the push-side counterpart to [`validate_workflow`] (which trusts
/// the header's resolved, version-pinned workflow and is used by the commit
/// and `set_remote` gates). A revision may have been committed by an older or
/// different client, or against a config version that has since been deleted
/// or lifecycle-expired, so push re-loads `.quilt/workflows/config.yml` from
/// the destination bucket and decides against it — mirroring quilt3, which
/// re-loads the registry's current config on every push
/// (`quilt3/workflows/__init__.py::validate`).
///
/// Cases (with `workflow_id` = the id stamped in the header, if any):
///
/// - No config exists now:
///   - `workflow_id` is `None` → pass (ungoverned bucket).
///   - `workflow_id` is `Some(id)` → hard error, matching quilt3's decision
///     (its message reads "no workflows config exist"; ours fixes the
///     grammar).
/// - Config exists:
///   - `workflow_id` is `Some(id)` missing from the current config → hard
///     error, matching quilt3's decision ("There is no `{id}` workflow in
///     config"; ours adds the article).
///   - `workflow_id` is `None` → [`validate_package`] rejects with
///     `WorkflowRequired` when `is_workflow_required` (default true), else
///     passes.
///   - Otherwise the full gate (metadata/entries schemas, handle pattern,
///     message-required) runs against the current config's rules for `id`.
///
/// A rule failure surfaces as [`crate::Error::WorkflowValidation`] and a
/// missing/unknown workflow as [`crate::Error::RemoteCatalog`] — both
/// classified as conflicts by the sync watcher — while a failed *fetch* stays
/// an `Error::S3` and remains transient.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn validate_workflow_against_current_config<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    bucket: &str,
    name: &str,
    message: Option<&str>,
    user_meta: Option<&Value>,
    header_workflow: Option<&Workflow>,
    entries: &[EntryView<'_>],
) -> Res<()> {
    let workflow_id = header_workflow
        .and_then(|workflow| workflow.id.as_ref())
        .map(|id| id.id.as_str());
    let config = fetch_workflows_config_for_bucket(remote, host, bucket).await?;
    let Some(config) = config else {
        return match workflow_id {
            None => Ok(()),
            Some(id) => Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(format!(
                "\"{id}\" workflow is specified, but no workflows config exists"
            )))),
        };
    };
    let rules = match workflow_id {
        Some(id) => {
            if !config.has_workflow(id) {
                return Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(format!(
                    "There is no \"{id}\" workflow in the config"
                ))));
            }
            Some(fetch_workflow_rules(remote, host, &config, id).await?)
        }
        None => None,
    };
    let candidate = PackageCandidate {
        name,
        message,
        user_meta,
        entries,
    };
    validate_package(rules.as_ref(), config.is_workflow_required, &candidate)?;
    Ok(())
}

/// Resolve the workflow to attach to a manifest header, given the caller's
/// [`WorkflowIntent`] and the presence/contents of `workflows/config.yaml`.
///
/// - [`WorkflowIntent::Named`] — a config is required; the id is resolved against
///   it (`""` and any unknown id error), otherwise it is an error.
/// - [`WorkflowIntent::NoWorkflow`] — `Some(Workflow { id: None })` when a config
///   is present, `None` when there is no config.
/// - [`WorkflowIntent::BucketDefault`] — `None` when there is no config;
///   otherwise the config's top-level `default_workflow` decides: absent →
///   `Some(Workflow { id: None })`; a string → resolved like [`WorkflowIntent::Named`]
///   (a missing referenced workflow errors — misconfiguration must be loud, not
///   silently ungoverned); a non-string value → error.
pub async fn resolve_workflow<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    intent: WorkflowIntent,
    uri: &S3Uri,
) -> Res<Option<Workflow>> {
    let (config, parsed) = fetch_workflows_config(remote, host, uri).await?;
    resolve_workflow_from_config(remote, host, intent, config, parsed.as_ref()).await
}

/// Resolve a workflow from a config the caller has already fetched and parsed,
/// so a caller that must also enforce the workflow (e.g. `set_remote`, which
/// resolves then recommits) fetches `.quilt/workflows/config.yml` exactly once.
///
/// `config` is the (possibly version-pinned) config URI returned by
/// [`fetch_workflows_config`] and `parsed` its parsed value; the resulting
/// [`Workflow`] stamps `config`, so the pinned URI it carries and `parsed`
/// describe the same object. Same resolution as [`resolve_workflow`], which
/// delegates here after fetching.
pub(crate) async fn resolve_workflow_from_config<R: Remote>(
    remote: &R,
    host: &Option<Host>,
    intent: WorkflowIntent,
    config: S3Uri,
    parsed: Option<&WorkflowsConfig>,
) -> Res<Option<Workflow>> {
    match (parsed, intent) {
        (Some(parsed), WorkflowIntent::Named(id)) => {
            resolve_named(remote, host, parsed, config, id).await
        }
        (None, WorkflowIntent::Named(id)) => {
            Err(Error::RemoteCatalog(RemoteCatalogError::Workflow(format!(
                "There is no workflows config, but the workflow \"{id}\" is set"
            ))))
        }
        (Some(_), WorkflowIntent::NoWorkflow) => Ok(Some(Workflow { config, id: None })),
        (None, WorkflowIntent::NoWorkflow) => Ok(None),
        (None, WorkflowIntent::BucketDefault) => Ok(None),
        (Some(parsed), WorkflowIntent::BucketDefault) => match parsed.bucket_default_id()? {
            None => Ok(Some(Workflow { config, id: None })),
            Some(id) => resolve_named(remote, host, parsed, config, id).await,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::remote::mocks::MockRemote;
    use crate::workflow::WorkflowInfo;
    use test_log::test;

    #[test(tokio::test)]
    async fn test_missing_schemas_section() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // Put test config.yaml with workflow but no schemas section
        let config = r"
version: '1'
workflows:
  foo:
    name: Foo
    metadata_schema: bar
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;

        // Should error when trying to resolve a workflow with missing schema section
        let err = resolve_workflow(
            &remote,
            &host,
            WorkflowIntent::Named("foo".to_string()),
            &uri,
        )
        .await
        .unwrap_err();

        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::Workflow(_))
        ));
        assert!(err.to_string().contains("Schemas not found"));

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_no_config_yaml() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // Case 1.a: No config.yaml and workflow_id is None
        let result = resolve_workflow(&remote, &host, WorkflowIntent::NoWorkflow, &uri).await?;
        assert!(result.is_none());

        // Case 1.b: No config.yaml but workflow_id is set
        let err = resolve_workflow(
            &remote,
            &host,
            WorkflowIntent::Named("test-workflow".to_string()),
            &uri,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::Workflow(_))
        ));
        assert!(err.to_string().contains("There is no workflows config"));

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_with_config_yaml() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // Put test config.yaml into mock remote storage
        let config = r"
version: '1'
workflows:
  foo:
    name: Foo
    metadata_schema: bar
schemas:
  bar:
    url: s3://test-bucket/schemas/test.json
";
        let schema_uri: S3Uri = "s3://test-bucket/schemas/test.json".parse()?;
        let schema = b"{}";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;
        remote
            .put_object(&None, &schema_uri, schema.to_vec())
            .await?;

        // Case 2.a: Config exists, workflow_id is set and valid
        let result = resolve_workflow(
            &remote,
            &host,
            WorkflowIntent::Named("foo".to_string()),
            &uri,
        )
        .await?
        .unwrap();
        assert_eq!(result.config, uri);
        assert_eq!(
            result.id.unwrap(),
            WorkflowId {
                id: "foo".to_string(),
                schemas: BTreeMap::from([(
                    "bar".to_string(),
                    "s3://test-bucket/schemas/test.json".parse()?
                )])
            }
        );

        // Case 2.b: Config exists but workflow_id is None
        let result = resolve_workflow(&remote, &host, WorkflowIntent::NoWorkflow, &uri)
            .await?
            .unwrap();
        assert_eq!(result.config, uri);
        assert!(result.id.is_none());

        // Case 2.c: Config exists but workflow_id is not found
        let err = resolve_workflow(
            &remote,
            &host,
            WorkflowIntent::Named("non-existent".to_string()),
            &uri,
        )
        .await
        .unwrap_err();
        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::Workflow(_))
        ));
        assert!(err.to_string().contains("Workflow non-existent not found"));

        // Case 2.d: Config exists but workflow_id is empty
        let err = resolve_workflow(&remote, &host, WorkflowIntent::Named(String::new()), &uri)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::Workflow(_))
        ));
        assert!(err.to_string().contains("Workflow  not found"));

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_bucket_default_no_config() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // No config.yaml: bucket-default resolves to nothing.
        let result = resolve_workflow(&remote, &host, WorkflowIntent::BucketDefault, &uri).await?;
        assert!(result.is_none());

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_bucket_default_without_key() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // Config present but no `default_workflow` key → today's null-id record.
        let config = r"
version: '1'
workflows:
  foo:
    name: Foo
    metadata_schema: bar
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;

        let result = resolve_workflow(&remote, &host, WorkflowIntent::BucketDefault, &uri)
            .await?
            .unwrap();
        assert_eq!(result.config, uri);
        assert!(result.id.is_none());

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_bucket_default_with_valid_key() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // Config declares a `default_workflow` pointing at a valid workflow.
        let config = r"
version: '1'
default_workflow: foo
workflows:
  foo:
    name: Foo
    metadata_schema: bar
schemas:
  bar:
    url: s3://test-bucket/schemas/test.json
";
        let schema_uri: S3Uri = "s3://test-bucket/schemas/test.json".parse()?;
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;
        remote
            .put_object(&None, &schema_uri, b"{}".to_vec())
            .await?;

        let result = resolve_workflow(&remote, &host, WorkflowIntent::BucketDefault, &uri)
            .await?
            .unwrap();
        assert_eq!(result.config, uri);
        assert_eq!(
            result.id.unwrap(),
            WorkflowId {
                id: "foo".to_string(),
                schemas: BTreeMap::from([(
                    "bar".to_string(),
                    "s3://test-bucket/schemas/test.json".parse()?
                )])
            }
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_resolve_stamps_both_declared_schemas() -> Res<()> {
        // A workflow declaring both a metadata_schema and an entries_schema must
        // stamp *both* content addresses, keyed by schema id — matching quilt3's
        // `data_to_store['schemas']`. Regression guard for the parity bug where
        // quilt-rs recorded only the metadata schema.
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        let config = r"
version: '1'
workflows:
  dual:
    name: Dual
    metadata_schema: meta-id
    entries_schema: entries-id
schemas:
  meta-id:
    url: s3://test-bucket/schemas/meta.json
  entries-id:
    url: s3://test-bucket/schemas/entries.json
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;
        for key in ["meta.json", "entries.json"] {
            remote
                .put_object(
                    &None,
                    &format!("s3://test-bucket/schemas/{key}").parse()?,
                    b"{}".to_vec(),
                )
                .await?;
        }

        let result = resolve_workflow(
            &remote,
            &host,
            WorkflowIntent::Named("dual".to_string()),
            &uri,
        )
        .await?
        .unwrap();

        assert_eq!(
            result.id.unwrap(),
            WorkflowId {
                id: "dual".to_string(),
                schemas: BTreeMap::from([
                    (
                        "meta-id".to_string(),
                        "s3://test-bucket/schemas/meta.json".parse()?
                    ),
                    (
                        "entries-id".to_string(),
                        "s3://test-bucket/schemas/entries.json".parse()?
                    ),
                ]),
            }
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_resolve_stamps_entries_only_schema() -> Res<()> {
        // An entries-schema-only workflow (the reproduced `fiskus-sandbox-dev`
        // case): the stamp carries the entries schema, where the old model
        // recorded nothing.
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        let config = r"
version: '1'
workflows:
  entries-only:
    name: Entries only
    entries_schema: entries-id
schemas:
  entries-id:
    url: s3://test-bucket/schemas/entries.json
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;
        remote
            .put_object(
                &None,
                &"s3://test-bucket/schemas/entries.json".parse()?,
                b"{}".to_vec(),
            )
            .await?;

        let result = resolve_workflow(
            &remote,
            &host,
            WorkflowIntent::Named("entries-only".to_string()),
            &uri,
        )
        .await?
        .unwrap();

        assert_eq!(
            result.id.unwrap(),
            WorkflowId {
                id: "entries-only".to_string(),
                schemas: BTreeMap::from([(
                    "entries-id".to_string(),
                    "s3://test-bucket/schemas/entries.json".parse()?
                )]),
            }
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_bucket_default_missing_referenced_workflow() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // `default_workflow` names a workflow that is not declared → loud error.
        let config = r"
version: '1'
default_workflow: ghost
workflows:
  foo:
    name: Foo
    metadata_schema: bar
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;

        let err = resolve_workflow(&remote, &host, WorkflowIntent::BucketDefault, &uri)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::Workflow(_))
        ));
        assert!(err.to_string().contains("Workflow ghost not found"));

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_bucket_default_non_string_key() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // `default_workflow` present but not a string → the config-schema
        // validation at load rejects it (quilt3 types `default_workflow` as a
        // string), so it never reaches resolution.
        let config = r"
version: '1'
default_workflow: [not, a, string]
workflows:
  foo:
    name: Foo
    metadata_schema: bar
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;

        let err = resolve_workflow(&remote, &host, WorkflowIntent::BucketDefault, &uri)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::InvalidWorkflowsConfig(_))
        ));
        let message = err.to_string();
        assert!(
            message.contains("does not satisfy the workflows config schema"),
            "unexpected message: {message}"
        );
        assert!(
            message.contains("/default_workflow"),
            "violation must name the offending path, got: {message}"
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_bucket_default_explicit_null_key() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        // `default_workflow:` with no value is a YAML explicit null — not a
        // string, so the config-schema validation at load rejects it (quilt3
        // types `default_workflow` as a string and schema-validates the whole
        // config on load, so this config fails every push there too).
        let config = r"
version: '1'
default_workflow:
workflows:
  foo:
    name: Foo
    metadata_schema: bar
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;

        let err = resolve_workflow(&remote, &host, WorkflowIntent::BucketDefault, &uri)
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::InvalidWorkflowsConfig(_))
        ));
        let message = err.to_string();
        assert!(
            message.contains("does not satisfy the workflows config schema"),
            "unexpected message: {message}"
        );
        assert!(
            message.contains("/default_workflow"),
            "violation must name the offending path, got: {message}"
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_fetch_workflows_config_for_bucket_present() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;

        let uri: S3Uri = "s3://my-bucket/.quilt/workflows/config.yml".parse()?;
        let config = r"
version: '1'
default_workflow: foo
workflows:
  foo:
    name: Foo
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;

        let parsed = fetch_workflows_config_for_bucket(&remote, &host, "my-bucket")
            .await?
            .expect("config present → Some");
        assert_eq!(parsed.default_workflow, Some("foo".to_string()));
        assert_eq!(
            parsed.workflows,
            vec![WorkflowInfo {
                id: "foo".to_string(),
                name: Some("Foo".to_string()),
                description: None,
            }]
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_fetch_workflows_config_for_bucket_absent() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;

        // No config object for this bucket → None.
        let result = fetch_workflows_config_for_bucket(&remote, &host, "empty-bucket").await?;
        assert!(result.is_none());

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_fetch_workflow_rules() -> Res<()> {
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        let config = r#"
version: "1"
workflows:
  foo:
    name: Foo
    handle_pattern: "^team/"
    is_message_required: true
    metadata_schema: meta
    entries_schema: entries
schemas:
  meta:
    url: s3://test-bucket/schemas/meta.json
  entries:
    url: s3://test-bucket/schemas/entries.json
"#;
        let meta_uri: S3Uri = "s3://test-bucket/schemas/meta.json".parse()?;
        let entries_uri: S3Uri = "s3://test-bucket/schemas/entries.json".parse()?;
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;
        remote
            .put_object(
                &None,
                &meta_uri,
                br#"{"type": "object", "required": ["owner"]}"#.to_vec(),
            )
            .await?;
        remote
            .put_object(&None, &entries_uri, br#"{"type": "array"}"#.to_vec())
            .await?;

        let (_, parsed) = fetch_workflows_config(&remote, &host, &uri).await?;
        let parsed = parsed.expect("config present");
        let rules = fetch_workflow_rules(&remote, &host, &parsed, "foo").await?;

        assert_eq!(
            rules,
            WorkflowRules {
                handle_pattern: Some("^team/".to_string()),
                is_message_required: true,
                metadata_schema: Some(serde_json::json!({
                    "type": "object", "required": ["owner"]
                })),
                entries_schema: Some(serde_json::json!({ "type": "array" })),
            }
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_fetch_workflow_rules_no_schemas() -> Res<()> {
        // A workflow declaring neither schema nor the optional flags yields
        // empty rules: no fetch attempted, defaults applied.
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        let config = r"
version: '1'
workflows:
  bare:
    name: Bare
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;

        let (_, parsed) = fetch_workflows_config(&remote, &host, &uri).await?;
        let parsed = parsed.expect("config present");
        let rules = fetch_workflow_rules(&remote, &host, &parsed, "bare").await?;

        assert_eq!(
            rules,
            WorkflowRules {
                handle_pattern: None,
                is_message_required: false,
                metadata_schema: None,
                entries_schema: None,
            }
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_non_string_schema_key_rejected_by_config_schema() -> Res<()> {
        // `metadata_schema: ~` — the key IS present, just null. The config
        // schema types it as a string, so the whole config is now rejected at
        // load (before any resolution), with a violation naming the exact path
        // — a typo can no longer silently disable schema validation.
        let remote = MockRemote::default();
        let host = None;
        let uri: S3Uri = "s3://any/.quilt/workflows/config.yml".parse()?;

        let config = r"
version: '1'
workflows:
  foo:
    name: Foo
    metadata_schema: ~
";
        remote
            .put_object(&None, &uri, config.as_bytes().to_vec())
            .await?;

        let err = fetch_workflows_config(&remote, &host, &uri)
            .await
            .unwrap_err();

        assert!(matches!(
            err,
            Error::RemoteCatalog(RemoteCatalogError::InvalidWorkflowsConfig(_))
        ));
        let message = err.to_string();
        assert!(
            message.contains("does not satisfy the workflows config schema"),
            "unexpected message: {message}"
        );
        assert!(
            message.contains("/workflows/foo/metadata_schema"),
            "violation must name the offending path, got: {message}"
        );

        Ok(())
    }

    #[cfg(unix)]
    #[test]
    fn test_entry_view_non_utf8_logical_key_is_lossy() {
        use std::ffi::OsStr;
        use std::os::unix::ffi::OsStrExt;
        use std::path::PathBuf;

        use crate::object_hash::ObjectHash;

        // A logical key with an invalid UTF-8 byte must project as the lossy
        // form (U+FFFD replacement character), not as a fabricated empty key.
        let row = ManifestRow {
            logical_key: PathBuf::from(OsStr::from_bytes(b"bad-\xFF.txt")),
            physical_key: "s3://bucket/bad".to_string(),
            hash: ObjectHash::default(),
            size: 1,
            meta: None,
        };
        let view = entry_view(&row);
        assert_eq!(view.logical_key, "bad-\u{FFFD}.txt");
    }
}
