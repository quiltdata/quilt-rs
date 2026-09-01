//! Package lifecycle commands: commit, push, publish, pull, create,
//! uninstall, remote configuration, and quiltignore edits.

use std::io::Write;
use std::path::PathBuf;
use std::str::FromStr;

use serde::Serialize;

use quilt_rs::io::remote::WorkflowIntent;

use quilt_uri::{Host, S3PackageUri};

use crate::Error;
use crate::autopull::Watcher;
use crate::experimental_settings::ExperimentalSettings;
use crate::experimental_settings::SharedExperimentalSettings;
use crate::model;
use crate::model::QuiltModel;
use crate::notify::Notify;
use crate::publish_settings::SharedPublishSettings;
use crate::quilt;
use crate::quilt::flow::PullOutcome;
use crate::quilt::lineage::SyncScope;
use crate::telemetry::MixpanelEvent;
use crate::telemetry::event::{PackageEvent, RemotePackageEvent};

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
#[allow(
    clippy::too_many_arguments,
    reason = "three of these are Tauri state injections a caller never passes; the rest are the revision's own fields plus the telemetry context"
)]
pub async fn package_commit(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    watcher: tauri::State<'_, Watcher>,
    namespace: String,
    message: String,
    metadata: String,
    workflow: WorkflowIntent,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    let msg_init = format!("Committing package {namespace}");
    let msg_ok = format!("Successfully committed {namespace}");
    let msg_err = |err: &Error| format!("Failed to commit: {err}");

    let result = package_commit_command(&m, &namespace, &message, &metadata, workflow).await;
    if let Ok(ns) = &result {
        watcher.clear_paused(ns).await;
    }
    Notify::new(msg_init)
        .on_success(
            &tracing,
            MixpanelEvent::PackageCommitted(PackageEvent::for_uri(uri.as_ref())),
        )
        .map(result.map(|_| ()), msg_ok, msg_err)
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
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    let msg_init = format!("Certifying latest for {namespace}");
    let msg_ok = format!("Successfully certified latest for {namespace}");
    let msg_err = |err: &Error| format!("Failed to certify latest: {err}");

    Notify::new(msg_init)
        .on_success(
            &tracing,
            MixpanelEvent::LatestCertified(RemotePackageEvent::for_uri(uri.as_ref())),
        )
        .map(
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
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    let msg_init = format!("Resetting local for {namespace}");
    let msg_ok = format!("Successfully reset local for {namespace}");
    let msg_err = |err: &Error| format!("Failed to reset local: {err}");

    let result = reset_local_command(&m, &namespace).await;
    if let Ok(ns) = &result {
        watcher.clear_paused(ns).await;
    }
    Notify::new(msg_init)
        .on_success(
            &tracing,
            MixpanelEvent::LocalReset(RemotePackageEvent::for_uri(uri.as_ref())),
        )
        .map(result.map(|_| ()), msg_ok, msg_err)
}

/// The user-visible text for a failed write, given the action that failed
/// (`"push package"`, `"publish package"`) and the error it failed with.
///
/// Write permission is invisible until the write happens: the role-scoped
/// bucket list is read-presence only and never tells `READ` apart from
/// `READ_WRITE`, so a role that can read a bucket but not write it looks
/// perfectly healthy right up to the push. When storage refuses with
/// `AccessDenied`, name the role as the cause and the remedy — a raw
/// storage error gives the user nothing to connect the failure back to the
/// role they picked.
fn write_failure_message(action: &str, err: &Error) -> String {
    if err.is_access_denied() {
        "Current role can't write here — switch role".to_string()
    } else if err.is_invalid_credentials() {
        // This path reports through a toast, not the error page
        // `to_frontend_string` feeds, so it cannot navigate to `/login`. The
        // message carries the remedy instead.
        match err.invalid_credentials_host() {
            Some(host) => format!("Your session for {host} has expired — sign in again"),
            None => "AWS credentials in ~/.aws/credentials are invalid — update them".to_string(),
        }
    } else {
        format!("Failed to {action}: {err}")
    }
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
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
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
    let msg_err = |err: &Error| write_failure_message("push package", err);

    Notify::new(msg_init)
        .on_success(
            &tracing,
            MixpanelEvent::PackagePushed(RemotePackageEvent::for_uri(uri.as_ref())),
        )
        .map(result.map(|_| ()), msg_ok, msg_err)
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
    // Box the publish future — the commit+push state machine exceeds the
    // `large_futures` budget (see `clippy.toml`).
    let (outcome, _message) = Box::pin(model::publish_with_settings(
        m, &namespace, &settings, status,
    ))
    .await?;
    Ok((namespace, outcome))
}

#[tauri::command]
pub async fn package_publish(
    m: tauri::State<'_, model::Model>,
    settings: tauri::State<'_, SharedPublishSettings>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    watcher: tauri::State<'_, Watcher>,
    namespace: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    let msg_init = format!("Publishing package {namespace}");
    let result = package_publish_command(&m, &settings, &namespace).await;
    if let Ok((ns, _)) = &result {
        watcher.clear_paused(ns).await;
    }

    if let Ok((_, outcome)) = &result {
        tracing.track(MixpanelEvent::PackagePublished(
            RemotePackageEvent::for_uri(uri.as_ref()),
        ));
        if matches!(outcome, quilt::PublishOutcome::CommittedAndPushed(_)) {
            tracing.track(MixpanelEvent::PackageCommitted(PackageEvent::for_uri(
                uri.as_ref(),
            )));
        }
        tracing.track(MixpanelEvent::PackagePushed(RemotePackageEvent::for_uri(
            uri.as_ref(),
        )));
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
    let msg_err = |err: &Error| write_failure_message("publish package", err);

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
#[allow(
    clippy::too_many_arguments,
    reason = "three of these are Tauri state injections a caller never passes; the rest are the revision's own fields plus the telemetry context"
)]
pub async fn package_commit_and_push(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    watcher: tauri::State<'_, Watcher>,
    namespace: String,
    message: String,
    metadata: String,
    workflow: WorkflowIntent,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    let msg_init = format!("Publishing package {namespace}");
    let result =
        package_commit_and_push_command(&m, &namespace, &message, &metadata, workflow).await;
    if let Ok((ns, _)) = &result {
        watcher.clear_paused(ns).await;
    }

    if let Ok((_, outcome)) = &result {
        tracing.track(MixpanelEvent::PackagePublished(
            RemotePackageEvent::for_uri(uri.as_ref()),
        ));
        if matches!(outcome, quilt::PublishOutcome::CommittedAndPushed(_)) {
            tracing.track(MixpanelEvent::PackageCommitted(PackageEvent::for_uri(
                uri.as_ref(),
            )));
        }
        tracing.track(MixpanelEvent::PackagePushed(RemotePackageEvent::for_uri(
            uri.as_ref(),
        )));
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
    let msg_err = |err: &Error| write_failure_message("publish package", err);

    Notify::new(msg_init).map(result.map(|_| ()), msg_ok, msg_err)
}

async fn package_pull_command(
    m: &model::Model,
    namespace: &str,
    experimental: &ExperimentalSettings,
) -> Result<quilt_uri::Namespace, Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    model::package_pull(m, &namespace, None, experimental).await?;
    Ok(namespace)
}

/// Record whether this package keeps its whole contents.
///
/// Storage only — it writes the standing choice and returns. Catching up on
/// files already listed is the caller's separate `package_install_paths` call,
/// deliberately not folded in here: one of them is a preference and the other
/// downloads bytes, and a failure to fetch must not silently un-choose the mode.
#[tauri::command]
pub async fn package_set_sync_scope(
    m: tauri::State<'_, model::Model>,
    namespace: String,
    entire_package: bool,
) -> Result<(), String> {
    let namespace = quilt_uri::Namespace::try_from(namespace.as_str())
        .map_err(|e: quilt_uri::UriError| e.to_string())?;
    let scope = if entire_package {
        SyncScope::EntirePackage
    } else {
        SyncScope::IndividualFiles
    };
    let installed = m
        .get_installed_package(&namespace)
        .await
        .map_err(|e| e.to_frontend_string())?
        .ok_or_else(|| format!("Package {namespace} not found"))?;
    m.package_set_sync_scope(&installed, scope)
        .await
        .map_err(|e| e.to_frontend_string())
}

#[tauri::command]
pub async fn package_pull(
    m: tauri::State<'_, model::Model>,
    tracing: tauri::State<'_, crate::telemetry::Telemetry>,
    watcher: tauri::State<'_, Watcher>,
    experimental: tauri::State<'_, SharedExperimentalSettings>,
    namespace: String,
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    let msg_init = format!("Pulling package {namespace}");
    let msg_ok = format!("Successfully pulled package {namespace}");
    let msg_err = |err: &Error| format!("Failed to pull package: {err}");

    let experimental = experimental.read().await.clone();
    let result = package_pull_command(&m, &namespace, &experimental).await;
    if let Ok(ns) = &result {
        watcher.clear_paused(ns).await;
    }
    Notify::new(msg_init)
        .on_success(
            &tracing,
            MixpanelEvent::PackagePulled(RemotePackageEvent::for_uri(uri.as_ref())),
        )
        .map(result.map(|_| ()), msg_ok, msg_err)
}

async fn package_pull_outcome_command(
    m: &model::Model,
    namespace: &str,
) -> Result<PullOutcome, Error> {
    let namespace = quilt_uri::Namespace::try_from(namespace)?;
    let installed = m
        .get_installed_package(&namespace)
        .await?
        .ok_or_else(|| Error::from(quilt::InstallPackageError::NotInstalled(namespace.clone())))?;
    m.package_pull_outcome(&installed).await
}

/// Dry-run classifier for the two-phase Pull affordance: what would
/// [`package_pull`] do right now? The UI renders the behind-status banner
/// immediately from local data, then calls this to fill in the Pull button's
/// enabled state and copy once `latest` is cached. `PullOutcome` derives
/// `Serialize`, so it crosses the Tauri boundary directly.
#[tauri::command]
pub async fn package_pull_outcome(
    m: tauri::State<'_, model::Model>,
    namespace: String,
) -> Result<PullOutcome, String> {
    package_pull_outcome_command(&m, &namespace)
        .await
        .map_err(|e| e.to_string())
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
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    let msg_init = format!("Uninstalling package {namespace}");
    let msg_ok = format!("Successfully uninstalled package {namespace}");
    let msg_err = |err: &Error| format!("Failed to uninstall package: {err}");

    Notify::new(msg_init)
        .on_success(
            &tracing,
            MixpanelEvent::PackageUninstalled(PackageEvent::for_uri(uri.as_ref())),
        )
        .map(
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
    // The origin is this command's own argument: the remote being set.
    let origin_host = Host::from_str(&origin).ok();
    // `Notify::new` logs the init line; on success/failure we log explicitly so
    // the success payload can be the typed struct rather than a bare string.
    Notify::new(format!("Setting remote for {namespace}"));
    match set_remote_command(&m, &namespace, &origin, &bucket, workflow).await {
        Ok((ns, resolution_warning)) => {
            // Reported here rather than through `Notify::on_success`, because this
            // command returns a typed payload and so does not route its outcome
            // through `map`. Same rule, hand-applied: the success arm only.
            tracing.track(MixpanelEvent::RemoteSet(RemotePackageEvent::for_host(
                origin_host,
            )));
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
    // A package created here has no remote yet, so it belongs to no deployment.
    let msg_init = format!("Creating package {namespace}");
    let msg_ok = format!("Successfully created package {namespace}");
    let msg_err = |err: &Error| format!("Failed to create package: {err}");

    Notify::new(msg_init)
        .on_success(
            &tracing,
            MixpanelEvent::PackageCreated(PackageEvent::hostless()),
        )
        .map(
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
    // Installing names its package by URI, so the catalog is already in hand.
    let target = S3PackageUri::try_from(uri.as_str()).ok();
    let msg_init = format!("Installing paths from {uri}");
    let msg_ok = format!("Successfully installed {} paths", paths.len());
    let msg_err = |err: &Error| format!("Failed to install paths: {err}");

    Notify::new(msg_init)
        .on_success(
            &tracing,
            MixpanelEvent::PackageInstalled(RemotePackageEvent::for_uri(target.as_ref())),
        )
        .map(
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
    uri: Option<S3PackageUri>,
) -> Result<String, String> {
    let msg_init = format!("Adding {pattern} to .quiltignore");
    let msg_ok = format!("Added {pattern} to .quiltignore");
    let msg_err = |err: &Error| format!("Failed to update .quiltignore: {err}");

    Notify::new(msg_init)
        .on_success(
            &tracing,
            MixpanelEvent::QuiltignorePatternAdded(PackageEvent::for_uri(uri.as_ref())),
        )
        .map(
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
    use crate::Error;
    use crate::quilt;

    /// The error S3 returns when the active role cannot write a bucket. The
    /// upload path types the `AccessDenied` code distinctly instead of
    /// folding it into a generic put failure, which is what lets the push
    /// message tell a role problem apart from a broken upload.
    fn access_denied_error() -> Error {
        Error::Quilt(quilt::Error::S3(quilt::S3Error::new(
            quilt::S3ErrorKind::AccessDenied("s3://locked/x".to_string()),
        )))
    }

    /// A denied push must say the role cannot write here, not surface a raw
    /// storage error. Write denial is invisible until the push — the
    /// readable bucket list never distinguishes read from write.
    #[test]
    fn push_denial_reports_a_write_permission_problem() {
        let msg = super::write_failure_message("push package", &access_denied_error());

        assert!(msg.contains("can't write"), "got: {msg}");
        assert!(
            msg.contains("switch role"),
            "must name the remedy, got: {msg}"
        );
    }

    /// The denial message replaces the storage error rather than appending
    /// to it: an `AccessDenied` in a toast reads as a bug, not as a role the
    /// user can change.
    #[test]
    fn push_denial_does_not_leak_the_raw_storage_error() {
        let msg = super::write_failure_message("push package", &access_denied_error());

        assert!(!msg.contains("AccessDenied"), "got: {msg}");
        assert!(!msg.contains("S3 error"), "got: {msg}");
    }

    /// Publish shares the denial wording with push — both are writes, and
    /// the remedy is the same regardless of which button was pressed.
    #[test]
    fn publish_denial_reports_the_same_write_permission_problem() {
        assert_eq!(
            super::write_failure_message("publish package", &access_denied_error()),
            super::write_failure_message("push package", &access_denied_error()),
        );
    }

    fn expired_session_error() -> Error {
        Error::from(quilt::Error::S3(quilt::S3Error {
            host: Some("demo.quiltdata.com".parse().unwrap()),
            kind: quilt::S3ErrorKind::InvalidCredentials("ExpiredToken: nope".to_string()),
        }))
    }

    /// A write is where a stale session usually surfaces. The toast cannot
    /// navigate to `/login` the way the error page does, so the message itself
    /// has to carry the remedy and the host.
    #[test]
    fn push_with_an_expired_session_names_signing_in() {
        let msg = super::write_failure_message("push package", &expired_session_error());

        assert!(msg.contains("sign in again"), "got: {msg}");
        assert!(
            msg.contains("demo.quiltdata.com"),
            "must name the host, got: {msg}"
        );
        assert!(!msg.contains("ExpiredToken"), "raw SDK text leaked: {msg}");
        assert!(!msg.contains("S3 error"), "got: {msg}");
    }

    /// Ambient credentials have no deployment to sign in to, so the remedy is
    /// the file, not a login.
    #[test]
    fn push_with_invalid_local_credentials_names_the_file() {
        let err = Error::from(quilt::Error::S3(quilt::S3Error::new(
            quilt::S3ErrorKind::InvalidCredentials("InvalidAccessKeyId: nope".to_string()),
        )));
        let msg = super::write_failure_message("push package", &err);

        assert!(msg.contains("~/.aws/credentials"), "got: {msg}");
        assert!(
            !msg.contains("sign in"),
            "there is no stack to sign in to: {msg}"
        );
    }

    /// Every failure that is not a denial keeps its pre-existing text, so
    /// the new branch narrows the message set rather than replacing it.
    #[test]
    fn non_denial_failures_keep_their_original_message() {
        let err = Error::Commit("Message is required".to_string());

        assert_eq!(
            super::write_failure_message("push package", &err),
            "Failed to push package: Commit error: Message is required",
        );
        assert_eq!(
            super::write_failure_message("publish package", &err),
            "Failed to publish package: Commit error: Message is required",
        );
    }

    /// A storage failure that is *not* a denial must not be swept into the
    /// role message — the user would go switch roles over a transient
    /// upload error.
    #[test]
    fn non_denial_storage_failure_is_not_reported_as_a_role_problem() {
        let err = Error::Quilt(quilt::Error::S3(quilt::S3Error::new(
            quilt::S3ErrorKind::PutObject("SlowDown: throttled".to_string()),
        )));

        let msg = super::write_failure_message("push package", &err);

        assert!(!msg.contains("switch role"), "got: {msg}");
        assert!(msg.contains("SlowDown"), "got: {msg}");
    }

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

    /// The UI mirrors this exact externally-tagged JSON in
    /// `quilt_sync_ui::commands`'s `pull_outcome_wire_form_is_verbatim`. If the
    /// two drift, the two-phase Pull affordance silently misreads the dry-run
    /// outcome at the Tauri boundary.
    #[test]
    fn pull_outcome_wire_form_is_verbatim() {
        use std::path::PathBuf;

        use crate::quilt::flow::PullOutcome;
        assert_eq!(
            serde_json::to_string(&PullOutcome::UpToDate).unwrap(),
            r#""UpToDate""#
        );
        assert_eq!(
            serde_json::to_string(&PullOutcome::CleanUpdate).unwrap(),
            r#""CleanUpdate""#
        );
        assert_eq!(
            serde_json::to_string(&PullOutcome::KeepsLocalChanges {
                added: vec![PathBuf::from("a.txt")],
                modified: vec![],
                removed: vec![PathBuf::from("c.txt")],
            })
            .unwrap(),
            r#"{"KeepsLocalChanges":{"added":["a.txt"],"modified":[],"removed":["c.txt"]}}"#
        );
        assert_eq!(
            serde_json::to_string(&PullOutcome::Blocked {
                conflicts: vec![PathBuf::from("x.txt")],
            })
            .unwrap(),
            r#"{"Blocked":{"conflicts":["x.txt"]}}"#
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
