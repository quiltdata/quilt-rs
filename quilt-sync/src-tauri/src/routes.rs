use serde::Serialize;
use std::fmt;
use std::str;
use url::Url;

use crate::error::Error;
use crate::quilt;
use crate::telemetry::prelude::*;

/// Which entry categories are visible (checked) in the filter toolbar.
#[derive(Debug, Default, PartialEq, Clone, Serialize)]
pub struct EntriesFilter {
    pub unmodified: bool,
    pub ignored: bool,
}

impl EntriesFilter {
    /// Default for installed-package page: show unmodified, hide ignored.
    #[cfg(test)]
    pub fn for_installed_package() -> Self {
        Self {
            unmodified: true,
            ignored: false,
        }
    }

    /// Parse from a comma-separated string (e.g. "unmodified,ignored").
    /// Unknown tokens are silently ignored.
    pub fn from_filter_str(s: &str) -> Self {
        let mut f = Self::default();
        for token in s.split(',') {
            match token.trim() {
                "unmodified" => f.unmodified = true,
                "ignored" => f.ignored = true,
                _ => {}
            }
        }
        f
    }

}

impl fmt::Display for EntriesFilter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut first = true;
        if self.unmodified {
            write!(f, "unmodified")?;
            first = false;
        }
        if self.ignored {
            if !first {
                write!(f, ",")?;
            }
            write!(f, "ignored")?;
        }
        Ok(())
    }
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
}

/// Dummy base used to resolve relative URLs such as `"/settings"` that
/// come from `Paths::Display` when used as the `back` parameter.
const RELATIVE_URL_BASE: &str = "http://relative.invalid/";

fn parse_url(location: &str) -> Result<Url, Error> {
    Url::parse(location).or_else(|_| {
        let base = Url::parse(RELATIVE_URL_BASE).expect("constant base URL is valid");
        Ok(base.join(location)?)
    })
}

