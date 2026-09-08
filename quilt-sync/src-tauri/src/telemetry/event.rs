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

/// Something autosync did on its own, against a deployment it is **known** to
/// have.
///
/// The host is required here where a manual package operation's is optional, and
/// the loop is why: it skips any package whose lineage carries no origin before
/// doing any work, so every outcome it reports is about a package with a remote.
/// That is the proof a required host needs — the tightening the previous change
/// left open for the manual paths, available here for free.
#[derive(Debug, Clone, Serialize)]
pub struct AutosyncEvent {
    pub host: Host,
}

/// Autosync stopping on a package, and the kind of thing that stopped it.
#[derive(Debug, Clone, Serialize)]
pub struct AutosyncPausedEvent {
    pub host: Host,
    pub reason: PausedKind,
}

/// Autosync stopping because the session expired.
///
/// The one autosync payload whose host is optional, and not for the usual
/// reason: the refusal itself carries the host only when the failing call knew
/// which one it was talking to, so an absent host here means the engine could not
/// name the deployment, not that there wasn't one.
#[derive(Debug, Clone, Serialize)]
pub struct AutosyncAuthEvent {
    pub host: Option<Host>,
}

/// An operation someone refused, and the action it would have been.
///
/// `action` is the wire name of the event this action *would* have reported on
/// success, so `action_refused where action = package_committed` compares directly
/// against the `package_committed` series. That is the whole reason this is a
/// separate event rather than a `result` property: a property would change what
/// every existing series counts, and the name-continuity rule already rejected
/// that trade once, for autosync.
#[derive(Debug, Clone, Serialize)]
pub struct RefusedEvent {
    pub action: String,
    pub reason: RefusalKind,
    pub host: Option<Host>,
}

/// Why autosync paused, coarsely — the variant, never its contents.
///
/// The engine's own reason type carries a conflicting-file list, a role name and
/// a free-text message for the UI banner. None of that crosses into telemetry:
/// the vocabulary admits no free text, and a file name is exactly the kind of
/// detail [the module's rule](self) exists to keep out. The category is what a
/// report can act on anyway — "how often does a role denial stop background
/// sync" does not need to know which file.
#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PausedKind {
    PendingChanges,
    PendingCommit,
    Diverged,
    PullConflict,
    RoleDenied,
    /// The engine's own catch-all for non-transient errors it has not
    /// enumerated. Coarse by inheritance rather than by choice: sharpening it
    /// means sharpening `PausedReason` upstream first.
    Other,
}

impl From<&crate::autopull::PausedReason> for PausedKind {
    fn from(reason: &crate::autopull::PausedReason) -> Self {
        use crate::autopull::PausedReason as R;
        // Exhaustive rather than wildcarded, so a new pause reason cannot be
        // added without deciding how a report should see it.
        match reason {
            R::PendingChanges => Self::PendingChanges,
            R::PendingCommit => Self::PendingCommit,
            R::Diverged => Self::Diverged,
            R::PullConflict(_) => Self::PullConflict,
            R::RoleDenied { .. } => Self::RoleDenied,
            R::Other(_) => Self::Other,
        }
    }
}

/// Which sink a failed operation belongs to.
///
/// The split is by **who can act**, not by severity: a fault is ours to fix and
/// belongs in the issue list, a refusal is somebody else's and belongs in the
/// event stream. See the corpus rule for why the crash reporter is the wrong home
/// for the second kind — chiefly that a refusal throws nothing, so if analytics
/// does not carry it, nothing does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Failure {
    /// A legitimate state someone other than us can resolve.
    Refusal(RefusalKind),
    /// Should not have happened, and only we can act on it.
    Fault,
}

/// Why an operation was refused, coarsely — never the error's contents.
///
/// Nine categories over thirty-odd error variants, and the grain is chosen so
/// each one answers *who acts*: the user, their administrator, or nobody.
/// Anything that cannot be placed stays a fault, deliberately: a misfiled refusal
/// is noise in one report, a misfiled fault is a bug nobody sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum RefusalKind {
    /// The user backed out. Filing this as an issue is the clearest case of the
    /// pollution the rule exists to prevent.
    Cancelled,
    /// Nobody is logged in, or the session could not be renewed.
    LoginRequired,
    /// Authenticated, but the active role cannot reach it.
    RoleDenied,
    /// The package does not satisfy the workflow — the case that named this unit.
    WorkflowRejected,
    /// The deployment's own configuration is wrong: a malformed workflow schema, a
    /// catalog missing its registry URL. Neither the user's fault nor ours, and it
    /// fails identically for everyone on that deployment until an admin fixes it.
    DeploymentMisconfigured,
    /// A tracked path changed on both sides.
    PullConflict,
    /// The requested state already holds, so there was nothing to do.
    AlreadySettled,
    /// The thing acted on is not there.
    Missing,
    /// The network gave no verdict. Reporting these would fill the issue list with
    /// other people's tunnels — the same reasoning the delivery rule uses.
    Unreachable,
}

