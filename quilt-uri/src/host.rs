use std::fmt;

use serde::Deserialize;
use serde::Serialize;
use std::str::FromStr;

use crate::UriError;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Host {
    inner: url::Host,
}

impl Serialize for Host {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

impl<'de> Deserialize<'de> for Host {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        Host::from_str(&s).map_err(serde::de::Error::custom)
    }
}

impl From<url::Host> for Host {
    fn from(inner: url::Host) -> Self {
        Host { inner }
    }
}

impl From<Host> for url::Host {
    fn from(h: Host) -> Self {
        h.inner
    }
}

impl FromStr for Host {
    type Err = UriError;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        url::Host::parse(s)
            .map_err(|e| UriError::Host(e.to_string()))
            .map(Host::from)
    }
}

impl Host {
    /// `https://{host}/` with each of `segments` as one escaped path segment.
    ///
    /// A `.` or `..` segment is an error: a browser reads it, and its escaped
    /// form `%2E`, as a step up the path, so no URL keeps it.
    pub(crate) fn web_url<'a>(
        &self,
        segments: impl IntoIterator<Item = &'a str>,
    ) -> Result<url::Url, String> {
        let mut url = url::Url::parse(&format!("https://{self}/")).map_err(|e| e.to_string())?;
        {
            let mut path = url
                .path_segments_mut()
                .map_err(|()| format!("{self} cannot hold a path"))?;
            path.pop_if_empty();
            for segment in segments {
                if segment == "." || segment == ".." {
                    return Err(format!(
                        "cannot link to a `{segment}` path segment in the catalog"
                    ));
                }
                path.push(segment);
            }
        }
        Ok(url)
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures;

    #[test]
    fn test_host_from_str_valid() {
        let host = Host::from_str("quilt.test").unwrap();
        assert_eq!(host.to_string(), "quilt.test");
    }

    #[test]
    fn test_host_from_str_invalid() {
        // Unterminated IPv6 literal is rejected by `url::Host::parse`.
        let result = Host::from_str("[::1");
        assert!(result.is_err());
        assert!(matches!(result.unwrap_err(), UriError::Host(_)));
    }

    #[test]
    fn test_host_serde_round_trip() {
        let host = fixtures::host();
        let json = serde_json::to_string(&host).unwrap();
        assert_eq!(json, "\"quilt.test\"");
        let parsed: Host = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, host);
    }

    #[test]
    fn test_host_deserialize_invalid() {
        let result: Result<Host, _> = serde_json::from_str("\"[::1\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_host_url_host_round_trip() {
        let host = fixtures::host();
        let inner: url::Host = host.clone().into();
        let back: Host = inner.into();
        assert_eq!(back, host);
    }
}
