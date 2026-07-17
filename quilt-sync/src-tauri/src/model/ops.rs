//! Command-level operations over [`QuiltModel`](super::QuiltModel).

use std::path::PathBuf;

use crate::commit_message;
use crate::error::Error;
use crate::publish_settings::PublishSettings;
use crate::quilt;
use crate::telemetry::prelude::*;

use quilt_rs::flow::UserMeta;
use quilt_rs::io::remote::HostConfig;
use quilt_rs::io::remote::WorkflowIntent;

use quilt_uri::ManifestUri;
use quilt_uri::Namespace;

use super::{InstallCheck, InstallOutcome, QuiltModel};

fn parse_metadata(input: &str) -> Result<UserMeta, Error> {
    // Whitespace-only is treated as "no opinion" — keep the package's
    // existing metadata. The UI form (`update_publish_settings`) trims
    // before validating and saves the normalised value, so this branch
    // also covers hand-edited `publish_settings.json` files.
    if input.trim().is_empty() {
        return Ok(UserMeta::Keep);
    }
    match serde_json::from_str(input) {
        Ok(json) => Ok(UserMeta::Set(json)),
        Err(err) => Err(Error::Json(err)),
    }
}

pub async fn package_commit(
    model: &impl QuiltModel,
    namespace: quilt_uri::Namespace,
    message: &str,
    metadata: &str,
    workflow: WorkflowIntent,
    host_config: Option<HostConfig>,
) -> Result<(), Error> {
    debug!(
        "Committing the package.\nNamespace:\n{},\nmessage: {},\nuser_meta: {},\nworkflow: {:?}",
        namespace, message, metadata, workflow
    );
    let metadata = parse_metadata(metadata)?;

    let installed_package = model
        .get_installed_package(&namespace)
        .await?
        .ok_or_else(|| Error::from(quilt::InstallPackageError::NotInstalled(namespace)))?;

    let workflow = installed_package.resolve_workflow(workflow).await?;
    model
        .package_commit(
            &installed_package,
            message.to_string(),
            metadata,
            workflow,
            host_config,
        )
        .await?;
    Ok(())
}

pub async fn install_paths(
    model: &impl QuiltModel,
    installed_package: &quilt::InstalledPackage,
    paths: Vec<PathBuf>,
) -> Result<PathBuf, Error> {
    if paths.is_empty() {
        return Err(Error::General(
            "Cannot install paths: empty paths vector provided".to_string(),
        ));
    }

    let namespace = &installed_package.namespace;

    model
        .package_install_paths(installed_package, &paths)
        .await?;

    // Post-installation actions based on number of paths
    if paths.len() == 1 {
        let path = &paths[0];
        info!("Installed {:?}", path);
        model.reveal_in_file_browser(namespace, path).await
    } else {
        info!("Installed {} paths", paths.len());
        model.open_in_file_browser(namespace).await
    }
}

pub async fn install_package_only(
    model: &impl QuiltModel,
    uri: &quilt_uri::S3PackageUri,
) -> Result<InstallOutcome, Error> {
    let manifest_uri = model.resolve_manifest_uri(uri).await?;

    match model.is_package_installed(&manifest_uri).await? {
        InstallCheck::AlreadyInstalled => {
            debug!("Package already installed: {:?}", manifest_uri.namespace);
            Ok(InstallOutcome::Installed)
        }
        InstallCheck::DifferentVersion(installed_hash) => {
            debug!(
                "Different version already installed: {:?}",
                manifest_uri.namespace
            );
            Ok(InstallOutcome::DifferentVersion {
                requested_hash: manifest_uri.hash.clone(),
                installed_hash,
            })
        }
        InstallCheck::LocalOnly => {
            debug!(
                "Local-only package already installed: {:?}",
                manifest_uri.namespace
            );
            Ok(InstallOutcome::LocalOnly)
        }
        InstallCheck::NotInstalled => {
            debug!("Package not installed, installing: {:?}", manifest_uri);
            model.package_install(&manifest_uri).await?;
            Ok(InstallOutcome::Installed)
        }
    }
}

pub async fn install_paths_only(
    model: &impl QuiltModel,
    namespace: &quilt_uri::Namespace,
    paths: Vec<PathBuf>,
) -> Result<PathBuf, Error> {
    let installed_package = model
        .get_installed_package(namespace)
        .await?
        .ok_or_else(|| Error::General("Package not found for path installation".to_string()))?;

    install_paths(model, &installed_package, paths).await
}

pub async fn package_uninstall(
    model: &impl QuiltModel,
    namespace: quilt_uri::Namespace,
) -> Result<(), Error> {
    debug!("Uninstall package for {} namespace", &namespace);
    model.package_uninstall(namespace).await?;
    Ok(())
}

