use std::path::PathBuf;

use url::Url;

use crate::engine::quilt;

/// The desktop app's failures, carried as text so the engine needs no Tauri
/// types.
#[derive(thiserror::Error, Debug)]
pub enum DesktopError {
    #[error("Tauri failed with {0}")]
    Tauri(String),

    #[error("Window not found")]
    Window,

    #[error("Clipboard unavailable: {0}")]
    Clipboard(String),

    #[error("User cancelled operation")]
    UserCancelled,
}

#[derive(thiserror::Error, Debug)]
pub enum RouteError {
    #[error("URL has no path segments: {0}")]
    NoPathSegments(Url),

    #[error("No page found in URL path: {0}")]
    NoPageInPath(Url),

    #[error("Missing host fragment in URL: {0}")]
    MissingHostFragment(Url),

    #[error("Missing S3 URI query parameter: {0}")]
    MissingS3UriQuery(Url),

    #[error("Page not found: {0}")]
    PageNotFound(String),
}

#[derive(thiserror::Error, Debug)]
pub enum OAuthUiError {
    #[error("OAuth error: {0}")]
    OAuth(String),

    #[error("Post-login navigation failed: {0}")]
    PostLogin(String),
}

#[derive(thiserror::Error, Debug)]
pub enum TelemetryError {
    #[error("Mixpanel error: {0}")]
    Mixpanel(#[from] crate::engine::telemetry::mixpanel::SendError),

    #[error("Mixpanel serialization error: {0}")]
    Serialize(String),

    /// A send abandoned before the ingest API answered.
    ///
    /// Its own variant rather than borrowing another, because how a failure is
    /// *classified* follows from its variant: a timeout reached the network and got
    /// no verdict, which is the user's connection and not our bug. Filed under a
    /// serialization error it would read as a refusal, raise a false fault, and
    /// spend the one refusal report a run is allowed.
    #[error("Mixpanel send timed out after {0}s")]
    SendTimeout(u64),
}

#[derive(thiserror::Error, Debug)]
pub enum FsOpenError {
    #[error("Failed to open: {0}")]
    Open(#[from] opener::OpenError),

    #[error("Path {0} doesn't exist")]
    PathNotFound(PathBuf),

    #[error("Zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
}

#[derive(thiserror::Error, Debug)]
pub enum PackageUriError {
    #[error("Failed using Quilt+S3 URI: {0}")]
    Invalid(String),

