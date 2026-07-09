//! Package lifecycle commands: commit, push, publish, pull, create,
//! uninstall, remote configuration, and quiltignore edits.

use std::io::Write;
use std::path::PathBuf;
use std::str::FromStr;

use serde::Serialize;

use quilt_rs::io::remote::WorkflowIntent;

use crate::Error;
use crate::autopull::Watcher;
use crate::model;
use crate::model::QuiltModel;
use crate::notify::Notify;
use crate::publish_settings::SharedPublishSettings;
use crate::quilt;
use crate::telemetry::MixpanelEvent;

async fn package_commit_command(
    m: &model::Model,
    namespace: &str,
    message: &str,
    metadata: &str,
    workflow: WorkflowIntent,
) -> Result<quilt_uri::Namespace, Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    if message.is_empty() {
        return Err(Error::Commit("Message is required".to_string()));
    }

    model::package_commit(m, namespace.clone(), message, metadata, workflow, None).await?;
    Ok(namespace)
}

#[tauri::command]
pub async fn package_commit(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    watcher: tauri::State<'_, Watcher>,
    namespace: String,
    message: String,
    metadata: String,
    workflow: WorkflowIntent,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackageCommitted).await;

    let msg_init = format!("Committing package {namespace}");
    let msg_ok = format!("Successfully committed {namespace}");
    let msg_err = |err: &Error| format!("Failed to commit: {err}");

    let result = package_commit_command(&m, &namespace, &message, &metadata, workflow).await;
    if let Ok(ns) = &result {
        watcher.clear_paused(ns).await;
    }
    Notify::new(msg_init).map(result.map(|_| ()), msg_ok, msg_err)
}

async fn certify_latest_command(m: &model::Model, namespace: &str) -> Result<(), Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    model::package_revision_certify_latest(m, namespace.clone()).await?;
    Ok(())
}

#[tauri::command]
pub async fn certify_latest(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::LatestCertified).await;

    let msg_init = format!("Certifying latest for {namespace}");
    let msg_ok = format!("Successfully certified latest for {namespace}");
    let msg_err = |err: &Error| format!("Failed to certify latest: {err}");

    Notify::new(msg_init).map(
        certify_latest_command(&m, &namespace).await,
        msg_ok,
        msg_err,
    )
}

async fn reset_local_command(
    m: &model::Model,
    namespace: &str,
) -> Result<quilt_uri::Namespace, Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    model::package_revision_reset_local(m, namespace.clone()).await?;
    Ok(namespace)
}

#[tauri::command]
pub async fn reset_local(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    watcher: tauri::State<'_, Watcher>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::LocalReset).await;

    let msg_init = format!("Resetting local for {namespace}");
    let msg_ok = format!("Successfully reset local for {namespace}");
    let msg_err = |err: &Error| format!("Failed to reset local: {err}");

    let result = reset_local_command(&m, &namespace).await;
    if let Ok(ns) = &result {
        watcher.clear_paused(ns).await;
    }
    Notify::new(msg_init).map(result.map(|_| ()), msg_ok, msg_err)
}

async fn package_push_command(
    m: &model::Model,
    namespace: &str,
) -> Result<(quilt_uri::Namespace, quilt::PushOutcome), Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    let outcome = model::package_push(m, &namespace, None).await?;
    Ok((namespace, outcome))
}

#[tauri::command]
pub async fn package_push(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    watcher: tauri::State<'_, Watcher>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackagePushed).await;

    let msg_init = format!("Pushing package {namespace}");

    let result = package_push_command(&m, &namespace).await;
    if let Ok((ns, _)) = &result {
        watcher.clear_paused(ns).await;
    }
    // TODO: push-not-certified should be surfaced as a warning, not a success.
    // Currently both outcomes go through the success path because converting to
    // Err skips on_done()/refetch and leaves the UI stale.
    let msg_ok = match &result {
        Ok((_, outcome)) if outcome.certified_latest => {
            format!("Successfully pushed package {namespace}")
        }
        Ok(_) => {
            format!("Pushed {namespace}, but could not update latest: remote has newer changes")
        }
        _ => String::new(),
    };
    let msg_err = |err: &Error| format!("Failed to push package: {err}");

    Notify::new(msg_init).map(result.map(|_| ()), msg_ok, msg_err)
}

