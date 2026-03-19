use serde::Serialize;
use std::fmt;
use std::str;
use url::Url;

use crate::error::Error;
use crate::quilt;
use crate::telemetry::prelude::*;

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
}

fn parse_page(location: &str) -> Result<String, Error> {
    let uri = Url::parse(location)?;
    // NOTE: it is a temporary variable just to get the last element of it
    let mut segments = match uri.path_segments() {
        Some(segments) => segments,
        None => return Err(Error::PageUrl(RouteError::NoPathSegments(uri))),
    };
    let page = match segments.next_back() {
        Some(page) => page,
        None => return Err(Error::PageUrl(RouteError::NoPageInPath(uri))),
    };
    Ok(page.to_string())
}

#[derive(Debug, serde::Deserialize)]
struct FragmentNamespaceParsed {
    pub namespace: String,
}

fn parse_namespace(location: &str) -> Result<String, Error> {
    let uri = Url::parse(location)?;
    let namespace = match uri.fragment() {
        Some(n) => {
            let qs: FragmentNamespaceParsed = serde_qs::from_str(n)?;
            qs.namespace
        }
        None => "".to_string(),
    };
    Ok(namespace)
}

#[derive(Debug, serde::Deserialize)]
struct FragmentHostParsed {
    pub host: quilt::uri::Host,
}

fn parse_host(location: &str) -> Result<quilt::uri::Host, Error> {
    let uri = Url::parse(location)?;
    match uri.fragment() {
        Some(n) => {
            let qs: FragmentHostParsed = serde_qs::from_str(n)?;
            Ok(qs.host)
        }
        None => Err(Error::PageUrl(RouteError::MissingHostFragment(uri))),
    }
}

#[derive(Debug, serde::Deserialize)]
struct FragmentLoginErrorParsed {
    pub host: quilt::uri::Host,
    #[serde(default)]
    pub title: Option<String>,
    pub error: String,
}

fn parse_login_error(location: &str) -> Result<(quilt::uri::Host, Option<String>, String), Error> {
    let uri = Url::parse(location)?;
    match uri.fragment() {
        Some(n) => {
            let qs: FragmentLoginErrorParsed = serde_qs::from_str(n)?;
            Ok((qs.host, qs.title, qs.error))
        }
        None => Err(Error::PageUrl(RouteError::MissingHostFragment(uri))),
    }
}

#[derive(Debug, serde::Deserialize)]
struct FragmentRemotePackage {
    pub uri: String,
}

fn parse_s3_package_uri(location: &str) -> Result<quilt::uri::S3PackageUri, Error> {
    let uri = Url::parse(location)?;
    match uri.query() {
        Some(n) => {
            // TODO: replace unwrap() with ? to avoid a panic on a malformed uri query param
            let qs: FragmentRemotePackage = serde_qs::from_str(n).unwrap();
            debug!("Pre-parsed URI is {}", qs.uri);
            Ok(quilt::uri::S3PackageUri::try_from(qs.uri.as_str())?)
        }
        None => Err(Error::PageUrl(RouteError::MissingS3UriQuery(uri))),
    }
}

#[derive(Debug, PartialEq, Clone, Serialize)]
#[serde(tag = "t", content = "c")]
pub enum Paths {
    #[serde(rename = "commit")]
    Commit(quilt::uri::Namespace),
    #[serde(rename = "installed_package")]
    InstalledPackage(quilt::uri::Namespace),
    #[serde(rename = "installed_packages_list")]
    InstalledPackagesList,
    #[serde(rename = "login")]
    Login(quilt::uri::Host),
    #[serde(rename = "login_error")]
    LoginError(quilt::uri::Host, String, String),
    #[serde(rename = "merge")]
    Merge(quilt::uri::Namespace),
    #[serde(rename = "remote_package")]
    RemotePackage(quilt::uri::S3PackageUri),
    #[serde(rename = "setup")]
    Setup,
}

