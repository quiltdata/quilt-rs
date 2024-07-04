use std::fmt;
use std::path::PathBuf;

use url::Url;

use crate::uri::S3Uri;
use crate::Error;

#[derive(Clone, Debug, PartialEq)]
pub enum PlaceValue {
    Header,
    PathBuf(PathBuf),
    S3Uri(S3Uri),
    SharePoint(Url), // TODO: SharePointUri
}

#[derive(Clone, Debug, PartialEq)]
pub struct Place {
    pub value: PlaceValue,
}

impl Place {
    pub fn new(value: PlaceValue) -> Self {
        Place { value }
    }

    pub fn header() -> Self {
        Place {
            value: PlaceValue::Header,
        }
    }
}

impl Default for Place {
    fn default() -> Self {
        Place {
            value: PlaceValue::PathBuf(PathBuf::default()),
        }
    }
}

impl From<PathBuf> for Place {
    fn from(path: PathBuf) -> Place {
        Place::new(PlaceValue::PathBuf(path))
    }
}

impl From<Place> for PathBuf {
    fn from(place: Place) -> Self {
        match place.value {
            PlaceValue::PathBuf(path) => path,
            _ => panic!("Place is not a file://"),
        }
    }
}

impl From<S3Uri> for Place {
    fn from(s3_uri: S3Uri) -> Place {
        Place {
            value: PlaceValue::S3Uri(s3_uri),
        }
    }
}

impl From<Place> for S3Uri {
    fn from(place: Place) -> Self {
        match place.value {
            PlaceValue::S3Uri(s3_uri) => s3_uri,
            _ => panic!("Place is not an S3 URI"),
        }
    }
}

impl TryFrom<&str> for Place {
    type Error = Error;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        if input.starts_with("s3://") {
            return S3Uri::try_from(input)
                .map(Place::from)
                .map_err(|e| Error::Place(e.to_string()));
        }
        if input.starts_with("file://") {
            return match Url::try_from(input) {
                Ok(url) => {
                    if let Some(domain) = url.domain() {
                        if domain != "localhost" {
                            let msg = format!("Unsupported file://{}", domain);
                            return Err(Self::Error::Place(msg));
                        }
                    }
                    return Ok(Place::from(PathBuf::from(url.path().to_string())));
                }
                Err(e) => Err(Error::Place(e.to_string())),
            };
        }
        Err(Self::Error::Place(input.to_string()))
    }
}

impl TryFrom<String> for Place {
    type Error = Error;

    fn try_from(input: String) -> Result<Self, Self::Error> {
        input.as_str().try_into()
    }
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match &self.value {
            PlaceValue::Header => write!(f, "."),
            PlaceValue::PathBuf(path) => write!(f, "file://{}", path.display()),
            PlaceValue::S3Uri(uri) => write!(f, "{}", uri),
            PlaceValue::SharePoint(url) => write!(f, "{}", url),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::Res;

    #[test]
    fn test_formatting_path_buf() {
        let place = Place::from(PathBuf::from("/tmp/foo"));
        assert_eq!(place.to_string(), "file:///tmp/foo");
    }

    #[test]
    fn test_parsing_path_buf() -> Res {
        let place1 = Place::try_from("file:///tmp/foo/bar")?;
        let place2 = Place::try_from("file:///foo")?;
        let place3 = Place::try_from("file://invalid");
        assert_eq!(place1, Place::from(PathBuf::from("/tmp/foo/bar")));
        assert_eq!(place2, Place::from(PathBuf::from("/foo")));
        match place3 {
            Err(err) => assert_eq!(
                err.to_string(),
                "Invalid Place: Unsupported file://invalid".to_string()
            ),
            Ok(_) => panic!("shouldn't happen"),
        }
        Ok(())
    }

    #[test]
    fn test_formatting_s3_uri() -> Res {
        let place = Place::from(S3Uri::try_from("s3://bucket/foo/bar?versionId=abc")?);
        assert_eq!(place.to_string(), "s3://bucket/foo/bar?versionId=abc");
        Ok(())
    }

    #[test]
    fn test_parsing_s3_uri() -> Res {
        let place1 = Place::try_from("s3://bucket/foo/bar?versionId=abc")?;
        let place2 = Place::try_from("s3://invalid");
        assert_eq!(
            place1,
            Place::from(S3Uri {
                bucket: "bucket".to_string(),
                key: "foo/bar".to_string(),
                version: Some("abc".to_string())
            })
        );
        match place2 {
            Err(err) => assert_eq!(
                err.to_string(),
                "Invalid Place: Invalid S3 URI: missing key".to_string()
            ),
            Ok(_) => panic!("shouldn't happen"),
        }
        Ok(())
    }

    #[test]
    fn test_formatting_header() {
        let place = Place::header();
        assert_eq!(place.to_string(), ".");
    }

    #[test]
    fn test_parsing_header() {
        let place = Place::try_from(".");
        match place {
            Err(err) => assert_eq!(err.to_string(), "Invalid Place: .".to_string()),
            Ok(_) => panic!("shouldn't happen"),
        }
    }
}
