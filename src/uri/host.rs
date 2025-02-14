use serde::Deserialize;
use serde::Serialize;

use crate::Error;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