    #[error("Failed to parse {0}")]
    Qs(#[from] serde_qs::Error),
}

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error(transparent)]
    Desktop(#[from] DesktopError),

    #[error(transparent)]
    Route(#[from] RouteError),

    #[error(transparent)]
    OAuthUi(#[from] OAuthUiError),

    #[error(transparent)]
    Telemetry(#[from] TelemetryError),

    #[error(transparent)]
    FsOpen(#[from] FsOpenError),

    #[error(transparent)]
    PackageUri(#[from] PackageUriError),

    #[error("Quilt error: {0}")]
    Quilt(quilt::Error),

    #[error("Filesystem error: {0}")]
    FS(#[from] std::io::Error),

    #[error("Failed to parse JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Failed to parse Url: {0}")]
    ParseUrl(#[from] url::ParseError),

    #[error("Commit error: {0}")]
    Commit(String),

    #[error("General error: {0}")]
    General(String),

    #[error("Package name must be prefix/name, using only letters, digits, _ and -; got: {0}")]
    PackageName(String),

    #[error("Test failed: {0}")]
    Test(String),
}

impl From<quilt::Error> for Error {
    fn from(err: quilt::Error) -> Error {
        Error::Quilt(err)
    }
}

#[cfg(test)]
impl From<quilt::LogicalKeyError> for Error {
    fn from(err: quilt::LogicalKeyError) -> Error {
        Error::Quilt(err.into())
    }
}

impl From<quilt::InstallPackageError> for Error {
    fn from(err: quilt::InstallPackageError) -> Error {
        Error::Quilt(quilt::Error::InstallPackage(err))
    }
}

impl From<quilt_uri::UriError> for Error {
    fn from(err: quilt_uri::UriError) -> Error {
        Error::Quilt(quilt::Error::Uri(err))
    }
}

impl From<String> for Error {
    fn from(s: String) -> Error {
        Error::General(s)
    }
}

// Compose `?` through focused enums: each external error routes via its
// owning focused enum into the top-level `Error`.
impl From<opener::OpenError> for Error {
    fn from(e: opener::OpenError) -> Self {
        Error::FsOpen(FsOpenError::Open(e))
    }
}

impl From<zip::result::ZipError> for Error {
    fn from(e: zip::result::ZipError) -> Self {
        Error::FsOpen(FsOpenError::Zip(e))
    }
}

impl From<serde_qs::Error> for Error {
    fn from(e: serde_qs::Error) -> Self {
        Error::PackageUri(PackageUriError::Qs(e))
    }
}

impl From<crate::engine::telemetry::mixpanel::SendError> for Error {
    fn from(e: crate::engine::telemetry::mixpanel::SendError) -> Self {
        Error::Telemetry(TelemetryError::Mixpanel(e))
    }
}

impl Error {
    /// True when storage refused the call with `AccessDenied`.
    ///
    /// Forwards to [`quilt::Error::is_access_denied`] so callers holding
    /// only a UI-level [`enum@Error`] can still tell "the active role cannot
    /// reach this bucket" apart from a broken session. The distinction
    /// matters because a denial is *not* an auth failure: credential
    /// vending succeeded, and signing in again re-vends the same denied
    /// role.
    #[must_use]
    pub fn is_access_denied(&self) -> bool {
        matches!(self, Error::Quilt(err) if err.is_access_denied())
    }

    /// The quilt error under any failed upload: what to classify on.
    #[must_use]
    pub fn quilt_cause(&self) -> Option<&quilt::Error> {
        match self {
            Error::Quilt(err) => Some(err.cause()),
            _ => None,
        }
    }

    /// This error in the words a user should read: the domain error's own
    /// message, without the `Quilt error:` wrapper that [`Display`] adds.
    ///
    /// The wrapper says which of this crate's error variants the failure
    /// arrived in, which is a fact about the codebase rather than about what
    /// went wrong, and it reads as noise in front of a sentence the domain
    /// already worded. The autosync pause path drops it by binding the inner
    /// error per arm (`autopull/tick.rs`) and asserts it never reaches the
    /// tray; this is the same removal for a caller holding only the outer
    /// error.
    ///
    /// [`Display`]: std::fmt::Display
    #[must_use]
    pub fn user_facing(&self) -> String {
        match self {
            Error::Quilt(inner) => inner.to_string(),
            other => other.to_string(),
        }
    }

    /// True when S3 rejected the credentials themselves.
    ///
    /// The counterpart to [`Error::is_access_denied`]: that one says the
    /// session is healthy and the role is not allowed, this one says the
    /// session is not healthy. The watcher must not back this off — retrying a
    /// rejected credential never succeeds — so it routes to the same
    /// login-required affordance a refused vend does.
    #[must_use]
    pub fn is_invalid_credentials(&self) -> bool {
        matches!(self, Error::Quilt(err) if err.is_invalid_credentials())
    }

    /// True when there is no usable session, by either route — see
    /// [`quilt::Error::is_session_absent`]. This is the predicate a surface
    /// should branch on when deciding what to show; the narrower
    /// [`Error::is_invalid_credentials`] exists for the watcher, which counts
    /// episodes and wants only the rejected-credential route.
    #[must_use]
    pub fn is_session_absent(&self) -> bool {
        matches!(self, Error::Quilt(err) if err.is_session_absent())
    }

    /// The deployment this request was for, if any. `None` for a bare S3
    /// bucket reached with ambient credentials, and for non-S3 errors.
    #[must_use]
    pub fn s3_host(&self) -> Option<&quilt_uri::Host> {
        match self {
            Error::Quilt(err) => err.s3_host(),
            _ => None,
        }
    }

    /// Serialize a recognized state as JSON for the frontend; `Display`
    /// otherwise.
    ///
    /// Kinds name the state, never the action — which response it deserves is
    /// the receiving surface's call.
    pub fn to_frontend_string(&self) -> String {
        match self {
            Error::Quilt(err) => match quilt_state(err, &self.to_string()) {
                Some(json) => json.to_string(),
                // Without the "Quilt error:" frame: S3's own sentence is the
                // whole message.
                None if matches!(err, quilt::Error::S3(_)) => err.to_string(),
                None => self.to_string(),
            },
            _ => self.to_string(),
        }
    }
}

/// The recognized state `err` is in, as the frontend's JSON; `None` for one
/// shown as plain text. `display` is the message for states that show the
/// error as it reads.
fn quilt_state(err: &quilt::Error, display: &str) -> Option<serde_json::Value> {
    match err {
        // The cause's state, with the file in front of its message.
        quilt::Error::Upload { key, source } => {
            let mut json = quilt_state(source, &source.to_string())?;
            let message = json["message"].as_str().unwrap_or_default();
            json["message"] = format!("Couldn't upload {key}: {message}").into();
            Some(json)
        }
        quilt::Error::Login(quilt::LoginError::NoSession(host)) => {
            let mut json = serde_json::json!({
                "kind": "session_absent",
                "message": display,
            });
            if let Some(h) = host {
                json["host"] = serde_json::Value::String(h.to_string());
            }
            Some(json)
        }
        // Not `session_absent`: the deployment answered, so no session is
        // missing and a sign-in cannot help. Unmatched by the router,
        // which renders it in place.
        quilt::Error::Login(quilt::LoginError::NoRegistryUrl(host)) => Some(serde_json::json!({
            "kind": "registry_url_missing",
            "message": format!(
                "{host} is not configured as a Quilt deployment — its config.json \
                 names no registry. Whoever administers it will need to fix that."
            ),
            "host": host.to_string(),
        })),
        quilt::Error::Lineage(error) => lineage_kind(error).map(|kind| {
            serde_json::json!({
                "kind": kind,
                "message": display,
            })
        }),
        quilt::Error::S3(error) => s3_error_to_frontend(error),
        _ => None,
    }
}

/// Not `setup_required` for a domain moving between `data.json` and refs:
/// setting up again cannot finish or undo the move.
fn lineage_kind(error: &quilt::LineageError) -> Option<&'static str> {
    match error {
        quilt::LineageError::Missing | quilt::LineageError::MissingHome => Some("setup_required"),
        quilt::LineageError::RefsNotWritable { .. } => Some("lineage_not_writable"),
        quilt::LineageError::MigrationRefused { .. } => Some("lineage_migration_refused"),
        quilt::LineageError::RollbackRefused(_) => Some("lineage_rollback_refused"),
        quilt::LineageError::RollingBack { .. } => Some("lineage_rolling_back"),
        _ => None,
    }
}

fn s3_error_to_frontend(error: &quilt::S3Error) -> Option<serde_json::Value> {
    let (kind, message) = match &error.kind {
        quilt::S3ErrorKind::InvalidCredentials(_) if error.host.is_none() => (
            "credentials_invalid",
            "AWS credentials in ~/.aws/credentials are invalid. Please update your credentials."
                .to_string(),
        ),
        // Same state as a refused vend, so the same kind. Only user-initiated
        // commands reach this; autosync reports on its own event channel.
        quilt::S3ErrorKind::InvalidCredentials(_) => (
            "session_absent",
            format!(
                "Your session for {} has expired. Please sign in again.",
                error.host.as_ref().expect("host checked above")
            ),
        ),
        quilt::S3ErrorKind::NotFound(_) => (
            "not_found",
            "Package not found: the requested version or object does not exist.".to_string(),
        ),
        quilt::S3ErrorKind::AccessDenied(_) => (
            "access_denied",
            "The active role does not have access to this object.".to_string(),
        ),
        _ => return None,
    };

    let mut response = serde_json::json!({
        "kind": kind,
        "message": message,
    });
    if let Some(host) = &error.host {
        response["host"] = serde_json::Value::String(host.to_string());
    }
    Some(response)
}

impl From<Error> for String {
    fn from(err: Error) -> String {
        format!("{err}")
    }
}

#[cfg(test)]
mod tests {
    use quilt_uri::fixtures;

    use super::*;

    #[test]
    fn user_facing_drops_the_quilt_wrapper_and_keeps_the_sentence() {
        // With `to_string()`, the create dialog would read "Failed to create
        // package: Quilt error: The package proj/process is already installed."
        // Only the last part is useful to the user.
        let err = Error::Quilt(quilt::Error::InstallPackage(
            quilt::InstallPackageError::AlreadyInstalled(quilt_uri::Namespace::from((
                "proj", "process",
            ))),
        ));
        let inner = "The package proj/process is already installed";
        assert!(
            err.to_string().starts_with("Quilt error:"),
            "the wrapper is what Display adds: {err}"
        );
        assert!(
            !err.user_facing().contains("Quilt error:"),
            "and what this drops: {}",
            err.user_facing()
        );
        assert!(
            err.user_facing().contains(inner),
            "the sentence survives: {}",
            err.user_facing()
        );
    }

    #[test]
    fn user_facing_leaves_a_non_quilt_error_alone() {
        // Only the `Quilt` wrapper is framing. The others name a foreign error's
        // domain — "Filesystem error:" in front of an `io::Error` is the context
        // that says which subsystem failed, and dropping it would lose it.
        let err = Error::FS(std::io::Error::other("disk fell off"));
        assert_eq!(err.user_facing(), err.to_string());
        assert!(err.user_facing().starts_with("Filesystem error:"));
    }

    #[test]
    fn to_frontend_string_session_absent_with_host() {
        let host = fixtures::host();
        let err = Error::Quilt(quilt::Error::Login(quilt::LoginError::NoSession(Some(
            host,
        ))));
        let json: serde_json::Value = serde_json::from_str(&err.to_frontend_string()).unwrap();
        assert_eq!(json["kind"], "session_absent");
        assert_eq!(json["host"], "quilt.test");
        assert!(json["message"].as_str().unwrap().contains("No session"));
    }

    #[test]
    fn to_frontend_string_session_absent_no_host() {
        let err = Error::Quilt(quilt::Error::Login(quilt::LoginError::NoSession(None)));
        let json: serde_json::Value = serde_json::from_str(&err.to_frontend_string()).unwrap();
        assert_eq!(json["kind"], "session_absent");
        assert!(
            json.get("host").is_none(),
            "host should be absent when None"
        );
    }

    #[test]
    /// A deployment whose `config.json` names no registry is misconfigured, not
    /// signed out. It must not borrow `session_absent`, which the router
    /// answers by navigating to `/login` — a page that cannot fix a config
    /// file, for a problem the user has no way to act on.
    fn to_frontend_string_missing_registry_is_not_a_dead_session() {
        let host = fixtures::host();
        let err = Error::Quilt(quilt::Error::Login(quilt::LoginError::NoRegistryUrl(host)));
        let json: serde_json::Value = serde_json::from_str(&err.to_frontend_string()).unwrap();
        assert_eq!(json["kind"], "registry_url_missing");
        assert_ne!(
            json["kind"], "session_absent",
            "a misconfiguration must not route the user to sign in"
        );
        assert_eq!(json["host"], "quilt.test");
        let message = json["message"].as_str().unwrap();
        assert!(
            message.contains("quilt.test"),
            "must name the host: {message}"
        );
        assert!(
            !message.contains("sign in"),
            "there is nothing to sign in to: {message}"
        );
    }

    #[test]
    fn to_frontend_string_setup_required() {
        let err = Error::Quilt(quilt::Error::Lineage(quilt::LineageError::Missing));
        let json: serde_json::Value = serde_json::from_str(&err.to_frontend_string()).unwrap();
        assert_eq!(json["kind"], "setup_required");

        let err = Error::Quilt(quilt::Error::Lineage(quilt::LineageError::MissingHome));
        let json: serde_json::Value = serde_json::from_str(&err.to_frontend_string()).unwrap();
        assert_eq!(json["kind"], "setup_required");
    }

    /// A domain moving between `data.json` and refs is not one to set up:
    /// each state of the move is a kind of its own, worded as quilt words it,
    /// so an interrupted rollback still names the command that finishes it.
    #[test]
    fn to_frontend_string_a_lineage_move_is_not_setup() {
        let config = PathBuf::from("/d/.quilt/domain.json");
        let unparsed = serde_json::from_str::<serde_json::Value>("{").unwrap_err();
        for (error, kind) in [
            (
                quilt::LineageError::RefsNotWritable {
                    path: config.clone(),
                    state: "is in progress",
                },
                "lineage_not_writable",
            ),
            (
                quilt::LineageError::MigrationRefused {
                    path: PathBuf::from("/d/.quilt/data.json"),
                    source: unparsed,
                },
                "lineage_migration_refused",
            ),
            (
                quilt::LineageError::RollbackRefused(Box::new(quilt::LineageError::NoRemote)),
                "lineage_rollback_refused",
            ),
            (
                quilt::LineageError::RollingBack { path: config },
                "lineage_rolling_back",
            ),
        ] {
            let err = Error::Quilt(quilt::Error::Lineage(error));
            let sent = err.to_frontend_string();
            let json: serde_json::Value = serde_json::from_str(&sent).unwrap_or_default();
            assert_eq!(json["kind"], kind, "{sent}");
            assert_eq!(json["message"], err.to_string(), "{sent}");
        }
    }

    #[test]
    fn to_frontend_string_other_errors_are_plain_text() {
        let err = Error::General("something broke".to_string());
        let result = err.to_frontend_string();
        assert_eq!(result, "General error: something broke");
        assert!(serde_json::from_str::<serde_json::Value>(&result).is_err());
    }

    #[test]
    fn to_frontend_string_invalid_local_credentials_is_actionable() {
        let err = Error::Quilt(quilt::Error::S3(quilt::S3Error::new(
            quilt::S3ErrorKind::InvalidCredentials(
                "InvalidAccessKeyId: raw SDK details".to_string(),
            ),
        )));
        let json: serde_json::Value = serde_json::from_str(&err.to_frontend_string()).unwrap();
        assert_eq!(json["kind"], "credentials_invalid");
        assert_eq!(
            json["message"],
            "AWS credentials in ~/.aws/credentials are invalid. Please update your credentials."
        );
        assert!(!json["message"].as_str().unwrap().contains("raw SDK"));
    }

    /// A stale session on a deployment routes to the kind the frontend already
    /// navigates on, and carries the host it needs to build `/login?host=…`.
    /// Reporting it as its own kind would render a page telling the user to sign
    /// in with no way to do so.
    #[test]
    fn to_frontend_string_expired_session_sends_the_user_to_login() {
        let host = fixtures::host();
        let err = Error::Quilt(quilt::Error::S3(quilt::S3Error {
            host: Some(host.clone()),
            kind: quilt::S3ErrorKind::InvalidCredentials(
                "ExpiredToken: raw SDK details".to_string(),
            ),
        }));

        let json: serde_json::Value = serde_json::from_str(&err.to_frontend_string()).unwrap();
        assert_eq!(json["kind"], "session_absent");
        assert_eq!(json["host"], host.to_string());
        assert_eq!(
            json["message"],
            format!("Your session for {host} has expired. Please sign in again.")
        );
        assert!(!json["message"].as_str().unwrap().contains("raw SDK"));
    }

    /// An upload that fails on an expired session still sends the user to
    /// sign in, and says which file it was.
    #[test]
    fn to_frontend_string_failed_upload_keeps_its_cause_and_names_the_file() {
        let host = fixtures::host();
        let err = Error::Quilt(quilt::Error::Upload {
            key: quilt::LogicalKey::new("data/b.txt").unwrap(),
            source: Box::new(quilt::Error::S3(quilt::S3Error {
                host: Some(host.clone()),
                kind: quilt::S3ErrorKind::InvalidCredentials("ExpiredToken".to_string()),
            })),
        });

        let json: serde_json::Value = serde_json::from_str(&err.to_frontend_string()).unwrap();
        assert_eq!(json["kind"], "session_absent");
        assert_eq!(json["host"], host.to_string());
        assert_eq!(
            json["message"],
            format!(
                "Couldn't upload data/b.txt: Your session for {host} has expired. Please sign in again."
            )
        );
    }

    #[test]
    fn to_frontend_string_failed_upload_of_any_other_cause_is_plain_text() {
        let err = Error::Quilt(quilt::Error::Upload {
            key: quilt::LogicalKey::new("b.txt").unwrap(),
            source: Box::new(quilt::Error::Io(std::io::Error::from(
                std::io::ErrorKind::NotFound,
            ))),
        });
        let result = err.to_frontend_string();
        assert!(serde_json::from_str::<serde_json::Value>(&result).is_err());
        assert!(result.contains("Couldn't upload b.txt: "), "{result}");
    }

    #[test]
    fn to_frontend_string_missing_object_is_actionable() {
        let err = Error::Quilt(quilt::Error::S3(quilt::S3Error::new(
            quilt::S3ErrorKind::NotFound("NoSuchKey: raw SDK details".to_string()),
        )));
        let json: serde_json::Value = serde_json::from_str(&err.to_frontend_string()).unwrap();
        assert_eq!(json["kind"], "not_found");
        assert_eq!(
            json["message"],
            "Package not found: the requested version or object does not exist."
        );
        assert!(!json["message"].as_str().unwrap().contains("raw SDK"));
    }
}
