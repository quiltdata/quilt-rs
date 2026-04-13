use crate::quilt;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Filesystem error: {0}")]
    FS(#[from] std::io::Error),

    #[error("Failed to parse JSON: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Failed to open: {0}")]
    Open(#[from] opener::OpenError),

    #[error("Failed using Quilt+S3 URI: {0}")]
    PackageUri(String),

    // TODO: move it to routes#Error and use RouteError(routes#Error)
    #[error("Page not found: {0}")]
    PageNotFound(String),

    // TODO: move it to routes#Error and use RouteError()
    #[error("Failed to parse page Url: {0}")]
    PageUrl(#[from] crate::routes::RouteError),

    #[error("Failed to parse {0}")]
    Parse(#[from] serde_qs::Error),

    #[error("Failed to parse Url: {0}")]
    ParseUrl(#[from] url::ParseError),

    #[error("Path {0} doesn't exist")]
    PathNotFound(std::path::PathBuf),

    #[error("Quilt error: {0}")]
    Quilt(quilt::Error),

    #[error("Tauri failed with {0}")]
    Tauri(#[from] tauri::Error),

    #[error("Test failed: {0}")]
    Test(String),

    #[error("Window not found")]
    Window,

    #[error("User cancelled operation")]
    UserCancelled,

    #[error("Commit error: {0}")]
    Commit(String),

    #[error("General error: {0}")]
    General(String),

    #[error("OAuth error: {0}")]
    OAuth(String),

    #[error("Mixpanel error: {0}")]
    Mixpanel(#[from] mixpanel_rs::error::Error),

    #[error("Mixpanel serialization error: {0}")]
    MixpanelSer(String),

    #[error("Package has no catalog origin")]
    MissingOrigin,

    #[error("Post-login navigation failed: {0}")]
    PostLogin(String),

    #[error("Zip error: {0}")]
    Zip(#[from] zip::result::ZipError),
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

impl From<String> for Error {
    fn from(s: String) -> Error {
        Error::General(s)
    }
}

impl Error {
    /// Serialize actionable errors as JSON so the frontend can parse and react
    /// (e.g. redirect to `/login` or `/setup`). Falls back to `Display` for
    /// all other errors.
    pub fn to_frontend_string(&self) -> String {
        match self {
            Error::Quilt(quilt::Error::LoginRequired(host)) => {
                serde_json::json!({
                    "kind": "login_required",
                    "message": self.to_string(),
                    "host": host.as_ref().map(|h| h.to_string()).unwrap_or_default(),
                })
                .to_string()
            }
            Error::Quilt(quilt::Error::LoginRequiredRegistryUrl(host)) => {
                serde_json::json!({
                    "kind": "login_required",
                    "message": self.to_string(),
                    "host": host.to_string(),
                })
                .to_string()
            }
            Error::Quilt(quilt::Error::LineageMissing | quilt::Error::LineageMissingHome) => {
                serde_json::json!({
                    "kind": "setup_required",
                    "message": self.to_string(),
                })
                .to_string()
            }
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
        let err = Error::Quilt(quilt::Error::LoginRequired(Some(host)));
        let json: serde_json::Value = serde_json::from_str(&err.to_frontend_string()).unwrap();
        assert_eq!(json["kind"], "login_required");
        assert_eq!(json["host"], "catalog.dev");
        assert!(json["message"].as_str().unwrap().contains("Login required"));
    }

    #[test]
    fn to_frontend_string_login_required_no_host() {
        let err = Error::Quilt(quilt::Error::LoginRequired(None));
        let json: serde_json::Value = serde_json::from_str(&err.to_frontend_string()).unwrap();
        assert_eq!(json["kind"], "login_required");
        assert_eq!(json["host"], "");
    }

    #[test]
    fn to_frontend_string_login_required_registry_url() {
        let host = quilt::uri::Host::from_str("catalog.dev").unwrap();
        let err = Error::Quilt(quilt::Error::LoginRequiredRegistryUrl(host));
        let json: serde_json::Value = serde_json::from_str(&err.to_frontend_string()).unwrap();
        assert_eq!(json["kind"], "login_required");
        assert_eq!(json["host"], "catalog.dev");
    }

    #[test]
    fn to_frontend_string_setup_required() {
        let err = Error::Quilt(quilt::Error::LineageMissing);
        let json: serde_json::Value = serde_json::from_str(&err.to_frontend_string()).unwrap();
        assert_eq!(json["kind"], "setup_required");

        let err = Error::Quilt(quilt::Error::LineageMissingHome);
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