impl From<&crate::error::Error> for Failure {
    fn from(err: &crate::error::Error) -> Self {
        use crate::error::{
            Error as E, FsOpenError, OAuthUiError, PackageUriError, RouteError, TauriUiError,
            TelemetryError,
        };

        // Exhaustive, so a new error variant cannot inherit an answer.
        match err {
            E::TauriUi(TauriUiError::UserCancelled) => Self::Refusal(RefusalKind::Cancelled),
            E::FsOpen(FsOpenError::PathNotFound(_)) => Self::Refusal(RefusalKind::Missing),
            E::Telemetry(TelemetryError::SendTimeout(_)) => Self::Refusal(RefusalKind::Unreachable),

            E::Quilt(err) => Self::from(err),

            // Ours. One arm rather than one per family, since the bodies are
            // identical — but every inner variant is still named, so adding one
            // cannot silently inherit this answer. Each is either the UI handing the
            // backend something it should not have, the OS refusing us, or an opaque
            // string that would need a variant of its own before it could be placed.
            E::TauriUi(TauriUiError::Tauri(_) | TauriUiError::Window)
            | E::FsOpen(FsOpenError::Open(_) | FsOpenError::Zip(_))
            | E::Telemetry(TelemetryError::Mixpanel(_) | TelemetryError::Serialize(_))
            | E::Route(
                RouteError::NoPathSegments(_)
                | RouteError::NoPageInPath(_)
                | RouteError::MissingHostFragment(_)
                | RouteError::MissingS3UriQuery(_)
                | RouteError::PageNotFound(_),
            )
            | E::OAuthUi(OAuthUiError::OAuth(_) | OAuthUiError::PostLogin(_))
            | E::PackageUri(PackageUriError::Invalid(_) | PackageUriError::Qs(_))
            | E::FS(_)
            | E::Json(_)
            | E::ParseUrl(_)
            | E::Commit(_)
            | E::General(_)
            | E::Test(_) => Self::Fault,
        }
    }
}

