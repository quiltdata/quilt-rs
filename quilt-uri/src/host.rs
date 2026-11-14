use std::fmt;

use serde::Deserialize;
use serde::Serialize;
use std::str::FromStr;

use crate::UriError;

mod normalize;

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

/// Any `url::Host` becomes a `Host`, an IP address included. The rule for a typed
/// name ([`Host::from_user_input`]) is not applied here.
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
        normalize::stored(s).map(Host::from)
    }
}

impl Host {
    /// The host a person typed or pasted, such as the catalog in a login form.
    ///
    /// This is the strict door: it follows quiltx's rule for naming a catalog, and
    /// refuses an IP address, a port, a percent escape and any scheme but `https`.
    /// A name read back from disk or from a URI goes through [`FromStr`] instead,
    /// which also takes what was accepted before, so one such host cannot make a
    /// stored account or a whole lineage unreadable.
    ///
    /// # Errors
    ///
    /// [`UriError::Host`] when `input` is not a catalog host name.
    pub fn from_user_input(input: &str) -> Result<Self, UriError> {
        normalize::typed(input).map(Host::from)
    }

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
    fn test_host_from_str_and_deserialize_share_one_rule() {
        let typed = Host::from_str("Quilt.TEST").unwrap();
        let stored: Host = serde_json::from_str("\"Quilt.TEST\"").unwrap();
        assert_eq!(typed, stored);
        assert_eq!(typed.to_string(), "quilt.test");
    }

    #[test]
    fn test_host_from_user_input_is_stricter_than_from_str() {
        // A typed name follows quiltx's rule; a stored one also takes what was
        // read before, so one stored IP address cannot hide an account or a lineage.
        assert_eq!(
            Host::from_user_input(" https://Quilt.TEST/ ")
                .unwrap()
                .to_string(),
            "quilt.test"
        );
        assert!(Host::from_user_input("127.0.0.1").is_err());
        assert_eq!(
            Host::from_str("127.0.0.1").unwrap().to_string(),
            "127.0.0.1"
        );
        let stored: Host = serde_json::from_str("\"127.0.0.1\"").unwrap();
        assert_eq!(stored.to_string(), "127.0.0.1");
        assert_eq!(Host::from_str("[::1]").unwrap().to_string(), "[::1]");
    }

    #[test]
    fn test_host_unicode_round_trips_through_its_display() {
        // The folder name written for a typed name is read back as the same host.
        let typed = Host::from_user_input("bücher.example").unwrap();
        assert_eq!(typed.to_string(), "bücher.example");
        assert_eq!(Host::from_str(&typed.to_string()).unwrap(), typed);
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