async fn package_publish_command(
    m: &model::Model,
    settings: &SharedPublishSettings,
    namespace: &str,
) -> Result<(quilt_uri::Namespace, quilt::PublishOutcome), Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    let installed = m
        .get_installed_package(&namespace)
        .await?
        .ok_or_else(|| Error::from(quilt::InstallPackageError::NotInstalled(namespace.clone())))?;
    let status = m.get_installed_package_status(&installed, None).await?;

    let settings = settings.read().await.clone();
    let (outcome, _message) =
        model::publish_with_settings(m, &namespace, &settings, status).await?;
    Ok((namespace, outcome))
}

#[tauri::command]
pub async fn package_publish(
    m: tauri::State<'_, model::Model>,
    settings: tauri::State<'_, SharedPublishSettings>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    watcher: tauri::State<'_, Watcher>,
    namespace: String,
) -> Result<String, String> {
    let msg_init = format!("Publishing package {namespace}");
    let result = package_publish_command(&m, &settings, &namespace).await;
    if let Ok((ns, _)) = &result {
        watcher.clear_paused(ns).await;
    }

    if let Ok((_, outcome)) = &result {
        tracing.track(MixpanelEvent::PackagePublished).await;
        if matches!(outcome, quilt::PublishOutcome::CommittedAndPushed(_)) {
            tracing.track(MixpanelEvent::PackageCommitted).await;
        }
        tracing.track(MixpanelEvent::PackagePushed).await;
    }

    let msg_ok = match &result {
        Ok((_, outcome)) if outcome.push().certified_latest => {
            format!("Successfully published package {namespace}")
        }
        Ok(_) => {
            format!("Published {namespace}, but could not update latest: remote has newer changes")
        }
        _ => String::new(),
    };
    // TODO: route `Error` through `to_frontend_string()` so that
    // `login_required` / `setup_required` publish-time errors can trigger the
    // `/login` and `/setup` redirects in `ui::error_handler::handle_or_display`
    // instead of surfacing as a plain toast. This requires `make_action` to
    // parse the JSON envelope (or the Tauri command to bypass `Notify` for
    // these variants); both are out of scope here.
    let msg_err = |err: &Error| format!("Failed to publish package: {err}");

    Notify::new(msg_init).map(result.map(|_| ()), msg_ok, msg_err)
}

async fn package_commit_and_push_command(
    m: &model::Model,
    namespace: &str,
    message: &str,
    metadata: &str,
    workflow: WorkflowIntent,
) -> Result<(quilt_uri::Namespace, quilt::PublishOutcome), Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    if message.trim().is_empty() {
        return Err(Error::Commit("Message is required".to_string()));
    }
    let outcome = model::package_publish(
        m,
        namespace.clone(),
        message,
        metadata,
        workflow,
        None,
        None,
    )
    .await?;
    Ok((namespace, outcome))
}

#[tauri::command]
pub async fn package_commit_and_push(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    watcher: tauri::State<'_, Watcher>,
    namespace: String,
    message: String,
    metadata: String,
    workflow: WorkflowIntent,
) -> Result<String, String> {
    let msg_init = format!("Publishing package {namespace}");
    let result =
        package_commit_and_push_command(&m, &namespace, &message, &metadata, workflow).await;
    if let Ok((ns, _)) = &result {
        watcher.clear_paused(ns).await;
    }

    if let Ok((_, outcome)) = &result {
        tracing.track(MixpanelEvent::PackagePublished).await;
        if matches!(outcome, quilt::PublishOutcome::CommittedAndPushed(_)) {
            tracing.track(MixpanelEvent::PackageCommitted).await;
        }
        tracing.track(MixpanelEvent::PackagePushed).await;
    }

    let msg_ok = match &result {
        Ok((_, outcome)) if outcome.push().certified_latest => {
            format!("Successfully published package {namespace}")
        }
        Ok(_) => {
            format!("Published {namespace}, but could not update latest: remote has newer changes")
        }
        _ => String::new(),
    };
    let msg_err = |err: &Error| format!("Failed to publish package: {err}");

    Notify::new(msg_init).map(result.map(|_| ()), msg_ok, msg_err)
}

async fn package_pull_command(
    m: &model::Model,
    namespace: &str,
) -> Result<quilt_uri::Namespace, Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    model::package_pull(m, &namespace, None).await?;
    Ok(namespace)
}

