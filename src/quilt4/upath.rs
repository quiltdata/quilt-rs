//! 
//! UPath is a path abstraction that can be used to represent a path in a
//! local filesystem or remote object_store.
//! It is used to represent the path to a file or directory in a local domain,
//! or the path to an object or prefix in a remote object store.
//! It will eventually also support web and document stores.
//! 

use std::path::PathBuf;
use object_store::path::Path;
use url::Url;
use std::io;
use multihash::Multihash;
use serde::{Deserialize, Serialize};

use super::client::Client;
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
    pub fn parse(uri_string: &str) -> Result<Self, String> {
        let uri = UriParser::try_from(uri_string)?;
        match uri.scheme.as_str() {
            "file" => Ok(Self::Local(PathBuf::from(uri.path))),
            "s3" => Ok(Self::S3 {
                bucket: uri.host,
                path: Path::from(uri.path),
            }),
            _ => Err("unsupported scheme".into()),
        }
    }

    pub fn to_uri(&self) -> Url {
        match self {
            Self::Local(path) => Url::from_file_path(path).unwrap(),
            Self::S3 { bucket, path } => {
                let mut uri = Url::parse("s3://").unwrap();
                uri.set_host(Some(bucket)).unwrap();
                uri.set_path(&path.to_string());
                uri
            },
        }
    }

    pub fn to_string(&self) -> String {
        format!("UPath({})", self.to_uri())
    }

    pub async fn read_bytes(&self, _client: Client) -> io::Result<Vec<u8>> { unimplemented!() }
    pub async fn write_bytes(&self, _client: Client, _input: Vec<u8>) -> io::Result<Vec<u8>> { unimplemented!() }

    pub async fn parent(&self) -> Option<UPath> {
        // TODO: Implement parent method
        unimplemented!()
    }

    pub async fn hash(&self, _algorithm: &str) -> Multihash<128> {
        // TODO: Implement hash method
        unimplemented!()
    }

    pub async fn is_folder(&self) -> bool {
        // TODO: Implement is_folder method
        unimplemented!()
    }


}

#[cfg(test)]
mod tests {
    use tracing::info;
    use super::*;

    #[test]
    fn test_new_local() {
        let local_uri = utils::local_uri_parquet();
        let upath = UPath::parse(&local_uri).unwrap();
        info!("upath: {:?}", upath);
        assert_eq!(upath.to_uri().to_string(), local_uri);
        let upath_string = upath.to_string();
        assert_eq!(upath_string, format!("UPath({})", local_uri));
        let UPath::Local(fpath) = upath else { panic!() };
        assert_eq!(local_uri, format!("file://{}", fpath.to_string_lossy()));
    }

    #[test]
    fn test_new_s3() {
        let upath = UPath::parse(utils::TEST_S3_URI).unwrap();
        assert_eq!(upath.to_uri().to_string(), utils::TEST_S3_URI);
        assert_eq!(upath, UPath::S3 { bucket: "quilt-example".into(), path: "akarve/test_dest/README.md".into() });
    }

    #[test]
    fn test_new_invalid() {
        UPath::parse("blah://123").expect_err("did not get an error");
    }
}
