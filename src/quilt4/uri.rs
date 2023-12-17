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
    pub namespace: Option<String>,
    pub revision: Option<RevisionPointer>,
    pub query: Option<HashMap<String, String>>,
    pub fragments: Option<HashMap<String, String>>,
}

impl TryFrom<&str> for UriParser {
    type Error = String;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        let parsed_url = Url::parse(input).map_err(|err| err.to_string())?;

        let fragment = parsed_url.fragment().ok_or("missing fragment")?;
        let mut params: HashMap<_, _> = form_urlencoded::parse(fragment.as_bytes())
            .into_owned()
            .collect();

        let pkg_spec = params
            .remove("package")
            .ok_or("fragment must contain package")?;

        let (namespace, revision) = match pkg_spec.split_once('@') {
            Some((namespace, top_hash)) => (
                namespace.to_string(),
                RevisionPointer::Hash(top_hash.into()),
            ),
            None => (pkg_spec, RevisionPointer::default()),
        };

        let path = params.remove("path");

        if !params.is_empty() {
            return Err(format!("unexpected fragment params: {:?}", params));
        }

        let domain = parsed_url.host_str().ok_or("missing domain")?.to_string();

        Ok(Self {
            scheme: parsed_url.scheme().to_string(),
            domain,
            namespace,
            revision,
            path,
            query: None,
        })
    }
}
