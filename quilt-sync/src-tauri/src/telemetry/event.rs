//! What we report, grouped by the behaviour each event describes. *How* it is
//! sent lives in [`mixpanel`](super::mixpanel).
//!
//! The vocabulary is deliberately low-detail: no package names, no paths, no
//! user identity. The one dimension that matters and cannot be recovered after
//! the fact is **which deployment** an action happened against, so an event that
//! concerns one carries its catalog host — and the payload type says whether
//! that host is guaranteed (the emitter derived it) or merely possible (the
//! emitter received it from its caller).
//!
//! Groups are behavioural rather than shaped: `RemotePackageEvent`,
//! `PackageEvent` and `PackageFileEvent` are structurally identical today and
//! are expected to diverge — a remote operation will plausibly want the bucket
//! or the revision it moved, where opening a folder never will.

use quilt_uri::{Host, S3PackageUri};
use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LoginFlow {
    OAuth,
    Legacy,
}

/// The catalog of the URI the acting surface rendered from, when it has one.
fn catalog_of(uri: Option<&S3PackageUri>) -> Option<Host> {
    uri.and_then(|uri| uri.catalog.clone())
}

/// An operation against a package's **remote**: it cannot happen without one.
///
/// The host is `Option` only because of where it comes from — the acting surface
/// hands over a package URI whose catalog is optional, so the emitter cannot
/// prove a host even though the operation implies one. Tightening this to a
/// required `Host` needs a URI type that carries its catalog as proof; until
/// then a `None` here means the caller failed to supply context, not that the
/// operation had no remote.
#[derive(Debug, Clone, Serialize)]
pub struct RemotePackageEvent {
    pub host: Option<Host>,
}

/// Work on a package that does **not** require a remote — a package may be
/// local-only, in which case there is genuinely no deployment to name.
#[derive(Debug, Clone, Serialize)]
pub struct PackageEvent {
    pub host: Option<Host>,
}

/// An OS-level action on a package's files: revealing, opening, browsing. Local
/// by nature; the host is attribution only, and absent for a local-only package.
#[derive(Debug, Clone, Serialize)]
pub struct PackageFileEvent {
    pub host: Option<Host>,
}

/// An action against a host's session. The emitter parses the host from its own
/// argument before doing anything, so the host is **guaranteed**.
#[derive(Debug, Clone, Serialize)]
pub struct AuthEvent {
    pub host: Host,
}

/// A completed login: which deployment, and how the user got in.
#[derive(Debug, Clone, Serialize)]
pub struct LoginEvent {
    pub host: Host,
    pub flow: LoginFlow,
}

impl RemotePackageEvent {
    pub fn for_uri(uri: Option<&S3PackageUri>) -> Self {
        Self {
            host: catalog_of(uri),
        }
    }

    pub fn for_host(host: Option<Host>) -> Self {
        Self { host }
    }
}

impl PackageEvent {
    pub fn for_uri(uri: Option<&S3PackageUri>) -> Self {
        Self {
            host: catalog_of(uri),
        }
    }

    /// A package with no remote yet — creation, before any remote is set.
    pub fn hostless() -> Self {
        Self { host: None }
    }
}

impl PackageFileEvent {
    pub fn for_uri(uri: Option<&S3PackageUri>) -> Self {
        Self {
            host: catalog_of(uri),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "t", content = "c")]
#[serde(rename_all = "snake_case")]
pub enum MixpanelEvent {
    // ── app: lifecycle and chrome, all of it before or beside any deployment ──
    AppLaunched,
    SetupCompleted,
    DirectoryPickerOpened,
    /// The `QuiltSync` home directory — the sync root, not a package.
    HomeDirOpened,
    /// Deliberately hostless even though a URL has a host: this one action opens
    /// catalog links, documentation, local paths and `mailto:` alike, so its
    /// host is not necessarily a Quilt deployment.
    WebBrowserOpened,

    // ── debug: support actions on this install's own files ──
    DebugDotQuiltOpened,
    DebugLogsOpened,
    /// The application data directory.
    DataDirOpened,
    DiagnosticLogsSaved,
    CrashReportSent,

    // ── remote package operations: impossible without a remote ──
    PackagePulled(RemotePackageEvent),
    PackagePushed(RemotePackageEvent),
    PackagePublished(RemotePackageEvent),
    LatestCertified(RemotePackageEvent),
    /// Discards local commits in favour of the remote `latest`.
    LocalReset(RemotePackageEvent),
    RemoteSet(RemotePackageEvent),
    PackageInstalled(RemotePackageEvent),

    // ── local package operations: a package may have no remote ──
    PackageCommitted(PackageEvent),
    PackageCreated(PackageEvent),
    PackageUninstalled(PackageEvent),
    QuiltignorePatternAdded(PackageEvent),

    // ── package files: OS-level actions on a package's working tree ──
    /// A package's own directory. Replaces the retired `file_browser_opened`,
    /// which also covered the home and data directories and so could not tell
    /// "opened my sync folder" from "opened a package folder".
    PackageDirOpened(PackageFileEvent),
    FileRevealed(PackageFileEvent),
    DefaultApplicationOpened(PackageFileEvent),

    // ── auth: always names the deployment it acts on ──
    UserLoggedIn(LoginEvent),
    RoleSwitched(AuthEvent),
    OAuthLoginInitiated(AuthEvent),
    AuthErased(AuthEvent),
}

impl MixpanelEvent {
    /// The deployment this event concerns, for crash-report context.
    ///
    /// Every variant is listed rather than falling through a wildcard, so a new
    /// event cannot be added without deciding whether it names a host — the
    /// point of putting the host in the payloads in the first place.
    pub(super) fn host(&self) -> Option<&Host> {
        match self {
            Self::PackagePulled(e)
            | Self::PackagePushed(e)
            | Self::PackagePublished(e)
            | Self::LatestCertified(e)
            | Self::LocalReset(e)
            | Self::RemoteSet(e)
            | Self::PackageInstalled(e) => e.host.as_ref(),

            Self::PackageCommitted(e)
            | Self::PackageCreated(e)
            | Self::PackageUninstalled(e)
            | Self::QuiltignorePatternAdded(e) => e.host.as_ref(),

            Self::PackageDirOpened(e)
            | Self::FileRevealed(e)
            | Self::DefaultApplicationOpened(e) => e.host.as_ref(),

            Self::RoleSwitched(e) | Self::OAuthLoginInitiated(e) | Self::AuthErased(e) => {
                Some(&e.host)
            }
            Self::UserLoggedIn(e) => Some(&e.host),

            Self::AppLaunched
            | Self::SetupCompleted
            | Self::DirectoryPickerOpened
            | Self::HomeDirOpened
            | Self::WebBrowserOpened
            | Self::DebugDotQuiltOpened
            | Self::DebugLogsOpened
            | Self::DataDirOpened
            | Self::DiagnosticLogsSaved
            | Self::CrashReportSent => None,
        }
    }
}