pub fn open_in_web_browser(url: &str) -> Result<(), Error> {
    Ok(opener::open(url)?)
}

pub async fn package_revision_certify_latest(
    model: &impl QuiltModel,
    namespace: quilt_uri::Namespace,
) -> Result<(), Error> {
    let installed_package = model
        .get_installed_package(&namespace)
        .await?
        .unwrap_or_else(|| panic!("Package {namespace} not found"));
    model
        .package_revision_certify_latest(&installed_package)
        .await?;
    Ok(())
}

pub async fn package_revision_reset_local(
    model: &impl QuiltModel,
    namespace: quilt_uri::Namespace,
) -> Result<(), Error> {
    let installed_package = model
        .get_installed_package(&namespace)
        .await?
        .unwrap_or_else(|| panic!("Package {namespace} not found"));
    model
        .package_revision_reset_local(&installed_package)
        .await?;
    Ok(())
}

pub async fn package_push(
    model: &impl QuiltModel,
    namespace: &quilt_uri::Namespace,
    host_config: Option<HostConfig>,
) -> Result<quilt::PushOutcome, Error> {
    let installed_package = model
        .get_installed_package(namespace)
        .await?
        .unwrap_or_else(|| panic!("Package {namespace} not found"));
    model.package_push(&installed_package, host_config).await
}

/// Render `PublishSettings` into a message / metadata / workflow triple
/// and route through [`package_publish`].
///
/// Shared entry point for both the manual one-click Publish command and
/// the autosync watcher tick — keep them in lockstep so a change to
/// publish settings (new placeholder, new field) applies identically
/// regardless of who triggered the publish. Returns the outcome paired
/// with the rendered commit message; the autosync tick needs the message
/// string for its `autosync-published` event, the manual command can
/// discard it.
pub async fn publish_with_settings(
    model: &impl QuiltModel,
    namespace: &quilt_uri::Namespace,
    settings: &PublishSettings,
    status: quilt::lineage::InstalledPackageStatus,
) -> Result<(quilt::PublishOutcome, String), Error> {
    let changes_summary = commit_message::generate(&status.changes);
    let message = commit_message::render_publish_message(
        settings.message_template.as_deref().unwrap_or_default(),
        &commit_message::PublishMessageContext {
            namespace,
            changes_summary,
        },
    );
    let metadata = settings.default_metadata.clone().unwrap_or_default();
    // A missing, empty, or whitespace-only `default_workflow` means "no opinion"
    // — honour the bucket's default workflow; a non-empty id (after trimming)
    // enforces that named workflow.
    let workflow = WorkflowIntent::from_optional_id(settings.default_workflow.as_deref());
    let outcome = package_publish(
        model,
        namespace.clone(),
        &message,
        &metadata,
        workflow,
        None,
        Some(status),
    )
    .await?;
    Ok((outcome, message))
}

pub async fn package_publish(
    model: &impl QuiltModel,
    namespace: quilt_uri::Namespace,
    message: &str,
    metadata: &str,
    workflow: WorkflowIntent,
    host_config: Option<HostConfig>,
    status: Option<quilt::lineage::InstalledPackageStatus>,
) -> Result<quilt::PublishOutcome, Error> {
    debug!(
        "Publishing the package.\nNamespace: {},\nmessage: {},\nuser_meta: {},\nworkflow: {:?}",
        namespace, message, metadata, workflow
    );
    let metadata = parse_metadata(metadata)?;

    let installed_package = model
        .get_installed_package(&namespace)
        .await?
        .ok_or_else(|| Error::from(quilt::InstallPackageError::NotInstalled(namespace)))?;

    let workflow = model.resolve_workflow(&installed_package, workflow).await?;
    model
        .package_publish(
            &installed_package,
            message.to_string(),
            metadata,
            workflow,
            host_config,
            status,
        )
        .await
}

pub async fn package_pull(
    model: &impl QuiltModel,
    namespace: &quilt_uri::Namespace,
    host_config: Option<HostConfig>,
) -> Result<(), Error> {
    let installed_package = model
        .get_installed_package(namespace)
        .await?
        .unwrap_or_else(|| panic!("Package {namespace} not found"));
    model.package_pull(&installed_package, host_config).await?;
    Ok(())
}