impl From<&crate::quilt::Error> for Failure {
    fn from(err: &crate::quilt::Error) -> Self {
        use crate::quilt::{
            Error as E, InstallPackageError, LoginError, PackageOpError, RemoteCatalogError,
            RoleError, WorkflowValidationError,
        };

        match err {
            // Credentials or tokens that cannot be read or renewed all resolve the
            // same way, whatever broke: log in again.
            E::Auth(_, _) | E::Login(LoginError::Required(_)) => {
                Self::Refusal(RefusalKind::LoginRequired)
            }
            E::Role(RoleError::NotAuthenticated(_)) => Self::Refusal(RefusalKind::LoginRequired),

            E::Role(RoleError::SwitchRejected(_)) => Self::Refusal(RefusalKind::RoleDenied),
            E::S3(s3) if s3.is_access_denied() => Self::Refusal(RefusalKind::RoleDenied),
            E::S3(s3) if s3.is_not_found() => Self::Refusal(RefusalKind::Missing),

            E::WorkflowValidation(WorkflowValidationError::Rejected(_)) => {
                Self::Refusal(RefusalKind::WorkflowRejected)
            }
            // A schema the deployment cannot use, and a catalog that never
            // published its registry URL, fail for everyone on it until an admin
            // acts — so they are refusals rather than our faults, and separating
            // them from a rejection keeps "how often does a workflow reject a
            // publish" answerable.
            E::WorkflowValidation(
                WorkflowValidationError::InvalidSchema { .. }
                | WorkflowValidationError::UnsupportedRef { .. }
                | WorkflowValidationError::UnsupportedMetaSchema { .. }
                | WorkflowValidationError::InvalidHandlePattern { .. },
            )
            | E::Login(LoginError::RequiredRegistryUrl(_))
            | E::RemoteCatalog(
                RemoteCatalogError::Workflow(_)
                | RemoteCatalogError::InvalidWorkflowsConfig(_)
                | RemoteCatalogError::HostConfig(_)
                | RemoteCatalogError::BucketUnreachable(_),
            ) => Self::Refusal(RefusalKind::DeploymentMisconfigured),

            E::PackageOp(PackageOpError::PullConflict(_)) => {
                Self::Refusal(RefusalKind::PullConflict)
            }

            E::PackageOp(PackageOpError::AlreadyUpToDate)
            | E::InstallPackage(
                InstallPackageError::AlreadyInstalled(_) | InstallPackageError::NotInstalled(_),
            ) => Self::Refusal(RefusalKind::AlreadySettled),

            // Reached the network and got no verdict. Anything else reqwest
            // reports is a request we built wrong, so it stays ours.
            E::Reqwest(err) if err.is_connect() || err.is_timeout() => {
                Self::Refusal(RefusalKind::Unreachable)
            }

            E::Fs(_) | E::Io(_) if err.is_not_found() => Self::Refusal(RefusalKind::Missing),

            // The remaining opaque-string and mechanical variants. Ours, or
            // unclassifiable without giving them variants first — which is the same
            // answer, since an unclassifiable failure must stay visible.
            // `Undo` sits here on the same reasoning, and with a caveat: every
            // undo refusal is a precondition the user can act on (nothing to
            // undo, a dirty tree, a chain already consumed by pushing), so none
            // of them is really ours. It is unreachable from this surface today
            // — undo is CLI-only, with no IPC behind it — and giving it a
            // refusal kind means widening the telemetry vocabulary, which
            // belongs with the surface that first drives it rather than here.
            E::PackageOp(
                PackageOpError::Commit(_)
                | PackageOpError::Push(_)
                | PackageOpError::Publish(_)
                | PackageOpError::Undo(_)
                | PackageOpError::Package(_),
            )
            | E::Role(RoleError::GraphQl(_))
            | E::S3(_)
            | E::Checksum(_)
            | E::Fs(_)
            | E::InstallPath(_)
            | E::Io(_)
            | E::Json(_)
            | E::Lineage(_)
            | E::Manifest(_)
            | E::ObjectHash(_)
            | E::Reqwest(_)
            | E::ToString(_)
            | E::TryFromIntError(_)
            | E::Unimplemented
            | E::Uri(_)
            | E::UrlParse(_)
            | E::Utf8(_)
            | E::Yaml(_) => Self::Fault,
        }
    }
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

    // ── autosync: the engine acting with no user present ──
    /// A publish the loop completed on its own. Deliberately *not*
    /// `package_published`: folding unattended work into a series read as
    /// user actions would redefine it silently, which is the thing the
    /// name-continuity rule exists to prevent.
    AutosyncPublished(AutosyncEvent),
    /// The loop stopped on a package, with the category that stopped it.
    AutosyncPaused(AutosyncPausedEvent),
    /// The loop stopped because the session expired — the silent-failure signal:
    /// background sync ceases working and nothing asked the user anything.
    AutosyncLoginRequired(AutosyncAuthEvent),

    // ── auth: always names the deployment it acts on ──
    UserLoggedIn(LoginEvent),
    RoleSwitched(AuthEvent),
    OAuthLoginInitiated(AuthEvent),
    AuthErased(AuthEvent),

    // ── refusals: an action somebody other than us declined ──
    /// Something the user, their administrator, or the network refused. Not a
    /// fault: those go to the crash reporter and are absent from this vocabulary
    /// by design.
    ActionRefused(RefusedEvent),
}

impl MixpanelEvent {
    /// The deployment this event concerns, for crash-report context.
    ///
    /// Every variant is listed rather than falling through a wildcard, so a new
    /// event cannot be added without deciding whether it names a host — the
    /// point of putting the host in the payloads in the first place.
    /// The refusal this action reports instead of its success event.
    ///
    /// Derived from the success event rather than named at the call site, so the
    /// `action` property cannot drift from the series it is meant to be compared
    /// against. `None` when the name could not be derived — the caller then logs
    /// the failure and reports nothing, which is the same trade the dry-run
    /// renderer makes: a formatting slip must not turn into a lost observation, but
    /// it must not fabricate one either.
    pub(crate) fn refused(&self, reason: RefusalKind) -> Option<Self> {
        let (action, _) = super::mixpanel::event_payload(self).ok()?;
        Some(Self::ActionRefused(RefusedEvent {
            action,
            reason,
            host: self.host().cloned(),
        }))
    }

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

