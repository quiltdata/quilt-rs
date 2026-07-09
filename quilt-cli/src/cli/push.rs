use quilt_rs::PushOutcome;
use quilt_rs::io::remote::HostConfig;
use quilt_rs::io::remote::WorkflowIntent;
use quilt_uri::Host;
use quilt_uri::Namespace;

use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Std;

#[derive(Debug)]
pub struct Input {
    pub namespace: Namespace,
    pub host_config: Option<HostConfig>,
    pub bucket: Option<String>,
    pub origin: Option<Host>,
    /// Workflow for the first push (when `bucket`/`origin` are set and a
    /// re-commit is performed). Ignored on subsequent pushes.
    pub workflow: WorkflowIntent,
}

#[derive(Debug)]
pub struct Output {
    pub hash: String,
    pub certified_latest: bool,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, r#"New revision "{}" pushed"#, self.hash)?;
        if !self.certified_latest {
            write!(
                f,
                "\nWarning: could not update latest — remote has newer changes"
            )?;
        }
        Ok(())
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.push(args).await)
}

async fn push_package(
    local_domain: &quilt_rs::LocalDomain,
    namespace: Namespace,
    host_config: Option<HostConfig>,
    bucket: Option<String>,
    origin: Option<Host>,
    workflow: WorkflowIntent,
) -> Result<PushOutcome, Error> {
    match local_domain.get_installed_package(&namespace).await? {
        Some(installed_package) => {
            // If bucket/origin provided, set remote before pushing.
            // Safe: push() reads lineage fresh from disk, so it sees the
            // remote_uri written by set_remote() — no caching in between.
            // Note: clap enforces that bucket and origin are always provided
            // together via `requires` constraints.
            if let (Some(bucket), Some(origin)) = (bucket, origin) {
                let outcome = installed_package
                    .set_remote(bucket, Some(origin), workflow)
                    .await?;
                // The remote's default workflow could not be resolved on the
                // best-effort BucketDefault path: the push proceeds without a
                // workflow stamp, but tell the user why rather than leaving it
                // silent. Warnings go to stderr so the success output on stdout
                // stays machine-parseable; the exit status is unchanged.
                if let Some(reason) = outcome.resolution_warning {
                    eprintln!(
                        "warning: couldn't resolve the bucket's default workflow ({reason}); pushing without a workflow"
                    );
                }
            }
            Ok(installed_package.push(host_config).await?)
        }
        None => Err(Error::NamespaceNotFound(namespace)),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input {
        namespace,
        host_config,
        bucket,
        origin,
        workflow,
    }: Input,
) -> Result<Output, Error> {
    let outcome = push_package(
        local_domain,
        namespace,
        host_config,
        bucket,
        origin,
        workflow,
    )
    .await?;
    Ok(Output {
        hash: outcome.manifest_uri.hash,
        certified_latest: outcome.certified_latest,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use quilt_rs::flow::UserMeta;
    use quilt_rs::io::remote::WorkflowIntent;
    use quilt_rs::io::storage::ByteStream;

    use quilt_rs::io::storage::LocalStorage;
    use quilt_rs::io::storage::Storage;
    use std::path::PathBuf;

    use crate::cli::commit;
    use crate::cli::fixtures::packages::default as pkg;
    use crate::cli::model::create_model_in_temp_dir;
    use crate::cli::model::install_package_into_temp_dir;

    /// Verifies that push command returns error when push a non-existent package
    #[test(tokio::test)]
    async fn test_namespace_not_found() -> Result<(), Error> {
        let (m, _temp_dir) = create_model_in_temp_dir().await?;

        if let Std::Err(error_str) = command(
            m,
            Input {
                namespace: ("in", "valid").into(),
                host_config: None,
                bucket: None,
                origin: None,
                workflow: WorkflowIntent::BucketDefault,
            },
        )
        .await
        {
            assert_eq!(error_str.to_string(), "Package in/valid not found");
        } else {
            return Err(Error::Test("Expected package not found error".to_string()));
        }

        Ok(())
    }

    /// A push re-validates the revision against the destination bucket's
    /// **current** workflow config, not the version-pinned config the commit
    /// resolved against. A package committed valid under a permissive config is
    /// rejected once the bucket owner tightens that config — and the rejection
    /// surfaces at the CLI boundary with the validator's own wording, naming the
    /// failing rule (`metadata_schema`) and the missing field (`owner`).
    ///
    /// This drives `InstalledPackage::push` (the call `push::model` makes)
    /// against an in-memory mock bucket, exposed by quilt-rs's `testing`
    /// feature — the only way to script the "config mutates between commit and
    /// push" scenario a live bucket cannot reproduce. It mirrors the commit-side
    /// `test_commit_rejected_by_workflow_surfaces_clear_error`.
    #[test(tokio::test)]
    async fn test_push_rejected_by_mutated_workflow_surfaces_clear_error() -> Result<(), Error> {
        use quilt_rs::InstalledPackage;
        use quilt_rs::io::remote::Remote;
        use quilt_rs::io::remote::mocks::MockRemote;
        use quilt_rs::lineage::DomainLineageIo;
        use quilt_rs::lineage::Home;
        use quilt_rs::lineage::PackageLineageIo;
        use quilt_rs::manifest::Workflow;
        use quilt_rs::manifest::WorkflowId;
        use quilt_rs::paths::DomainPaths;
        use quilt_uri::S3Uri;
        use tempfile::TempDir;

        let home_dir = TempDir::new()?;
        let paths_dir = TempDir::new()?;
        let storage = LocalStorage::new();
        let remote = MockRemote::default();
        let namespace: Namespace = ("reference", "push-gate").into();
        let home = Home::new(home_dir.path().to_path_buf());
        let paths = DomainPaths::new(paths_dir.path().to_path_buf());

        paths
            .scaffold_for_installing(&storage, &home, &namespace)
            .await?;

        // A brand-new local package: a commit will be stamped below, and only
        // then is the remote attached, mirroring `create → commit → set-remote`.
        let lineage_json = format!(
            r#"{{
                "packages": {{
                    "reference/push-gate": {{
                        "commit": null,
                        "remote": null,
                        "base_hash": "",
                        "latest_hash": "",
                        "paths": {{}}
                    }}
                }},
                "home": "{}"
            }}"#,
            home_dir.path().display()
        );
        storage
            .write_byte_stream(&paths.lineage(), lineage_json.as_bytes().to_vec().into())
            .await?;

        let config_uri = S3Uri::try_from("s3://b/.quilt/workflows/config.yml")?;
        // Config v1: a permissive `gate` workflow with no metadata_schema.
        remote
            .put_object(
                &None,
                &config_uri,
                b"version: \"1\"\nworkflows:\n  gate:\n    name: Gate\n".to_vec(),
            )
            .await?;

        let package = InstalledPackage {
            lineage: PackageLineageIo::new(
                DomainLineageIo::new(paths.lineage()),
                namespace.clone(),
            ),
            paths,
            remote,
            storage,
            namespace: namespace.clone(),
        };

        // Commit valid under v1: the `gate` workflow (which declares no
        // metadata_schema) is stamped into the header, so clearing the
        // package metadata passes the commit-time gate.
        let workflow = Workflow {
            config: config_uri.to_string().parse()?,
            id: Some(WorkflowId {
                id: "gate".to_string(),
                metadata: None,
            }),
        };
        package
            .commit(
                "governed commit".to_string(),
                UserMeta::Clear,
                Some(workflow),
                None,
            )
            .await?;

        // Attach the governed bucket `b` now, first-push (no remote hash yet),
        // so push skips the remote-manifest browse and reaches the gate.
        package
            .set_remote("b".to_string(), None, WorkflowIntent::BucketDefault)
            .await?;

        // The bucket owner tightens the *current* config: `gate` now requires an
        // `owner` key the committed (cleared) metadata lacks. Removing this
        // mutation is the discriminating (RED-equivalent) run: the push then
        // clears the gate and succeeds, and the rejection assertions fail.
        package
            .remote
            .put_object(
                &None,
                &config_uri,
                b"version: \"1\"\nworkflows:\n  gate:\n    name: Gate\n    metadata_schema: meta\nschemas:\n  meta:\n    url: s3://b/schemas/meta.json\n".to_vec(),
            )
            .await?;
        package
            .remote
            .put_object(
                &None,
                &S3Uri::try_from("s3://b/schemas/meta.json")?,
                br#"{"type": "object", "required": ["owner"]}"#.to_vec(),
            )
            .await?;

        // Push re-resolves the current config and rejects the identical
        // manifest. The CLI wraps the quilt-rs error transparently, so the
        // validator's own wording reaches the user verbatim.
        let Err(err) = package.push(None).await else {
            return Err(Error::Test(
                "push must be rejected by the tightened workflow config".to_string(),
            ));
        };
        let message = Error::from(err).to_string();
        assert!(
            message.starts_with("quilt_rs error: package does not satisfy the workflow"),
            "message must announce a workflow rejection, got: {message}"
        );
        assert!(
            message.contains("metadata_schema"),
            "message must name the failing rule, got: {message}"
        );
        assert!(
            message.contains("owner"),
            "message must name the missing field, got: {message}"
        );

        Ok(())
    }

    /// Verifies that push command returns error when there are no commits:
    ///   * installs a package but makes no commits
    ///   * attempts to push without commits
    #[test(tokio::test)]
    async fn test_no_commit() -> Result<(), Error> {
        let uri = pkg::URI;
        let (m, _, _temp_dir) = install_package_into_temp_dir(uri).await?;

        if let Std::Err(error_str) = command(
            m,
            Input {
                namespace: pkg::NAMESPACE.into(),
                host_config: None,
                bucket: None,
                origin: None,
                workflow: WorkflowIntent::BucketDefault,
            },
        )
        .await
        {
            assert_eq!(
                error_str.to_string(),
                "quilt_rs error: Push error: No commits to push"
            );
        } else {
            return Err(Error::Test("Expected no changes error".to_string()));
        }

        Ok(())
    }

    /// Comprehensive integration test for push workflow:
    /// 1. Pull package from data-yaml-spec-tests bucket
    /// 2. Verify initial top hash: 4076eb7774f5159aab212302288a2a2a9e59fab69cf4e41e827072fee80fabb4
    /// 3. Modify e0-0.txt content, commit with specific message and meta
    /// 4. Push and verify first expected top hash: c8027e8697016feb74b8ea523ca55934243653b890b94d64166ef2664a71ebab
    /// 5. Revert e0-0.txt content, commit with different message and meta
    /// 6. Push and verify final expected top hash matches original: 4076eb7774f5159aab212302288a2a2a9e59fab69cf4e41e827072fee80fabb4
    #[test(tokio::test)]
    async fn test_push_sha256_checksum() -> Result<(), Error> {
        let namespace: Namespace = ("quilt_rs", "test").into();
        let uri = "quilt+s3://data-yaml-spec-tests#package=quilt_rs/test";
        let host_config = Some(HostConfig::default_sha256_chunked());

        // Step 1: Install (pull) the package
        let (m, installed_package, _temp_dir) = install_package_into_temp_dir(uri).await?;

        // Step 2: Verify initial top hash
        let initial_lineage = installed_package.lineage().await?;
        let initial_hash = initial_lineage.current_hash().unwrap();
        assert_eq!(
            initial_hash, "4076eb7774f5159aab212302288a2a2a9e59fab69cf4e41e827072fee80fabb4",
            "Initial top hash should match expected value"
        );

        let working_dir = installed_package.package_home().await?;
        let storage = LocalStorage::new();
        let e0_file_path = working_dir.join(PathBuf::from("e0-0.txt"));

        // Step 3: Change e0-0.txt content
        storage
            .write_byte_stream(
                &e0_file_path,
                ByteStream::from_static(b"Emperor-Drainage8-Presoak\n"),
            )
            .await?;

        m.commit(commit::Input {
            message: "Unbounded Defy 2 Landmine".to_string(),
            namespace: namespace.clone(),
            user_meta: UserMeta::Set(serde_json::json!({"Naturist": "Conjure"})),
            workflow: WorkflowIntent::NoWorkflow,
            host_config: None,
        })
        .await?;

        // Step 4: Push new package revision with changed file using SHA256 chunked checksums
        let first_push_output = m
            .push(Input {
                namespace: namespace.clone(),
                host_config: host_config.clone(),
                bucket: None,
                origin: None,
                workflow: WorkflowIntent::BucketDefault,
            })
            .await?;

        assert_eq!(
            first_push_output.hash,
            "c8027e8697016feb74b8ea523ca55934243653b890b94d64166ef2664a71ebab",
            "First push top hash should match expected value"
        );

        // Step 5: Revert e0-0.txt content
        storage
            .write_byte_stream(
                &e0_file_path,
                ByteStream::from_static(b"Thu Feb 29 19:07:56 PST 2024\n"),
            )
            .await?;

        m.commit(commit::Input {
            message: "Equate 1 Fragment Grimace".to_string(),
            namespace: namespace.clone(),
            user_meta: UserMeta::Set(serde_json::json!({"Antitoxic": "Mankind"})),
            workflow: WorkflowIntent::NoWorkflow,
            host_config: None,
        })
        .await?;

        // Step 6: Push and verify final expected top hash matches original using SHA256 chunked checksums
        let final_push_output = m
            .push(Input {
                namespace,
                host_config,
                bucket: None,
                origin: None,
                workflow: WorkflowIntent::BucketDefault,
            })
            .await?;

        assert_eq!(
            final_push_output.hash,
            "4076eb7774f5159aab212302288a2a2a9e59fab69cf4e41e827072fee80fabb4",
            "Final push top hash should match original expected value"
        );

        Ok(())
    }

    /// Integration test: create local package → set bucket → push to S3 → verify lineage.
    /// Uses fiskus-us-east-1 with a dedicated namespace (no catalog — local AWS creds).
    #[test(tokio::test)]
    async fn test_push_local_package_bucket_only() -> Result<(), Error> {
        use crate::cli::create;
        use crate::cli::status;
        use quilt_rs::lineage::UpstreamState;

        let namespace: Namespace = ("cli_test", "local_push").into();
        let host_config = Some(HostConfig::default_sha256_chunked());

        // Step 1: Create model and local package
        let (m, _temp_dir) = create_model_in_temp_dir().await?;

        let create_output = m
            .create(create::Input {
                namespace: namespace.clone(),
                source: None,
                message: None,
            })
            .await?;

        // Step 2: Write a file into the package home directory
        let working_dir = create_output.installed_package.package_home().await?;
        let storage = LocalStorage::new();

        let data_file = working_dir.join("data.txt");
        storage
            .write_byte_stream(
                &data_file,
                ByteStream::from_static(b"hello from local package\n"),
            )
            .await?;

        // Step 3: Commit
        m.commit(commit::Input {
            message: "Add data".to_string(),
            namespace: namespace.clone(),
            user_meta: UserMeta::Keep,
            workflow: WorkflowIntent::NoWorkflow,
            host_config: None,
        })
        .await?;

        // Step 4: Set remote bucket (no catalog — uses local AWS creds)
        create_output
            .installed_package
            .set_remote(
                "fiskus-us-east-1".to_string(),
                None,
                WorkflowIntent::BucketDefault,
            )
            .await?;

        // Step 5: Push to S3 (first push — no existing latest tag)
        let push_output = m
            .push(Input {
                namespace: namespace.clone(),
                host_config: host_config.clone(),
                bucket: None,
                origin: None,
                workflow: WorkflowIntent::BucketDefault,
            })
            .await?;

        assert!(
            !push_output.hash.is_empty(),
            "Push should return a non-empty hash"
        );

        // Step 6: Verify lineage after push
        let lineage = create_output.installed_package.lineage().await?;
        let remote_uri = lineage
            .remote_uri
            .as_ref()
            .expect("remote_uri should be set");
        assert_eq!(
            lineage.base_hash, remote_uri.hash,
            "base_hash should equal remote hash after push"
        );
        assert!(
            !remote_uri.hash.is_empty(),
            "remote hash should not be empty after push"
        );
        assert_eq!(remote_uri.bucket, "fiskus-us-east-1", "bucket should match");

        // Step 7: Status should be UpToDate (first push certifies latest)
        let status_output = m
            .status(status::Input {
                namespace: namespace.clone(),
                host_config,
            })
            .await?;
        assert_eq!(
            status_output.status.upstream_state,
            UpstreamState::UpToDate,
            "Package should be up-to-date after first push"
        );

        Ok(())
    }

    /// Comprehensive integration test for push workflow with CRC64 checksums:
    /// 1. Pull package from fiskus-us-east-1 bucket with CRC64 hashing
    /// 2. Verify initial top hash: b427c3867bce2445a988f69f43ad3998237d2fedf6f5e678822acd1a1e8f580a
    /// 3. Modify 1.txt content, commit with specific message and meta
    /// 4. Push and verify first expected top hash: 8c9beced00f51cb100da861e62688e71f77a692a1c71bce422e329706ede6e63
    /// 5. Revert 1.txt content, commit with different message and meta
    /// 6. Push and verify final expected top hash matches original: b427c3867bce2445a988f69f43ad3998237d2fedf6f5e678822acd1a1e8f580a
    #[test(tokio::test)]
    async fn test_push_crc64_checksum() -> Result<(), Error> {
        let namespace: Namespace = ("crc64", "s3").into();
        let uri = "quilt+s3://fiskus-us-east-1#package=crc64/s3";
        let host_config = Some(HostConfig::default_crc64());

        // Step 1: Install (pull) the package
        let (m, installed_package, _temp_dir) = install_package_into_temp_dir(uri).await?;

        // Step 2: Verify initial top hash
        let initial_lineage = installed_package.lineage().await?;
        let initial_hash = initial_lineage.current_hash().unwrap();
        assert_eq!(
            initial_hash, "b427c3867bce2445a988f69f43ad3998237d2fedf6f5e678822acd1a1e8f580a",
            "Initial top hash should match expected value"
        );

        let working_dir = installed_package.package_home().await?;
        let storage = LocalStorage::new();
        let file_path = working_dir.join(PathBuf::from("1.txt"));

        // Step 3: Change 1.txt content
        storage
            .write_byte_stream(
                &file_path,
                ByteStream::from_static(b"Emperor-Drainage8-Presoak\n"),
            )
            .await?;

        m.commit(commit::Input {
            message: "Unbounded Defy 2 Landmine".to_string(),
            namespace: namespace.clone(),
            user_meta: UserMeta::Set(serde_json::json!({"Naturist": "Conjure"})),
            workflow: WorkflowIntent::NoWorkflow,
            host_config: host_config.clone(),
        })
        .await?;

        // Step 4: Push new package revision with changed file using CRC64 checksums
        let first_push_output = m
            .push(Input {
                namespace: namespace.clone(),
                host_config: host_config.clone(),
                bucket: None,
                origin: None,
                workflow: WorkflowIntent::BucketDefault,
            })
            .await?;

        assert_eq!(
            first_push_output.hash,
            "8c9beced00f51cb100da861e62688e71f77a692a1c71bce422e329706ede6e63",
            "First push top hash should match expected value"
        );

        // Step 5: Revert 1.txt content
        storage
            .write_byte_stream(
                &file_path,
                ByteStream::from_static(b"jue 27 nov 2025 16:36:45 CET\n"),
            )
            .await?;

        m.commit(commit::Input {
            message: "Initial commit".to_string(),
            namespace: namespace.clone(),
            // The original revision has no package-level metadata, so the
            // revert must clear step 3's — `Keep` would inherit it and the
            // final top hash would no longer match the original.
            user_meta: UserMeta::Clear,
            workflow: WorkflowIntent::NoWorkflow,
            host_config: host_config.clone(),
        })
        .await?;

        // Step 6: Push and verify final expected top hash matches original using CRC64 checksums
        let final_push_output = m
            .push(Input {
                namespace,
                host_config,
                bucket: None,
                origin: None,
                workflow: WorkflowIntent::BucketDefault,
            })
            .await?;

        assert_eq!(
            final_push_output.hash,
            "b427c3867bce2445a988f69f43ad3998237d2fedf6f5e678822acd1a1e8f580a",
            "Final push top hash should match original expected value"
        );

        Ok(())
    }
}
