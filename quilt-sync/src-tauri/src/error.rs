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

    #[error("Package has no catalog origin")]
    MissingOrigin,

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

impl From<quilt::UriError> for Error {
    fn from(err: quilt::UriError) -> Error {
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
            _ => self.to_string(),
        }
    }
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
        let host = quilt::uri::Host::from_str("catalog.dev").unwrap();
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
        let host = quilt::uri::Host::from_str("catalog.dev").unwrap();
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
}