impl fmt::Display for Paths {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Paths::Commit(namespace) => {
                write!(f, "commit.html#namespace={namespace}")
            }
            Paths::InstalledPackage(namespace) => {
                write!(f, "installed-package.html#namespace={namespace}")
            }
            Paths::InstalledPackagesList => {
                write!(f, "installed-packages-list.html")
            }
            Paths::Login(host) => {
                write!(f, "login.html#host={host}")
            }
            Paths::LoginError(host, title, error) => {
                let title_encoded = urlencoding::encode(title);
                let error_encoded = urlencoding::encode(error);
                write!(
                    f,
                    "login-error.html#host={host}&title={title_encoded}&error={error_encoded}"
                )
            }
            Paths::Merge(namespace) => {
                write!(f, "merge.html#namespace={namespace}")
            }
            Paths::RemotePackage(uri) => {
                let uri_str = uri.to_string();
                let uri_encoded = urlencoding::encode(&uri_str);
                write!(f, "remote-package.html?uri={uri_encoded}")
            }
            Paths::Setup => {
                write!(f, "setup.html")
            }
        }
    }
}

impl Paths {
    /// Returns the path name without any sensitive data values using serde serialization
    pub fn pathname(&self) -> String {
        use serde_json::Value;

        // Serialize the enum to JSON and extract the variant name from adjacently tagged format
        match serde_json::to_value(self) {
            Ok(Value::Object(map)) => {
                // Adjacently tagged format: {"t": "pathname", "c": ...} or {"t": "pathname"}
                let Some(Value::String(tag)) = map.get("t") else {
                    return "unknown".to_string();
                };
                tag.clone()
            }
            _ => "unknown".to_string(),
        }
    }
}

pub fn from_url(path: Paths, mut url: Url) -> url::Url {
    match path {
        Paths::Commit(namespace) => {
            url.set_path("pages/commit.html");
            url.set_fragment(Some(&format!("namespace={namespace}")));
            url
        }
        Paths::InstalledPackage(namespace) => {
            url.set_path("pages/installed-package.html");
            url.set_fragment(Some(&format!("namespace={namespace}")));
            url
        }
        Paths::InstalledPackagesList => {
            url.set_path("pages/installed-packages-list.html");
            url
        }
        Paths::Login(host) => {
            url.set_path("pages/login.html");
            url.set_fragment(Some(&format!("host={host}")));
            url
        }
        Paths::LoginError(host, ref title, ref error) => {
            let title_encoded = urlencoding::encode(title);
            let error_encoded = urlencoding::encode(error);
            url.set_path("pages/login-error.html");
            url.set_fragment(Some(&format!(
                "host={host}&title={title_encoded}&error={error_encoded}"
            )));
            url
        }
        Paths::Merge(namespace) => {
            url.set_path("pages/merge.html");
            url.set_fragment(Some(&format!("namespace={namespace}")));
            url
        }
        Paths::RemotePackage(uri) => {
            let uri_str = uri.to_string();
            let uri_encoded = urlencoding::encode(&uri_str);
            url.set_path("pages/remote-package.html");
            url.set_query(Some(&format!("uri={uri_encoded}")));
            url
        }
        Paths::Setup => {
            url.set_path("pages/setup.html");
            url
        }
    }
}

impl str::FromStr for Paths {
    type Err = Error;

