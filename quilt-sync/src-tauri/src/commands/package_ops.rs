//! Package lifecycle commands: commit, push, publish, pull, create,
//! uninstall, remote configuration, and quiltignore edits.

use std::io::Write;
use std::path::PathBuf;
use std::str::FromStr;

use serde::Serialize;

use quilt_rs::io::remote::WorkflowIntent;

use quilt_uri::Host;

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
    // Box the publish future: it exceeds the 18.5 KiB `large_futures`
    // budget (see `clippy.toml`) — commit + push chained in one state
    // machine.
    let result = Box::pin(package_publish_command(&m, &settings, &namespace)).await;
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

#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(
    tag = "kind",
    rename_all = "camelCase",
    rename_all_fields = "camelCase"
)]
pub enum RemoteBanner {
    /// A different revision than the one requested by the deep link is
    /// already installed; the working copy was not switched. Carries the
    /// requested revision's own remote (bucket + origin) so the UI fetches
    /// its message from where it actually lives, not from the installed
    /// package's remote.
    DifferentVersion {
        requested_hash: String,
        requested_bucket: String,
        requested_origin: Option<Host>,
        installed_hash: String,
    },
    /// The package is installed locally without a remote origin.
    LocalOnly,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RemotePackageResult {
    pub namespace: String,
    /// `None` when the requested revision was installed/opened normally.
    pub banner: Option<RemoteBanner>,
}

fn banner_for_outcome(
    outcome: &model::InstallOutcome,
    requested: &quilt_uri::S3PackageUri,
) -> Option<RemoteBanner> {
    match outcome {
        model::InstallOutcome::DifferentVersion {
            requested_hash,
            installed_hash,
        } => Some(RemoteBanner::DifferentVersion {
            requested_hash: requested_hash.clone(),
            requested_bucket: requested.bucket.clone(),
            requested_origin: requested.catalog.clone(),
            installed_hash: installed_hash.clone(),
        }),
        model::InstallOutcome::LocalOnly => Some(RemoteBanner::LocalOnly),
        model::InstallOutcome::Installed => None,
    }
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

    let outcome = model::install_package_only(&*m, &s3_uri)
        .await
        .map_err(|e| e.to_frontend_string())?;

    // Preserve the Installed side effect: if the URI names a path, install
    // it and open it in the default application.
    if let model::InstallOutcome::Installed = outcome
        && let Some(ref path) = s3_uri.path
    {
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
        banner: banner_for_outcome(&outcome, &s3_uri),
    })
}

/// Fetch a requested revision's manifest commit message by top-hash, without
/// installing it. Lazily backs Phase 2 of the version-mismatch banner: the
/// banner shows immediately with hashes, then the UI calls this to fill in
/// the requested side's message once it resolves.
#[tauri::command]
pub async fn get_revision_message(
    m: tauri::State<'_, model::Model>,
    bucket: String,
    namespace: String,
    hash: String,
    catalog: Option<String>,
) -> Result<Option<String>, String> {
    let namespace = quilt_uri::Namespace::try_from(namespace)
        .map_err(|e: quilt_uri::UriError| e.to_string())?;
    let origin = catalog
        .map(|c| Host::from_str(&c))
        .transpose()
        .map_err(|e: quilt_uri::UriError| e.to_string())?;
    let manifest_uri = quilt_uri::ManifestUri {
        origin,
        bucket,
        namespace,
        hash,
    };
    model::revision_message(&*m, manifest_uri)
        .await
        .map_err(|e| e.to_frontend_string())
}

#[cfg(test)]
mod tests {
    /// The requested revision's deep-link URI — its own bucket + catalog,
    /// deliberately different from anything installed.
    fn requested_uri() -> quilt_uri::S3PackageUri {
        quilt_uri::S3PackageUri {
            catalog: Some("cat.example.com".parse().unwrap()),
            bucket: "reqbucket".to_string(),
            namespace: ("foo", "bar").into(),
            revision: quilt_uri::RevisionPointer::Hash("aaaa1111".to_string()),
            path: None,
        }
    }

    #[test]
    fn banner_serializes_expected_json_shape() {
        let dv = super::RemoteBanner::DifferentVersion {
            requested_hash: "aaaa1111".to_string(),
            requested_bucket: "reqbucket".to_string(),
            requested_origin: Some("cat.example.com".parse().unwrap()),
            installed_hash: "bbbb2222".to_string(),
        };
        assert_eq!(
            serde_json::to_string(&dv).unwrap(),
            r#"{"kind":"differentVersion","requestedHash":"aaaa1111","requestedBucket":"reqbucket","requestedOrigin":"cat.example.com","installedHash":"bbbb2222"}"#
        );
        assert_eq!(
            serde_json::to_string(&super::RemoteBanner::LocalOnly).unwrap(),
            r#"{"kind":"localOnly"}"#
        );
    }

    #[test]
    fn banner_maps_different_version() {
        let outcome = crate::model::InstallOutcome::DifferentVersion {
            requested_hash: "aaaa1111".to_string(),
            installed_hash: "bbbb2222".to_string(),
        };
        assert_eq!(
            super::banner_for_outcome(&outcome, &requested_uri()),
            Some(super::RemoteBanner::DifferentVersion {
                requested_hash: "aaaa1111".to_string(),
                requested_bucket: "reqbucket".to_string(),
                requested_origin: Some("cat.example.com".parse().unwrap()),
                installed_hash: "bbbb2222".to_string(),
            })
        );
    }

    #[test]
    fn banner_maps_local_only_and_installed() {
        assert_eq!(
            super::banner_for_outcome(&crate::model::InstallOutcome::LocalOnly, &requested_uri()),
            Some(super::RemoteBanner::LocalOnly)
        );
        assert_eq!(
            super::banner_for_outcome(&crate::model::InstallOutcome::Installed, &requested_uri()),
            None
        );
    }
}
