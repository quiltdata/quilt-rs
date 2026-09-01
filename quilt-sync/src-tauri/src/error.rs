use std::path::PathBuf;

use url::Url;

use crate::quilt;

#[derive(thiserror::Error, Debug)]
pub enum TauriUiError {
    #[error("Tauri failed with {0}")]
    Tauri(#[from] tauri::Error),

    #[error("Window not found")]
    Window,

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
    Mixpanel(#[from] mixpanel_rs::error::Error),

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
    TauriUi(#[from] TauriUiError),

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

    #[error("Test failed: {0}")]
    Test(String),
}

impl From<quilt::Error> for Error {
    fn from(err: quilt::Error) -> Error {
        Error::Quilt(err)
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
impl From<tauri::Error> for Error {
    fn from(e: tauri::Error) -> Self {
        Error::TauriUi(TauriUiError::Tauri(e))
    }
}

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

impl From<mixpanel_rs::error::Error> for Error {
    fn from(e: mixpanel_rs::error::Error) -> Self {
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

    /// The host whose credentials S3 rejected, when there is one. `None` for a
    /// bucket reached with ambient `~/.aws` credentials: there is no stack to
    /// sign in to.
    #[must_use]
    pub fn invalid_credentials_host(&self) -> Option<&quilt_uri::Host> {
        match self {
            Error::Quilt(err) => err.invalid_credentials_host(),
            _ => None,
        }
    }

    /// Serialize actionable errors as JSON so the frontend can parse and react
    /// (e.g. redirect to `/login` or `/setup`). Falls back to `Display` for
    /// all other errors.
    pub fn to_frontend_string(&self) -> String {
        match self {
            Error::Quilt(quilt::Error::Login(quilt::LoginError::Required(host))) => {
                let mut json = serde_json::json!({
                    "kind": "login_required",
                    "message": self.to_string(),
                });
                if let Some(h) = host {
                    json["host"] = serde_json::Value::String(h.to_string());
                }
                json.to_string()
            }
            Error::Quilt(quilt::Error::Login(quilt::LoginError::RequiredRegistryUrl(host))) => {
                serde_json::json!({
                    "kind": "login_required",
                    "message": self.to_string(),
                    "host": host.to_string(),
                })
                .to_string()
            }
            Error::Quilt(quilt::Error::Lineage(
                quilt::LineageError::Missing | quilt::LineageError::MissingHome,
            )) => serde_json::json!({
                "kind": "setup_required",
                "message": self.to_string(),
            })
            .to_string(),
            Error::Quilt(quilt::Error::S3(error)) => s3_error_to_frontend(error),
            _ => self.to_string(),
        }
    }
}

fn s3_error_to_frontend(error: &quilt::S3Error) -> String {
    let (kind, message) = match &error.kind {
        quilt::S3ErrorKind::InvalidCredentials(_) if error.host.is_none() => (
            "credentials_invalid",
            "AWS credentials in ~/.aws/credentials are invalid. Please update your credentials."
                .to_string(),
        ),
        // A stale session on a Quilt deployment is the same dead end as a
        // refused vend, so it takes the same `login_required` route rather than
        // a second kind: the frontend already navigates to `/login?host=…&back=…`
        // for that, which turns "sign in again" from a sentence into somewhere
        // to go. Only user-initiated commands reach this — autosync reports a
        // login episode on its own event channel and never navigates.
        quilt::S3ErrorKind::InvalidCredentials(_) => (
            "login_required",
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
        _ => return error.to_string(),
    };

    let mut response = serde_json::json!({
        "kind": kind,
        "message": message,
    });
    if let Some(host) = &error.host {
        response["host"] = serde_json::Value::String(host.to_string());
    }
    response.to_string()
}

impl From<Error> for String {
    fn from(err: Error) -> String {
        format!("{err}")
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use super::*;

    #[test]
    fn to_frontend_string_login_required_with_host() {
        let host = quilt_uri::Host::from_str("catalog.dev").unwrap();
        let err = Error::Quilt(quilt::Error::Login(quilt::LoginError::Required(Some(host))));
        let json: serde_json::Value = serde_json::from_str(&err.to_frontend_string()).unwrap();
        assert_eq!(json["kind"], "login_required");
        assert_eq!(json["host"], "catalog.dev");
        assert!(json["message"].as_str().unwrap().contains("Login required"));
    }

    #[test]
    fn to_frontend_string_login_required_no_host() {
        let err = Error::Quilt(quilt::Error::Login(quilt::LoginError::Required(None)));
        let json: serde_json::Value = serde_json::from_str(&err.to_frontend_string()).unwrap();
        assert_eq!(json["kind"], "login_required");
        assert!(
            json.get("host").is_none(),
            "host should be absent when None"
        );
    }

    #[test]
    fn to_frontend_string_login_required_registry_url() {
        let host = quilt_uri::Host::from_str("catalog.dev").unwrap();
        let err = Error::Quilt(quilt::Error::Login(quilt::LoginError::RequiredRegistryUrl(
            host,
        )));
        let json: serde_json::Value = serde_json::from_str(&err.to_frontend_string()).unwrap();
        assert_eq!(json["kind"], "login_required");
        assert_eq!(json["host"], "catalog.dev");
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
        let host: quilt_uri::Host = "demo.quiltdata.com".parse().unwrap();
        let err = Error::Quilt(quilt::Error::S3(quilt::S3Error {
            host: Some(host.clone()),
            kind: quilt::S3ErrorKind::InvalidCredentials(
                "ExpiredToken: raw SDK details".to_string(),
            ),
        }));

        let json: serde_json::Value = serde_json::from_str(&err.to_frontend_string()).unwrap();
        assert_eq!(json["kind"], "login_required");
        assert_eq!(json["host"], host.to_string());
        assert_eq!(
            json["message"],
            format!("Your session for {host} has expired. Please sign in again.")
        );
        assert!(!json["message"].as_str().unwrap().contains("raw SDK"));
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