#[tauri::command]
pub async fn package_pull(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    watcher: tauri::State<'_, Watcher>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackagePulled).await;

    let msg_init = format!("Pulling package {namespace}");
    let msg_ok = format!("Successfully pulled package {namespace}");
    let msg_err = |err: &Error| format!("Failed to pull package: {err}");

    let result = package_pull_command(&m, &namespace).await;
    if let Ok(ns) = &result {
        watcher.clear_paused(ns).await;
    }
    Notify::new(msg_init).map(result.map(|_| ()), msg_ok, msg_err)
}

async fn package_uninstall_command(m: &model::Model, namespace: &str) -> Result<(), Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    model::package_uninstall(m, namespace.clone()).await?;
    Ok(())
}

#[tauri::command]
pub async fn package_uninstall(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackageUninstalled).await;

    let msg_init = format!("Uninstalling package {namespace}");
    let msg_ok = format!("Successfully uninstalled package {namespace}");
    let msg_err = |err: &Error| format!("Failed to uninstall package: {err}");

    Notify::new(msg_init).map(
        package_uninstall_command(&m, &namespace).await,
        msg_ok,
        msg_err,
    )
}

/// Typed response for the `set_remote` command. `resolution_warning` is
/// `Some(reason)` when the remote was set but the bucket's default workflow
/// could not be resolved (best-effort path) — the UI raises a warning notice
/// rather than a plain success. A typed struct keeps the Tauri boundary
/// self-describing instead of overloading the success string.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SetRemoteResponse {
    pub message: String,
    pub resolution_warning: Option<String>,
}

async fn set_remote_command(
    m: &model::Model,
    namespace: &str,
    origin: &str,
    bucket: &str,
    workflow: WorkflowIntent,
) -> Result<(quilt_uri::Namespace, Option<String>), Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    let origin = quilt_uri::Host::from_str(origin)?;
    let warning = model::set_remote(m, &namespace, origin, bucket.to_string(), workflow).await?;
    Ok((namespace, warning))
}

#[tauri::command]
pub async fn set_remote(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    watcher: tauri::State<'_, Watcher>,
    namespace: String,
    origin: String,
    bucket: String,
    workflow: WorkflowIntent,
) -> Result<SetRemoteResponse, String> {
    tracing.track(MixpanelEvent::RemoteSet).await;

    // `Notify::new` logs the init line; on success/failure we log explicitly so
    // the success payload can be the typed struct rather than a bare string.
    Notify::new(format!("Setting remote for {namespace}"));
    match set_remote_command(&m, &namespace, &origin, &bucket, workflow).await {
        Ok((ns, resolution_warning)) => {
            watcher.clear_paused(&ns).await;
            let message = format!("Successfully set remote for {namespace}");
            ::tracing::debug!("{message}");
            Ok(SetRemoteResponse {
                message,
                resolution_warning,
            })
        }
        Err(err) => {
            let msg = format!("Failed to set remote: {err}");
            ::tracing::error!("{msg}");
            Err(msg)
        }
    }
}

async fn package_create_command(
    m: &model::Model,
    namespace: &str,
    source: Option<String>,
    message: Option<String>,
) -> Result<(), Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    let source = source.map(PathBuf::from);
    model::package_create(m, namespace, source, message).await?;
    Ok(())
}

#[tauri::command]
pub async fn package_create(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
    source: Option<String>,
    message: Option<String>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackageCreated).await;

    let msg_init = format!("Creating package {namespace}");
    let msg_ok = format!("Successfully created package {namespace}");
    let msg_err = |err: &Error| format!("Failed to create package: {err}");

    Notify::new(msg_init).map(
        package_create_command(&m, &namespace, source, message).await,
        msg_ok,
        msg_err,
    )
}

async fn package_install_paths_command(
    m: &model::Model,
    uri: &str,
    paths: &[String],
) -> Result<(), Error> {
    let uri = quilt_uri::S3PackageUri::try_from(uri)?;
    let paths: Vec<PathBuf> = paths.iter().map(PathBuf::from).collect();
    model::install_paths_only(m, &uri.namespace, paths).await?;
    Ok(())
}