    fn from_str(location: &str) -> Result<Self, Self::Err> {
        let page = parse_page(location)?;
        match page.as_str() {
            "commit.html" => {
                let namespace = parse_namespace(location)?;
                Ok(Paths::Commit(namespace.try_into()?))
            }
            "installed-package.html" => {
                let namespace = parse_namespace(location)?;
                Ok(Paths::InstalledPackage(namespace.try_into()?))
            }
            "installed-packages-list.html" => Ok(Paths::InstalledPackagesList),
            "login.html" => {
                let host = parse_host(location)?;
                Ok(Paths::Login(host))
            }
            "login-error.html" => {
                let (host, title, error) = parse_login_error(location)?;
                let title = title.unwrap_or_else(|| "Login failed".into());
                Ok(Paths::LoginError(host, title, error))
            }
            "merge.html" => {
                let namespace = parse_namespace(location)?;
                Ok(Paths::Merge(namespace.try_into()?))
            }
            "remote-package.html" => {
                let uri = parse_s3_package_uri(location)?;
                Ok(Paths::RemotePackage(uri))
            }
            "setup.html" => Ok(Paths::Setup),
            _ => Err(Error::PageNotFound(page)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use quilt::uri::Host;

    use crate::Result;

    #[test]
    fn test_commit() -> Result<()> {
        let page_url = from_url(
            Paths::Commit(("foo", "bar").into()),
            Url::parse("http://test:1234/")?,
        );
        let page_url_str = page_url.as_str();
        assert_eq!(
            page_url_str,
            "http://test:1234/pages/commit.html#namespace=foo/bar"
        );

        let route: Paths = page_url_str.parse()?;

        assert_eq!(route, Paths::Commit(("foo", "bar").into()));
        assert_eq!(format!("{route}"), "commit.html#namespace=foo/bar");

        Ok(())
    }

    #[test]
    fn test_installed_package() -> Result<()> {
        let page_url = from_url(
            Paths::InstalledPackage(("foo", "bar").into()),
            Url::parse("http://test:1234/")?,
        );
        let page_url_str = page_url.as_str();
        assert_eq!(
            page_url_str,
            "http://test:1234/pages/installed-package.html#namespace=foo/bar"
        );

        let route: Paths = page_url_str.parse()?;

        assert_eq!(route, Paths::InstalledPackage(("foo", "bar").into()));
        assert_eq!(
            format!("{route}"),
            "installed-package.html#namespace=foo/bar"
        );

        Ok(())
    }

    #[test]
    fn test_installed_packages_list() -> Result<()> {
        let page_url = from_url(
            Paths::InstalledPackagesList,
            Url::parse("http://test:1234/")?,
        );
        let page_url_str = page_url.as_str();
        assert_eq!(
            page_url_str,
            "http://test:1234/pages/installed-packages-list.html"
        );

        let route: Paths = page_url_str.parse()?;

        assert_eq!(route, Paths::InstalledPackagesList);
        assert_eq!(format!("{route}"), "installed-packages-list.html");

        Ok(())
    }

    #[test]
    fn test_login() -> Result<()> {
        let host: Host = "test.quilt.dev".parse()?;
        let page_url = from_url(Paths::Login(host.clone()), Url::parse("http://test:1234/")?);
        let page_url_str = page_url.as_str();
        assert_eq!(
            page_url_str,
            "http://test:1234/pages/login.html#host=test.quilt.dev"
        );

        let route: Paths = page_url_str.parse()?;

        assert_eq!(route, Paths::Login(host));
        assert_eq!(format!("{route}"), "login.html#host=test.quilt.dev");

        Ok(())
    }

    #[test]
    fn test_login_error() -> Result<()> {
        let host: Host = "test.quilt.dev".parse()?;
        let title = "Login failed";
        let error = "Auth failed: invalid_grant (token expired)";
        let page_url = from_url(
            Paths::LoginError(host.clone(), title.to_string(), error.to_string()),
            Url::parse("http://test:1234/")?,
        );
        let page_url_str = page_url.as_str();
        assert!(page_url_str
            .starts_with("http://test:1234/pages/login-error.html#host=test.quilt.dev&title="));

        let route: Paths = page_url_str.parse()?;
        assert_eq!(
            route,
            Paths::LoginError(host, title.to_string(), error.to_string())
        );

        Ok(())
    }

    #[test]
    fn test_login_error_without_title_defaults_to_login_failed() -> Result<()> {
        let host: Host = "test.quilt.dev".parse()?;
        let error = "Auth failed";
        // Old URL format without title — must parse without panicking and default gracefully.
        let url = format!(
            "http://test:1234/pages/login-error.html#host={host}&error={}",
            urlencoding::encode(error)
        );
        let route: Paths = url.parse()?;
        assert_eq!(
            route,
            Paths::LoginError(host, "Login failed".to_string(), error.to_string())
        );
        Ok(())
    }

    #[test]
    fn test_merge() -> Result<()> {
        let page_url = from_url(
            Paths::Merge(("foo", "bar").into()),
            Url::parse("http://test:1234/")?,
        );
        let page_url_str = page_url.as_str();
        assert_eq!(
            page_url_str,
            "http://test:1234/pages/merge.html#namespace=foo/bar"
        );

        let route: Paths = page_url_str.parse()?;

        assert_eq!(route, Paths::Merge(("foo", "bar").into()));
        assert_eq!(format!("{route}"), "merge.html#namespace=foo/bar");

        Ok(())
    }

    #[test]
    fn test_remote_package() -> Result<()> {
        let uri = quilt::uri::S3PackageUri::try_from("quilt+s3://test#package=foo/bar")?;
        let page_url = from_url(
            Paths::RemotePackage(uri.clone()),
            Url::parse("http://test:1234/")?,
        );
        let page_url_str = page_url.as_str();
        assert_eq!(
            page_url_str,
            "http://test:1234/pages/remote-package.html?uri=quilt%2Bs3%3A%2F%2Ftest%23package%3Dfoo%2Fbar"
        );

        let route: Paths = page_url_str.parse()?;

        assert_eq!(route, Paths::RemotePackage(uri));
        assert_eq!(
            format!("{route}"),
            "remote-package.html?uri=quilt%2Bs3%3A%2F%2Ftest%23package%3Dfoo%2Fbar"
        );

        Ok(())
    }

    #[test]
    fn test_setup() -> Result<()> {
        let page_url = from_url(Paths::Setup, Url::parse("http://test:1234/")?);
        let page_url_str = page_url.as_str();
        assert_eq!(page_url_str, "http://test:1234/pages/setup.html");

        let route: Paths = page_url_str.parse()?;

        assert_eq!(route, Paths::Setup);
        assert_eq!(format!("{route}"), "setup.html");

        Ok(())
    }

    #[test]
    fn test_pathname_privacy() -> Result<()> {
        // Test that pathname() returns only the variant name without sensitive data (in snake_case)
        let commit_path = Paths::Commit(("sensitive", "namespace").into());
        assert_eq!(commit_path.pathname(), "commit");

        let login_path = Paths::Login("sensitive.host.com".parse()?);
        assert_eq!(login_path.pathname(), "login");

        let installed_package_path = Paths::InstalledPackage(("private", "package").into());
        assert_eq!(installed_package_path.pathname(), "installed_package");

        let list_path = Paths::InstalledPackagesList;
        assert_eq!(list_path.pathname(), "installed_packages_list");

        let merge_path = Paths::Merge(("secret", "repo").into());
        assert_eq!(merge_path.pathname(), "merge");

        let setup_path = Paths::Setup;
        assert_eq!(setup_path.pathname(), "setup");

        // Verify that Display still contains sensitive data (for non-tracking purposes)
        assert!(commit_path.to_string().contains("sensitive/namespace"));
        // But pathname() does not
        assert!(!commit_path.pathname().contains("sensitive"));
        assert!(!commit_path.pathname().contains("namespace"));

        Ok(())
    }

    #[test]
    fn test_serde_pathname() -> Result<()> {
        use serde_json::Value;

        // Test that serde serialization produces the expected snake_case variant names
        let setup_path = Paths::Setup;
        let serialized = serde_json::to_value(&setup_path)?;
        match serialized {
            Value::Object(map) => {
                assert_eq!(map.get("t"), Some(&Value::String("setup".to_string())));
            }
            _ => panic!("Expected adjacently tagged object for Setup"),
        }
        assert_eq!(setup_path.pathname(), "setup");

        let commit_path = Paths::Commit(("test", "package").into());
        let serialized = serde_json::to_value(&commit_path)?;
        match serialized {
            Value::Object(map) => {
                assert_eq!(map.get("t"), Some(&Value::String("commit".to_string())));
                assert!(map.contains_key("c")); // Should have content
                assert_eq!(commit_path.pathname(), "commit");
            }
            _ => panic!("Expected adjacently tagged object for Commit"),
        }

        let list_path = Paths::InstalledPackagesList;
        let serialized = serde_json::to_value(&list_path)?;
        match serialized {
            Value::Object(map) => {
                assert_eq!(
                    map.get("t"),
                    Some(&Value::String("installed_packages_list".to_string()))
                );
            }
            _ => panic!("Expected adjacently tagged object for InstalledPackagesList"),
        }
        assert_eq!(list_path.pathname(), "installed_packages_list");

        Ok(())
    }
}