            Self::AutosyncPublished(e) => Some(&e.host),
            Self::AutosyncPaused(e) => Some(&e.host),
            Self::AutosyncLoginRequired(e) => e.host.as_ref(),

            Self::RoleSwitched(e) | Self::OAuthLoginInitiated(e) | Self::AuthErased(e) => {
                Some(&e.host)
            }
            Self::UserLoggedIn(e) => Some(&e.host),

            Self::ActionRefused(e) => e.host.as_ref(),

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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::{Error, FsOpenError, TauriUiError};
    use crate::quilt;

    fn refusal(err: &Error) -> Option<RefusalKind> {
        match Failure::from(err) {
            Failure::Refusal(kind) => Some(kind),
            Failure::Fault => None,
        }
    }

    /// The case that named the unit: a package that does not satisfy the workflow is
    /// the user's to fix, so it must not reach the crash reporter.
    #[test]
    fn a_workflow_rejection_is_a_refusal_not_a_fault() {
        let violations = quilt::workflow::Violations::from_nonempty(vec![
            quilt::workflow::RuleViolation::MessageRequired,
        ])
        .expect("a nonempty violation list");
        let err = Error::Quilt(quilt::Error::WorkflowValidation(
            quilt::WorkflowValidationError::Rejected(violations),
        ));

        assert_eq!(refusal(&err), Some(RefusalKind::WorkflowRejected));
    }

    /// A workflow the deployment cannot even load is nobody's bug here — not the
    /// user's to fix and not ours — and it is a *different* question from a
    /// rejection, so it gets its own category rather than being folded in.
    #[test]
    fn a_broken_workflow_schema_is_a_separate_category_from_a_rejection() {
        let err = Error::Quilt(quilt::Error::WorkflowValidation(
            quilt::WorkflowValidationError::UnsupportedRef {
                kind: quilt::workflow::SchemaKind::Metadata,
            },
        ));

        assert_eq!(refusal(&err), Some(RefusalKind::DeploymentMisconfigured));
    }

    /// Pressing Cancel used to file a crash report — the clearest instance of the
    /// pollution the rule exists to prevent.
    #[test]
    fn cancelling_is_a_refusal() {
        assert_eq!(
            refusal(&Error::TauriUi(TauriUiError::UserCancelled)),
            Some(RefusalKind::Cancelled)
        );
    }

    /// Unclassifiable stays a fault, in both directions: an opaque string carries no
    /// variant to decide on, and downgrading it would hide a real bug.
    #[test]
    fn what_cannot_be_placed_stays_a_fault() {
        assert_eq!(refusal(&Error::General("boom".to_string())), None);
        assert_eq!(refusal(&Error::Quilt(quilt::Error::Unimplemented)), None);
        assert_eq!(
            refusal(&Error::FsOpen(FsOpenError::PathNotFound("/gone".into()))),
            Some(RefusalKind::Missing),
            "a missing path is the user's to resolve, not a crash"
        );
    }

    /// The property that makes the refusal series comparable: `action` is exactly
    /// the wire name of the success event, so `action_refused where action = X`
    /// lines up against the `X` series. Derived, never typed at a call site.
    #[test]
    fn a_refusal_carries_the_success_event_s_own_wire_name() {
        let success = MixpanelEvent::PackageCommitted(PackageEvent::hostless());
        let (success_name, _) =
            super::super::mixpanel::event_payload(&success).expect("a serializable event");

        let refused = success
            .refused(RefusalKind::WorkflowRejected)
            .expect("a derivable refusal");
        let (refused_name, properties) =
            super::super::mixpanel::event_payload(&refused).expect("a serializable event");

        assert_eq!(refused_name, "action_refused");
        let properties = properties.expect("a refusal carries properties");
        assert_eq!(
            properties.get("action").and_then(|v| v.as_str()),
            Some(success_name.as_str())
        );
        assert_eq!(
            properties.get("reason").and_then(|v| v.as_str()),
            Some("workflow_rejected")
        );
    }
}
