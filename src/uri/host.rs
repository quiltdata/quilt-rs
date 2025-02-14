use std::fmt;

use serde::Deserialize;
use serde::Serialize;
use std::str::FromStr;

use crate::Error;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Hash)]
pub struct Host {
    inner: url::Host,
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
