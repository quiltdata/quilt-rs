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

    #[error("Failed rendering template: {0}")]
    Template(#[from] askama::Error),

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

    #[error("Mixpanel error: {0}")]
    Mixpanel(#[from] mixpanel_rs::error::Error),

    #[error("Mixpanel serialization error: {0}")]
    MixpanelSer(String),

    #[error("Package has no catalog origin")]
    MissingOrigin,
}

impl From<quilt::Error> for Error {
    fn from(err: quilt::Error) -> Error {
        Error::Quilt(err)
    }
}

impl From<String> for Error {
    fn from(s: String) -> Error {
        Error::General(s)
    }
}

impl From<Error> for String {
    fn from(err: Error) -> String {
        format!("{err}")
    }
}
