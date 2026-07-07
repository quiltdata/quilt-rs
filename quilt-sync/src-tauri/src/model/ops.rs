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
    workflow: Option<String>,
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

    let intent = match workflow {
        Some(id) => WorkflowIntent::Named(id),
        None => WorkflowIntent::NoWorkflow,
    };
    let workflow = installed_package.resolve_workflow(intent).await?;
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
/// Shared entry point for both the manual Commit & Push command and the
/// autosync watcher tick — keep them in lockstep so a change to publish
/// settings (new placeholder, new field) applies identically regardless
/// of who triggered the publish. Returns the outcome paired with the
/// rendered commit message; the autosync tick needs the message string
/// for its `autosync-published` event, the manual command can discard it.
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
    let workflow = settings.default_workflow.clone();
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
    workflow: Option<String>,
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

pub async fn set_remote(
    model: &impl QuiltModel,
    namespace: &quilt_uri::Namespace,
    origin: quilt_uri::Host,
    bucket: String,
) -> Result<(), Error> {
    let installed_package = model
        .get_installed_package(namespace)
        .await?
        .ok_or_else(|| Error::from(quilt::InstallPackageError::NotInstalled(namespace.clone())))?;
    model.set_remote(&installed_package, origin, bucket).await?;
    Ok(())
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
