use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use url::{form_urlencoded, Url};

use crate::Error;
const LATEST_TAG: &str = "latest";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "_tag", content = "value")]
pub enum RevisionPointer {
    Hash(String),
    Tag(String),
}

impl Default for RevisionPointer {
    fn default() -> Self {
        Self::Tag(String::from(LATEST_TAG))
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UriQuilt {
    pub domain: String,
    pub namespace: String,
    pub revision: RevisionPointer,
    pub path: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct UriParser {
    pub scheme: String,
    pub host: String,
    pub path: String,
    pub query: HashMap<String, String>,
    pub fragments: HashMap<String, String>,
    pub quilt: Option<UriQuilt>,
}
// TODO: replace with URL Crate

fn make_domain(uri_parser: &UriParser) -> String {
    let mut domain = uri_parser.scheme.clone();
    domain.push_str("://");
    domain.push_str(&uri_parser.host);
    domain.push_str(&uri_parser.path);
    domain
}

impl TryFrom<&UriParser> for UriQuilt {
    type Error = Error;

    fn try_from(uri_parser: &UriParser) -> Result<Self, Self::Error> {
        let domain = make_domain(uri_parser);
        let path = uri_parser
            .fragments
            .get("path")
            .unwrap_or(&"".to_string())
            .clone();
        let pkg_spec = uri_parser
            .fragments
            .get("package")
            .unwrap_or(&"".to_string())
            .clone();
        let (namespace, revision) = match pkg_spec.split_once(['@', ':']) {
            Some((namespace, top_hash)) => (
                namespace.to_string(),
                RevisionPointer::Hash(top_hash.into()),
            ),
            None => (pkg_spec, RevisionPointer::default()),
        };
        Ok(Self {
            domain,
            namespace,
            revision,
            path,
        })
    }
}

fn mapify(input: &str) -> HashMap<String, String> {
    form_urlencoded::parse(input.as_bytes())
        .into_owned()
        .collect()
}

fn normalize_input(input: &str) -> String {
    let split = input.split_once("://");
    if split.is_some() {
        return input.to_string();
    }
    let body = if input.starts_with('/') {
        input.to_string()
    } else {
        let cwd = std::env::current_dir().unwrap();
        if let Some(stripped) = input.strip_prefix("./") {
            let body = stripped.to_string();
            format!("{}/{}", cwd.to_string_lossy(), body)
        } else {
            format!("{}/{}", cwd.to_string_lossy(), input)
        }
    };
    format!("file://{}", body)
}

impl TryFrom<&str> for UriParser {
    type Error = Error;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        let uri_string = normalize_input(input);
        let parsed_url = Url::parse(&uri_string)?;
        let scheme = parsed_url.scheme().to_string();
        let is_quilt = scheme.starts_with("quilt+");
        let host = parsed_url.host_str().unwrap_or("").to_string();
        let path = parsed_url.path().to_string();
        let query_string = parsed_url.query().unwrap_or("");
        let query = mapify(query_string);
        let fragment_string = parsed_url.fragment().unwrap_or("");
        let fragments = mapify(fragment_string);

        let mut parsed: Self = Self {
            scheme,
            host,
            path,
            query,
            fragments,
            quilt: None,
        };
        if !is_quilt {
            return Ok(parsed);
        }
        let quilt: Option<UriQuilt> = UriQuilt::try_from(&parsed).ok();
        parsed.quilt = quilt;
        Ok(parsed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_from_uri() {
        // Test valid input
        let input =
            "quilt+s3://example.com?param1=value1&param2=value2#package=my/package&path=my_path";
        let result = UriParser::try_from(input);
        assert!(result.is_ok());
        let uri_parser = result.unwrap();
        assert_eq!(uri_parser.scheme, "quilt+s3");
        assert_eq!(uri_parser.host, "example.com");
        assert_eq!(uri_parser.query.get("param1"), Some(&"value1".to_string()));
        assert_eq!(uri_parser.query.get("param2"), Some(&"value2".to_string()));
        assert_eq!(uri_parser.path, "");
        assert_eq!(uri_parser.fragments.len(), 2);
        let quilt = uri_parser.quilt.unwrap();
        assert_eq!(quilt.domain, "quilt+s3://example.com");
        assert_eq!(quilt.namespace, "my/package");
        assert_eq!(quilt.revision, RevisionPointer::default());
        assert_eq!(quilt.path, "my_path");
    }

    #[test]
    fn test_try_from_relative() {
        // Test valid input
        let input = "./my_domain/folder";
        let result = UriParser::try_from(input);
        assert!(result.is_ok());
        let uri_parser = result.unwrap();
        assert_eq!(uri_parser.scheme, "file");
        assert_eq!(uri_parser.host, "");
        assert_eq!(uri_parser.query.len(), 0);
        assert!(uri_parser.path.ends_with("/my_domain/folder"));
        assert_eq!(uri_parser.fragments.len(), 0);
        assert_eq!(uri_parser.quilt, None);
    }
}
