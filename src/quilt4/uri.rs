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
    if split.is_some() {
        return input.to_string();
    }
    let body = if input.starts_with("/") {
        input.to_string()
    } else {
        let cwd = std::env::current_dir().unwrap();
        cwd.join(input).canonicalize().unwrap().to_string_lossy().to_string()
    };
    format!("file://localhost{}", body)
}
impl TryFrom<&str> for UriParser {
    type Error = String;

    fn try_from(input: &str) -> Result<Self, Self::Error> {

        let uri_string = normalize_input(input);
        let parsed_url = Url::parse(&uri_string).map_err(|err| err.to_string())?;
        let scheme = parsed_url.scheme().to_string();
        let domain = parsed_url.host_str().ok_or("localhost")?.to_string();
        let query = mapify(parsed_url.query().ok_or("")?);
        let mut fragments = mapify(parsed_url.fragment().ok_or("")?);
        let pkg_spec = fragments.remove("package").ok_or("")?;
        let path = fragments.remove("path").ok_or("")?;

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