/// Set the package's remote. Returns `Some(reason)` when the remote was set but
/// the bucket's default workflow could not be resolved (best-effort path), so
/// the command layer can surface the warning to the user; `None` otherwise.
pub async fn set_remote(
    model: &impl QuiltModel,
    namespace: &quilt_uri::Namespace,
    origin: quilt_uri::Host,
    bucket: String,
    workflow: WorkflowIntent,
) -> Result<Option<String>, Error> {
    let installed_package = model
        .get_installed_package(namespace)
        .await?
        .ok_or_else(|| Error::from(quilt::InstallPackageError::NotInstalled(namespace.clone())))?;
    model
        .set_remote(&installed_package, origin, bucket, workflow)
        .await
}

pub async fn package_create(
    model: &impl QuiltModel,
    namespace: quilt_uri::Namespace,
    source: Option<PathBuf>,
    message: Option<String>,
) -> Result<quilt::InstalledPackage, Error> {
    model.package_create(namespace, source, message).await
}

pub async fn login(
    model: &impl QuiltModel,
    host: &quilt_uri::Host,
    code: String,
) -> Result<(), Error> {
    model
        .get_quilt()
        .lock()
        .await
        .get_remote()
        .login(host, code)
        .await?;
    Ok(())
}

pub async fn login_oauth(
    model: &impl QuiltModel,
    host: &quilt_uri::Host,
    params: quilt::auth::OAuthParams,
) -> Result<(), Error> {
    model
        .get_quilt()
        .lock()
        .await
        .get_remote()
        .login_oauth(host, params)
        .await?;
    Ok(())
}

/// Resolve a revision's manifest commit message by top-hash, without
/// installing it — the requested-side fill for the version-mismatch banner.
/// Returns `None` when the package has no remote to browse or the manifest
/// carries no message.
pub async fn revision_message(
    model: &impl QuiltModel,
    namespace: Namespace,
    hash: String,
) -> Result<Option<String>, Error> {
    let installed = model
        .get_installed_package(&namespace)
        .await?
        .ok_or_else(|| Error::from(quilt::InstallPackageError::NotInstalled(namespace.clone())))?;
    let lineage = model.get_installed_package_lineage(&installed).await?;
    let Some(remote) = lineage.remote_uri else {
        return Ok(None);
    };
    let requested_uri = ManifestUri { hash, ..remote };
    let manifest = model.browse_remote_manifest(&requested_uri).await?;
    Ok(manifest.header.message)
}

pub async fn get_or_register_client(
    model: &impl QuiltModel,
    host: &quilt_uri::Host,
    redirect_uri: &str,
) -> Result<String, Error> {
    let client = model
        .get_quilt()
        .lock()
        .await
        .get_remote()
        .get_or_register_client(host, redirect_uri)
        .await?;
    Ok(client.client_id)
}

#[cfg(test)]
mod tests {
    use super::*;

    use mockall::predicate::{always, eq};

    use crate::model::MockQuiltModel;
    use crate::model::mocks;

    #[tokio::test]
    async fn revision_message_browses_requested_hash() {
        let mut model = mocks::create();
        mocks::mock_remote_package_different_version(&mut model);
        let ns = Namespace::try_from("test/package").unwrap();

        let msg = super::revision_message(&model, ns, "requestedhash0000".to_string())
            .await
            .unwrap();

        // The mock's browse_remote_manifest returns the remote manifest.
        assert_eq!(msg, mocks::create_remote_manifest().header.message.clone());
    }

    /// A workflow-rejection error as `quilt-rs` surfaces it from the
    /// commit/push flow. The single `MessageRequired` violation gives a
    /// deterministic Display we can assert names the failed rule.
    fn workflow_rejection() -> Error {
        Error::from(quilt::Error::from(
            quilt::workflow::WorkflowValidationError::Rejected(vec![
                quilt::workflow::RuleViolation::MessageRequired,
            ]),
        ))
    }

    fn fake_publish_outcome(namespace: &quilt_uri::Namespace) -> quilt::PublishOutcome {
        quilt::PublishOutcome::PushedOnly(quilt::PushOutcome {
            manifest_uri: quilt_uri::ManifestUri {
                bucket: "bucket".to_string(),
                namespace: namespace.clone(),
                hash: "h1".to_string(),
                origin: None,
            },
            certified_latest: true,
        })
    }

    /// A `MockQuiltModel` that asserts `publish_with_settings` resolves the
    /// workflow with exactly `expected` and then publishes once.
    fn model_expecting_intent(expected: WorkflowIntent) -> MockQuiltModel {
        let mut model = MockQuiltModel::new();
        model.expect_get_installed_package().returning(|_| {
            Ok(Some(
                quilt::LocalDomain::new(std::path::PathBuf::new())
                    .create_installed_package(("acme", "demo").into())
                    .unwrap(),
            ))
        });
        model
            .expect_resolve_workflow()
            .times(1)
            .with(always(), eq(expected))
            .returning(|_, _| Ok(None));
        model
            .expect_package_publish()
            .times(1)
            .returning(|_, _, _, _, _, _| Ok(fake_publish_outcome(&("acme", "demo").into())));
        model
    }