#[tauri::command]
pub async fn package_install_paths(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    uri: String,
    paths: Vec<String>,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::PackageInstalled).await;

    let msg_init = format!("Installing paths from {uri}");
    let msg_ok = format!("Successfully installed {} paths", paths.len());
    let msg_err = |err: &Error| format!("Failed to install paths: {err}");

    Notify::new(msg_init).map(
        package_install_paths_command(&m, &uri, &paths).await,
        msg_ok,
        msg_err,
    )
}

async fn add_to_quiltignore_command(
    m: &model::Model,
    namespace: &str,
    pattern: &str,
) -> Result<(), Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    let package_home = m.package_home(&namespace).await?;
    let quiltignore_path = package_home.join(".quiltignore");

    // Take only the first line to prevent injecting multiple rules
    let pattern = pattern.lines().next().unwrap_or(pattern);

    // Read first to check trailing newline, before opening for append
    let needs_newline = std::fs::read_to_string(&quiltignore_path)
        .is_ok_and(|s| !s.is_empty() && !s.ends_with('\n'));

    let mut file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&quiltignore_path)
        .map_err(|e| format!("Failed to open .quiltignore: {e}"))?;

    if needs_newline {
        writeln!(file).map_err(|e| e.to_string())?;
    }
    writeln!(file, "{pattern}").map_err(|e| e.to_string())?;

    Ok(())
}

#[tauri::command]
pub async fn add_to_quiltignore(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    namespace: String,
    pattern: String,
) -> Result<String, String> {
    tracing.track(MixpanelEvent::QuiltignorePatternAdded).await;

    let msg_init = format!("Adding {pattern} to .quiltignore");
    let msg_ok = format!("Added {pattern} to .quiltignore");
    let msg_err = |err: &Error| format!("Failed to update .quiltignore: {err}");

    Notify::new(msg_init).map(
        add_to_quiltignore_command(&m, &namespace, &pattern).await,
        msg_ok,
        msg_err,
    )
}

#[tauri::command]
pub async fn test_quiltignore_pattern(pattern: String, path: String) -> Result<bool, String> {
    Ok(quilt::junk::pattern_matches(&pattern, &path))
}

// ── Remote package handling for Leptos UI ──

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePackageResult {
    pub namespace: String,
    pub notification: Option<String>,
}

#[tauri::command]
pub async fn handle_remote_package(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    uri: String,
) -> Result<RemotePackageResult, String> {
    let s3_uri: quilt_uri::S3PackageUri = uri
        .parse()
        .map_err(|e: quilt_uri::UriError| e.to_string())?;
    let namespace = s3_uri.namespace.to_string();
    let _ = &tracing;

    match model::install_package_only(&*m, &s3_uri)
        .await
        .map_err(|e| e.to_frontend_string())?
    {
        model::InstallOutcome::DifferentVersion {
            requested_hash,
            installed_hash,
        } => {
            let short_requested: String = requested_hash.chars().take(8).collect();
            let short_installed: String = installed_hash.chars().take(8).collect();
            let notification = rust_i18n::t!(
                "installed_package_notification.different_version",
                requested => short_requested,
                installed => short_installed,
            )
            .to_string();
            Ok(RemotePackageResult {
                namespace,
                notification: Some(notification),
            })
        }
        model::InstallOutcome::LocalOnly => {
            let notification =
                rust_i18n::t!("installed_package_notification.local_only").to_string();
            Ok(RemotePackageResult {
                namespace,
                notification: Some(notification),
            })
        }
        model::InstallOutcome::Installed => {
            // If URI has a path, install it and open in default application
            if let Some(ref path) = s3_uri.path {
                let installed_package = m
                    .get_installed_package(&s3_uri.namespace)
                    .await
                    .map_err(|e| e.to_frontend_string())?
                    .ok_or_else(|| format!("Package {namespace} is not installed"))?;
                if !m
                    .is_path_installed(&installed_package, path)
                    .await
                    .map_err(|e| e.to_frontend_string())?
                {
                    m.package_install_paths(&installed_package, std::slice::from_ref(path))
                        .await
                        .map_err(|e| e.to_frontend_string())?;
                }
                m.open_in_default_application(&s3_uri.namespace, path)
                    .await
                    .map_err(|e| e.to_frontend_string())?;
            }
            Ok(RemotePackageResult {
                namespace,
                notification: None,
            })
        }
    }
}
