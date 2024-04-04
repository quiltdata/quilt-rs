//!
//! UPath is a path abstraction that can be used to represent a path in a
//! local filesystem or remote object_store.
//! It is used to represent the path to a file or directory in a local domain,
//! or the path to an object or prefix in a remote object store.
//! It will eventually also support web and document stores.
//!

use object_store::path::Path;
use serde::Deserialize;
use serde::Serialize;
use std::fmt;
use std::path::PathBuf;
use url::Url;

use crate::Error;

use super::uri::UriParser;

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
pub enum UPath {
    Local(PathBuf),
    S3 {
        bucket: String,
        #[serde(skip)]
        path: Path,
    },
}

impl UPath {
    pub fn parse(uri_string: &str) -> Result<Self, Error> {
        let uri = UriParser::try_from(uri_string)?;
        match uri.scheme.as_str() {
            "file" => Ok(Self::Local(PathBuf::from(uri.path))),
            "s3" => Ok(Self::S3 {
                bucket: uri.host,
                path: Path::from(uri.path),
            }),
            _ => Err(Error::InvalidScheme(uri.scheme)),
        }
    }

    pub fn to_uri(&self) -> Url {
        match self {
            Self::Local(path) => Url::from_file_path(path).unwrap(),
            Self::S3 { bucket, path } => {
                let mut uri = Url::parse("s3://").unwrap();
                uri.set_host(Some(bucket)).unwrap();
                uri.set_path(path.as_ref());
                uri
            }
        }
    }

    pub fn join(&self, sub_path: &str) -> Self {
        let mut uri = self.to_uri();
        uri.set_path(&format!("{}/{}", uri.path(), sub_path));
        Self::parse(uri.as_ref()).unwrap()
    }
}

impl fmt::Display for UPath {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "UPath({})", self.to_uri())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tracing::log;

    #[test]
    fn test_new_local() {
        let local_uri = crate::utils::local_uri_parquet();
        let upath = UPath::parse(&local_uri).unwrap();
        log::info!("upath: {:?}", upath);
        assert_eq!(upath.to_uri().to_string(), local_uri);
        let upath_string = upath.to_string();
        assert_eq!(upath_string, format!("UPath({})", local_uri));
        let UPath::Local(fpath) = upath else { panic!() };
        assert_eq!(local_uri, format!("file://{}", fpath.to_string_lossy()));
    }

    #[test]
    fn test_new_s3() {
        let upath = UPath::parse(crate::utils::TEST_S3_URI).unwrap();
        assert_eq!(upath.to_uri().to_string(), crate::utils::TEST_S3_URI);
        assert_eq!(
            upath,
            UPath::S3 {
                bucket: "quilt-example".into(),
                path: "akarve/test_dest/README.md".into()
            }
        );
    }

    #[test]
    fn test_new_invalid() {
        UPath::parse("blah://123").expect_err("did not get an error");
    }

    #[test]
    fn test_formatting() -> Result<(), Error> {
        let upath = UPath::parse("file://missing/parent/child")?;
        assert_eq!(upath.to_string(), "UPath(file:///parent/child)".to_string());
        Ok(())
    }
}
