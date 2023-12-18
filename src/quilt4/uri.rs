use std::collections::HashMap;

use serde::{Deserialize, Serialize};
use url::{Url, form_urlencoded};

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
pub struct UriParser {
    pub scheme: String,
    pub domain: String,
    pub query: HashMap<String, String>,
    pub namespace: String,
    pub revision: RevisionPointer,
    pub path: String,
    pub fragments: HashMap<String, String>,
}

fn mapify(input: &str) -> HashMap<String, String> {
    form_urlencoded::parse(input.as_bytes())
        .into_owned()
        .collect()
}

fn normalize_input(input: &str) -> String {
    let split = input.split_once("://");
    println!("split: {:?} <- {}", split, input);
    if split.is_some() {
        return input.to_string();
    }
    let body = if input.starts_with("/") {
        input.to_string()
    } else {
        let cwd = std::env::current_dir().unwrap();
        if input.starts_with("./") {
            let body = input[2..].to_string();
            format!("{}/{}", cwd.to_string_lossy(), body)
        } else {
            format!("{}/{}", cwd.to_string_lossy(), input)
        }
    };
    println!("body: {}", body);
    format!("file://localhost{}", body)
}

fn make_domain(parsed_url: &Url) -> String {
    let host = parsed_url.host_str().unwrap_or("");
    let path = parsed_url.path();
    if host.is_empty() {
        format!("file://localhost{}", path)
    } else {
        format!("{}://{}", parsed_url.scheme(), host)
    }
}

impl TryFrom<&str> for UriParser {
    type Error = String;

    fn try_from(input: &str) -> Result<Self, Self::Error> {

        let uri_string = normalize_input(input);
        println!("uri_string: {}", uri_string);
        let parsed_url = Url::parse(&uri_string).map_err(|err| err.to_string())?;
        println!("parsed_url: {:?}", parsed_url);
        let scheme = parsed_url.scheme().to_string();
        let domain = make_domain(&parsed_url);
        let query_string = parsed_url.query().unwrap_or("");
        println!("query_string: {}", query_string);
        let query = mapify(query_string);
        let fragment_string = parsed_url.fragment().unwrap_or("");
        println!("fragment_string: {}", fragment_string);
        let mut fragments = mapify(fragment_string);
        let pkg_spec = fragments.remove("package").unwrap_or("".to_string());
        let path = fragments.remove("path").unwrap_or("".to_string());

        let (namespace, revision) = match pkg_spec.split_once(['@',':']) {
            Some((namespace, top_hash)) => (
                namespace.to_string(),
                RevisionPointer::Hash(top_hash.into()),
            ),
            None => (pkg_spec, RevisionPointer::default()),
        };

        Ok(Self {
            scheme,
            domain,
            query,
            namespace,
            revision,
            path,
            fragments,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_try_from_uri() {
        // Test valid input
        let input = "quilt+s3://example.com?param1=value1&param2=value2#package=my_package&path=my_path";
        let result = UriParser::try_from(input);
        assert!(result.is_ok());
        let uri_parser = result.unwrap();
        assert_eq!(uri_parser.scheme, "quilt+s3");
        assert_eq!(uri_parser.domain, "quilt+s3://example.com");
        assert_eq!(uri_parser.query.get("param1"), Some(&"value1".to_string()));
        assert_eq!(uri_parser.query.get("param2"), Some(&"value2".to_string()));
        assert_eq!(uri_parser.namespace, "my_package");
        assert_eq!(uri_parser.revision, RevisionPointer::default());
        assert_eq!(uri_parser.path, "my_path");
        assert_eq!(uri_parser.fragments.len(), 0);
    }

    #[test]
    fn test_try_from_relative() {
        // Test valid input
        let input = "./my_domain/folder";
        let result = UriParser::try_from(input);
        assert!(result.is_ok());
        let uri_parser = result.unwrap();
        assert_eq!(uri_parser.scheme, "file");
        assert_eq!(uri_parser.domain, "file://localhost/Users/ernest/GitHub/quilt-rs/my_domain/folder");
        assert_eq!(uri_parser.query.len(), 0);
        assert_eq!(uri_parser.namespace, "");
        assert_eq!(uri_parser.revision, RevisionPointer::default());
        assert_eq!(uri_parser.path, "");
        assert_eq!(uri_parser.fragments.len(), 0);
    }

}
