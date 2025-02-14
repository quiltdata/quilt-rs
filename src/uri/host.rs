use std::fmt;

use serde::Deserialize;
use serde::Serialize;
use std::str::FromStr;

use crate::Error;

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Host {
    #[serde(skip)]
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
    type Err = Error;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        url::Host::parse(s)
            .map_err(|e| Error::Host(e.to_string()))
            .map(Host::from)
    }
}

#[cfg(test)]
impl Default for Host {
    fn default() -> Self {
        Host {
            inner: url::Host::Domain("test.quilt.dev".to_string()),
        }
    }
}

impl fmt::Display for Host {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.inner)
    }
}