fn parse_page(location: &str) -> Result<String, Error> {
    let uri = parse_url(location)?;
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
struct QueryNamespaceParsed {
    pub namespace: String,
    #[serde(default)]
    pub filter: Option<String>,
}

fn parse_namespace(location: &str) -> Result<String, Error> {
    let uri = parse_url(location)?;
    match uri.query() {
        Some(q) => {
            let qs: QueryNamespaceParsed = serde_qs::from_str(q)?;
            Ok(qs.namespace)
        }
        None => Ok(String::new()),
    }
}

fn parse_filter(location: &str) -> Result<EntriesFilter, Error> {
    let uri = parse_url(location)?;
    match uri.query() {
        Some(q) => {
            let qs: QueryNamespaceParsed = serde_qs::from_str(q)?;
            Ok(qs
                .filter
                .map(|f| EntriesFilter::from_filter_str(&f))
                .unwrap_or_default())
        }
        None => Ok(EntriesFilter::default()),
    }
}

#[derive(Debug, serde::Deserialize)]
struct QueryLoginParsed {
    pub host: quilt::uri::Host,
    pub back: String,
}

fn parse_login(location: &str) -> Result<(quilt::uri::Host, String), Error> {
    let uri = parse_url(location)?;
    match uri.query() {
        Some(q) => {
            let qs: QueryLoginParsed = serde_qs::from_str(q)?;
            Ok((qs.host, qs.back))
        }
        None => Err(Error::PageUrl(RouteError::MissingHostFragment(uri))),
    }
}

#[derive(Debug, serde::Deserialize)]
struct QueryLoginErrorParsed {
    pub host: quilt::uri::Host,
    #[serde(default)]
    pub title: Option<String>,
    pub error: String,
}

fn parse_login_error(location: &str) -> Result<(quilt::uri::Host, Option<String>, String), Error> {
    let uri = parse_url(location)?;
    match uri.query() {
        Some(q) => {
            let qs: QueryLoginErrorParsed = serde_qs::from_str(q)?;
            Ok((qs.host, qs.title, qs.error))
        }
        None => Err(Error::PageUrl(RouteError::MissingHostFragment(uri))),
    }
}

#[derive(Debug, serde::Deserialize)]
struct QueryRemotePackage {
    pub uri: String,
}

fn parse_s3_package_uri(location: &str) -> Result<quilt::uri::S3PackageUri, Error> {
    let uri = parse_url(location)?;
    match uri.query() {
        Some(q) => {
            let qs: QueryRemotePackage = serde_qs::from_str(q)?;
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
    Commit(quilt::uri::Namespace, EntriesFilter),
    #[serde(rename = "installed_package")]
    InstalledPackage(quilt::uri::Namespace, EntriesFilter),
    #[serde(rename = "installed_packages_list")]
    InstalledPackagesList,
    #[serde(rename = "login")]
    Login(quilt::uri::Host, String),
    #[serde(rename = "login_error")]
    LoginError(quilt::uri::Host, String, String),
    #[serde(rename = "merge")]
    Merge(quilt::uri::Namespace),
    #[serde(rename = "remote_package")]
    RemotePackage(quilt::uri::S3PackageUri),
    #[serde(rename = "settings")]
    Settings,
    #[serde(rename = "setup")]
    Setup,
}

impl fmt::Display for Paths {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Paths::Commit(namespace, filter) => {
                let filter_str = filter.to_string();
                if filter_str.is_empty() {
                    write!(f, "/commit?namespace={namespace}")
                } else {
                    write!(f, "/commit?namespace={namespace}&filter={filter_str}")
                }
            }
            Paths::InstalledPackage(namespace, filter) => {
                let filter_str = filter.to_string();
                if filter_str.is_empty() {
                    write!(f, "/installed-package?namespace={namespace}")
                } else {
                    write!(
                        f,
                        "/installed-package?namespace={namespace}&filter={filter_str}"
                    )
                }
            }
            Paths::InstalledPackagesList => {
                write!(f, "/installed-packages-list")
            }
            Paths::Login(host, back) => {
                let back_encoded = urlencoding::encode(back);
                write!(f, "/login?host={host}&back={back_encoded}")
            }
            Paths::LoginError(host, title, error) => {
                let title_encoded = urlencoding::encode(title);
                let error_encoded = urlencoding::encode(error);
                write!(
                    f,
                    "/error?host={host}&title={title_encoded}&error={error_encoded}"
                )
            }
            Paths::Merge(namespace) => {
                write!(f, "/merge?namespace={namespace}")
            }
            Paths::RemotePackage(uri) => {
                let uri_str = uri.to_string();
                let uri_encoded = urlencoding::encode(&uri_str);
                write!(f, "/remote-package?uri={uri_encoded}")
            }
            Paths::Settings => {
                write!(f, "/settings")
            }
            Paths::Setup => {
                write!(f, "/setup")
            }
        }
    }
}

impl Paths {
    /// Returns the path name without any sensitive data values using serde serialization
    #[cfg(test)]
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

fn format_namespace_filter_query(
    namespace: &quilt::uri::Namespace,
    filter: &EntriesFilter,
) -> String {
    let filter_str = filter.to_string();
    if filter_str.is_empty() {
        format!("namespace={namespace}")
    } else {
        format!("namespace={namespace}&filter={filter_str}")
    }
}

pub fn from_url(path: Paths, mut url: Url) -> url::Url {
    url.set_fragment(None);
    match path {
        Paths::Commit(ref namespace, ref filter) => {
            url.set_path("/commit");
            url.set_query(Some(&format_namespace_filter_query(namespace, filter)));
            url
        }
        Paths::InstalledPackage(ref namespace, ref filter) => {
            url.set_path("/installed-package");
            url.set_query(Some(&format_namespace_filter_query(namespace, filter)));
            url
        }
        Paths::InstalledPackagesList => {
            url.set_path("/installed-packages-list");
            url.set_query(None);
            url
        }
        Paths::Login(host, ref back) => {
            let back_encoded = urlencoding::encode(back);
            url.set_path("/login");
            url.set_query(Some(&format!("host={host}&back={back_encoded}")));
            url
        }
        Paths::LoginError(host, ref title, ref error) => {
            let title_encoded = urlencoding::encode(title);
            let error_encoded = urlencoding::encode(error);
            url.set_path("/error");
            url.set_query(Some(&format!(
                "host={host}&title={title_encoded}&error={error_encoded}"
            )));
            url
        }
        Paths::Merge(namespace) => {
            url.set_path("/merge");
            url.set_query(Some(&format!("namespace={namespace}")));
            url
        }
        Paths::RemotePackage(uri) => {
            let uri_str = uri.to_string();
            let uri_encoded = urlencoding::encode(&uri_str);
            url.set_path("/remote-package");
            url.set_query(Some(&format!("uri={uri_encoded}")));
            url
        }
        Paths::Settings => {
            url.set_path("/settings");
            url.set_query(None);
            url
        }
        Paths::Setup => {
            url.set_path("/setup");
            url.set_query(None);
            url
        }
    }
}

impl str::FromStr for Paths {
    type Err = Error;

    fn from_str(location: &str) -> Result<Self, Self::Err> {
        let page = parse_page(location)?;
        match page.as_str() {
            "commit" => {
                let namespace = parse_namespace(location)?;
                let filter = parse_filter(location)?;
                Ok(Paths::Commit(namespace.try_into()?, filter))
            }
            "installed-package" => {
                let namespace = parse_namespace(location)?;
                let filter = parse_filter(location)?;
                Ok(Paths::InstalledPackage(namespace.try_into()?, filter))
            }
            "installed-packages-list" => Ok(Paths::InstalledPackagesList),
            "login" => {
                let (host, loc) = parse_login(location)?;
                Ok(Paths::Login(host, loc))
            }
            "error" => {
                let (host, title, error) = parse_login_error(location)?;
                let title = title.unwrap_or_else(|| "Login failed".into());
                Ok(Paths::LoginError(host, title, error))
            }
            "merge" => {
                let namespace = parse_namespace(location)?;
                Ok(Paths::Merge(namespace.try_into()?))
            }
            "remote-package" => {
                let uri = parse_s3_package_uri(location)?;
                Ok(Paths::RemotePackage(uri))
            }
            "settings" => Ok(Paths::Settings),
            "setup" => Ok(Paths::Setup),
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
            Paths::Commit(("foo", "bar").into(), EntriesFilter::default()),
            Url::parse("http://test:1234/")?,
        );
        let page_url_str = page_url.as_str();
        assert_eq!(
            page_url_str,
            "http://test:1234/commit?namespace=foo/bar"
        );

        let route: Paths = page_url_str.parse()?;

        assert_eq!(
            route,
            Paths::Commit(("foo", "bar").into(), EntriesFilter::default())
        );
        assert_eq!(format!("{route}"), "/commit?namespace=foo/bar");

        Ok(())
    }

    #[test]
    fn test_commit_with_filter() -> Result<()> {
        let filter = EntriesFilter {
            unmodified: true,
            ignored: true,
        };
        let page_url = from_url(
            Paths::Commit(("foo", "bar").into(), filter.clone()),
            Url::parse("http://test:1234/")?,
        );
        let page_url_str = page_url.as_str();
        assert_eq!(
            page_url_str,
            "http://test:1234/commit?namespace=foo/bar&filter=unmodified,ignored"
        );

        let route: Paths = page_url_str.parse()?;
        assert_eq!(route, Paths::Commit(("foo", "bar").into(), filter));

        Ok(())
    }

    #[test]
    fn test_installed_package() -> Result<()> {
        let page_url = from_url(
            Paths::InstalledPackage(("foo", "bar").into(), EntriesFilter::default()),
            Url::parse("http://test:1234/")?,
        );
        let page_url_str = page_url.as_str();
        assert_eq!(
            page_url_str,
            "http://test:1234/installed-package?namespace=foo/bar"
        );

        let route: Paths = page_url_str.parse()?;

        assert_eq!(
            route,
            Paths::InstalledPackage(("foo", "bar").into(), EntriesFilter::default())
        );
        assert_eq!(
            format!("{route}"),
            "/installed-package?namespace=foo/bar"
        );

        Ok(())
    }

    #[test]
    fn test_installed_package_with_filter() -> Result<()> {
        let filter = EntriesFilter::for_installed_package();
        let page_url = from_url(
            Paths::InstalledPackage(("foo", "bar").into(), filter.clone()),
            Url::parse("http://test:1234/")?,
        );
        let page_url_str = page_url.as_str();
        assert_eq!(
            page_url_str,
            "http://test:1234/installed-package?namespace=foo/bar&filter=unmodified"
        );

        let route: Paths = page_url_str.parse()?;
        assert_eq!(
            route,
            Paths::InstalledPackage(("foo", "bar").into(), filter)
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
            "http://test:1234/installed-packages-list"
        );

        let route: Paths = page_url_str.parse()?;

        assert_eq!(route, Paths::InstalledPackagesList);
        assert_eq!(format!("{route}"), "/installed-packages-list");

        Ok(())
    }

    #[test]
    fn test_login() -> Result<()> {
        let host: Host = "test.quilt.dev".parse()?;
        let back = Paths::InstalledPackagesList.to_string();
        let page_url = from_url(
            Paths::Login(host.clone(), back.clone()),
            Url::parse("http://test:1234/")?,
        );
        let page_url_str = page_url.as_str();
        assert_eq!(
            page_url_str,
            "http://test:1234/login?host=test.quilt.dev&back=%2Finstalled-packages-list"
        );

        let route: Paths = page_url_str.parse()?;

        assert_eq!(route, Paths::Login(host, back));
        assert_eq!(
            format!("{route}"),
            "/login?host=test.quilt.dev&back=%2Finstalled-packages-list"
        );

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
            .starts_with("http://test:1234/error?host=test.quilt.dev&title="));

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
        let url = format!(
            "http://test:1234/error?host={host}&error={}",
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
            "http://test:1234/merge?namespace=foo/bar"
        );

        let route: Paths = page_url_str.parse()?;

        assert_eq!(route, Paths::Merge(("foo", "bar").into()));
        assert_eq!(format!("{route}"), "/merge?namespace=foo/bar");

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
            "http://test:1234/remote-package?uri=quilt%2Bs3%3A%2F%2Ftest%23package%3Dfoo%2Fbar"
        );

        let route: Paths = page_url_str.parse()?;

        assert_eq!(route, Paths::RemotePackage(uri));
        assert_eq!(
            format!("{route}"),
            "/remote-package?uri=quilt%2Bs3%3A%2F%2Ftest%23package%3Dfoo%2Fbar"
        );

        Ok(())
    }

    #[test]
    fn test_settings() -> Result<()> {
        let page_url = from_url(Paths::Settings, Url::parse("http://test:1234/")?);
        let page_url_str = page_url.as_str();
        assert_eq!(page_url_str, "http://test:1234/settings");

        let route: Paths = page_url_str.parse()?;

        assert_eq!(route, Paths::Settings);
        assert_eq!(format!("{route}"), "/settings");

        Ok(())
    }

    #[test]
    fn test_setup() -> Result<()> {
        let page_url = from_url(Paths::Setup, Url::parse("http://test:1234/")?);
        let page_url_str = page_url.as_str();
        assert_eq!(page_url_str, "http://test:1234/setup");

        let route: Paths = page_url_str.parse()?;

        assert_eq!(route, Paths::Setup);
        assert_eq!(format!("{route}"), "/setup");

        Ok(())
    }

    #[test]
    fn test_pathname_privacy() -> Result<()> {
        let commit_path =
            Paths::Commit(("sensitive", "namespace").into(), EntriesFilter::default());
        assert_eq!(commit_path.pathname(), "commit");

        let login_path = Paths::Login("sensitive.host.com".parse()?, "/secret-page".into());
        assert_eq!(login_path.pathname(), "login");

        let installed_package_path =
            Paths::InstalledPackage(("private", "package").into(), EntriesFilter::default());
        assert_eq!(installed_package_path.pathname(), "installed_package");

        let list_path = Paths::InstalledPackagesList;
        assert_eq!(list_path.pathname(), "installed_packages_list");

        let merge_path = Paths::Merge(("secret", "repo").into());
        assert_eq!(merge_path.pathname(), "merge");

        let settings_path = Paths::Settings;
        assert_eq!(settings_path.pathname(), "settings");

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

        let setup_path = Paths::Setup;
        let serialized = serde_json::to_value(&setup_path)?;
        match serialized {
            Value::Object(map) => {
                assert_eq!(map.get("t"), Some(&Value::String("setup".to_string())));
            }
            _ => panic!("Expected adjacently tagged object for Setup"),
        }
        assert_eq!(setup_path.pathname(), "setup");

        let commit_path = Paths::Commit(("test", "package").into(), EntriesFilter::default());
        let serialized = serde_json::to_value(&commit_path)?;
        match serialized {
            Value::Object(map) => {
                assert_eq!(map.get("t"), Some(&Value::String("commit".to_string())));
                assert!(map.contains_key("c"));
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

    #[test]
    fn test_login_with_encoded_back() -> Result<()> {
        let host: Host = "test.quilt.dev".parse()?;
        let back = Paths::InstalledPackage(
            ("foo", "bar").into(),
            EntriesFilter::for_installed_package(),
        )
        .to_string();
        let page_url = from_url(
            Paths::Login(host.clone(), back.clone()),
            Url::parse("http://test:1234/")?,
        );
        let route: Paths = page_url.as_str().parse()?;
        assert_eq!(route, Paths::Login(host, back));
        Ok(())
    }
}