    #[tokio::test]
    async fn publish_with_settings_empty_maps_to_bucket_default() -> Result<(), Error> {
        let namespace: quilt_uri::Namespace = ("acme", "demo").into();
        let model = model_expecting_intent(WorkflowIntent::BucketDefault);
        let settings = PublishSettings::default();
        let status = quilt::lineage::InstalledPackageStatus::default();
        publish_with_settings(&model, &namespace, &settings, status).await?;
        Ok(())
    }

    #[tokio::test]
    async fn publish_with_settings_named_workflow() -> Result<(), Error> {
        let namespace: quilt_uri::Namespace = ("acme", "demo").into();
        let model = model_expecting_intent(WorkflowIntent::Named("x".to_string()));
        let settings = PublishSettings {
            default_workflow: Some("x".to_string()),
            ..PublishSettings::default()
        };
        let status = quilt::lineage::InstalledPackageStatus::default();
        publish_with_settings(&model, &namespace, &settings, status).await?;
        Ok(())
    }

    #[tokio::test]
    async fn publish_with_settings_whitespace_maps_to_bucket_default() -> Result<(), Error> {
        let namespace: quilt_uri::Namespace = ("acme", "demo").into();
        let model = model_expecting_intent(WorkflowIntent::BucketDefault);
        let settings = PublishSettings {
            default_workflow: Some("   ".into()),
            ..PublishSettings::default()
        };
        let status = quilt::lineage::InstalledPackageStatus::default();
        publish_with_settings(&model, &namespace, &settings, status).await?;
        Ok(())
    }

    /// A workflow-rejection error from `quilt-rs` must propagate through the
    /// ops layer un-swallowed, carrying the validator's message that names the
    /// failed rule. The commit dialog's "Commit and Push" primary routes
    /// through `package_publish`, and the command wrapper embeds this error via
    /// `{err}` into the dialog's error notification, so a generic "operation
    /// failed" would hide what the user must fix. (The plain-commit path shares
    /// the same `?`-propagation of the trait `package_commit` error; it is not
    /// unit-tested here because its concrete `resolve_workflow` call runs before
    /// the mock and needs real storage.)
    #[tokio::test]
    async fn package_publish_surfaces_workflow_rejection() -> Result<(), Error> {
        let namespace: quilt_uri::Namespace = ("acme", "demo").into();
        let mut model = MockQuiltModel::new();
        model.expect_get_installed_package().returning(|_| {
            Ok(Some(
                quilt::LocalDomain::new(std::path::PathBuf::new())
                    .create_installed_package(("acme", "demo").into())
                    .unwrap(),
            ))
        });
        model.expect_resolve_workflow().returning(|_, _| Ok(None));
        model
            .expect_package_publish()
            .times(1)
            .returning(|_, _, _, _, _, _| Err(workflow_rejection()));

        // `PublishOutcome` is not `Debug`, so match rather than `expect_err`.
        let Err(err) = package_publish(
            &model,
            namespace,
            "msg",
            "",
            WorkflowIntent::BucketDefault,
            None,
            None,
        )
        .await
        else {
            panic!("publish should fail on workflow rejection");
        };
        assert!(
            err.to_string().contains("a commit message is required"),
            "surfaced error must name the failed rule, got: {err}"
        );
        Ok(())
    }

    /// `set_remote` must forward the caller's `WorkflowIntent` verbatim to the
    /// model layer (and thence to the package), so the popup's choice governs
    /// the recommit.
    #[tokio::test]
    async fn set_remote_forwards_workflow_intent() -> Result<(), Error> {
        let namespace: quilt_uri::Namespace = ("acme", "demo").into();
        let intent = WorkflowIntent::Named("nightly".to_string());
        let mut model = MockQuiltModel::new();
        model.expect_get_installed_package().returning(|_| {
            Ok(Some(
                quilt::LocalDomain::new(std::path::PathBuf::new())
                    .create_installed_package(("acme", "demo").into())
                    .unwrap(),
            ))
        });
        model
            .expect_set_remote()
            .times(1)
            .with(
                always(),
                always(),
                eq("my-bucket".to_string()),
                eq(intent.clone()),
            )
            .returning(|_, _, _, _| Ok(None));

        let origin: quilt_uri::Host = "test.quilt.dev".parse().unwrap();
        set_remote(&model, &namespace, origin, "my-bucket".to_string(), intent).await?;
        Ok(())
    }
}
