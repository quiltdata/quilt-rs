use std::fmt;
use std::path::PathBuf;

use url::Url;

use crate::uri::S3Uri;
use crate::Error;

#[derive(Clone, Debug, PartialEq)]
pub struct SharePointUri {
    value: Url,
}

impl fmt::Display for SharePointUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.value)
    }
}

impl TryFrom<&str> for SharePointUri {
    type Error = Error;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        let url = Url::try_from(input)?;
        if url.scheme() != "sharepoint" {
            return Err(Error::Place(format!("Invalid SharePoint URI: {}", input)));
        }
        Ok(SharePointUri { value: url })
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum PlaceValue {
    Header,
    PathBuf(PathBuf),
    S3Uri(S3Uri),
    SharePoint(SharePointUri),
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
        Place::new(PlaceValue::Header)
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

impl TryFrom<Place> for PathBuf {
    type Error = Error;

    fn try_from(place: Place) -> Result<Self, Self::Error> {
        match place.value {
            PlaceValue::PathBuf(path) => Ok(path),
            _ => Err(Error::Place("Place is not file://".to_string())),
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

impl TryFrom<Place> for S3Uri {
    type Error = Error;

    fn try_from(place: Place) -> Result<Self, Self::Error> {
        match place.value {
            PlaceValue::S3Uri(s3_uri) => Ok(s3_uri),
            _ => Err(Error::Place("Place is not s3://".to_string())),
        }
    }
}

impl From<SharePointUri> for Place {
    fn from(uri: SharePointUri) -> Place {
        Place::new(PlaceValue::SharePoint(uri))
    }
}

impl TryFrom<Place> for SharePointUri {
    type Error = Error;

    fn try_from(place: Place) -> Result<Self, Self::Error> {
        match place.value {
            PlaceValue::SharePoint(uri) => Ok(uri),
            _ => Err(Error::Place("Place is not sharepoint://".to_string())),
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
        if input.starts_with("sharepoint://") {
            return SharePointUri::try_from(input)
                .map(Place::from)
                .map_err(|e| Error::Place(e.to_string()));
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
        let path1 = PathBuf::from("/tmp/foo/bar");
        assert_eq!(place1, Place::from(path1.clone()));
        assert_eq!(PathBuf::try_from(place1)?, path1);

        let place2 = Place::try_from("file:///foo")?;
        let path2 = PathBuf::from("/foo");
        assert_eq!(place2, Place::from(path2.clone()));
        assert_eq!(PathBuf::try_from(place2)?, path2);

        let place3 = Place::try_from("file://invalid");
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
        let uri1 = S3Uri {
            bucket: "bucket".to_string(),
            key: "foo/bar".to_string(),
            version: Some("abc".to_string()),
        };
        assert_eq!(place1, Place::from(uri1.clone()));
        assert_eq!(S3Uri::try_from(place1)?, uri1);

        let place2 = Place::try_from("s3://invalid");
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

    #[test]
    fn test_formatting_sharepoint() -> Res {
        let place = Place::from(SharePointUri::try_from("sharepoint://foo/bar")?);
        assert_eq!(place.to_string(), "sharepoint://foo/bar");
        Ok(())
    }

    #[test]
    fn test_parsing_sharepoint() -> Res {
        let place1 = Place::try_from("sharepoint://tmp/foo/bar")?;
        let uri1 = SharePointUri::try_from("sharepoint://tmp/foo/bar")?;
        assert_eq!(place1, Place::from(uri1.clone()));
        assert_eq!(SharePointUri::try_from(place1)?, uri1);

        let place2 = Place::try_from("sharepoint://foo")?;
        let uri2 = SharePointUri::try_from("sharepoint://foo")?;
        assert_eq!(place2, Place::from(uri2.clone()));
        assert_eq!(SharePointUri::try_from(place2)?, uri2);

        let place3 = Place::try_from("file://invalid");
        match place3 {
            Err(err) => assert_eq!(
                err.to_string(),
                "Invalid Place: Unsupported file://invalid".to_string()
            ),
            Ok(_) => panic!("shouldn't happen"),
        }
        let uri3 = SharePointUri::try_from("file://foo");
        match uri3 {
            Err(err) => assert_eq!(
                err.to_string(),
                "Invalid Place: Invalid SharePoint URI: file://foo".to_string()
            ),
            Ok(_) => panic!("shouldn't happen"),
        }
        Ok(())
    }

    #[test]
    fn test_invalid_assignments() -> Res {
        let place1 = Place::from(PathBuf::from("/tmp/foo"));
        let s3_uri1 = S3Uri::try_from(place1.clone());
        match s3_uri1 {
            Err(err) => assert_eq!(
                err.to_string(),
                "Invalid Place: Place is not s3://".to_string()
            ),
            Ok(_) => panic!("shouldn't happen"),
        }
        let sharepoint_uri1 = SharePointUri::try_from(place1);
        match sharepoint_uri1 {
            Err(err) => assert_eq!(
                err.to_string(),
                "Invalid Place: Place is not sharepoint://".to_string()
            ),
            Ok(_) => panic!("shouldn't happen"),
        }

        let place2 = Place::from(S3Uri::try_from("s3://bucket/foo/bar?versionId=abc")?);
        let path_buf2 = PathBuf::try_from(place2.clone());
        match path_buf2 {
            Err(err) => assert_eq!(
                err.to_string(),
                "Invalid Place: Place is not file://".to_string()
            ),
            Ok(_) => panic!("shouldn't happen"),
        }
        let sharepoint_uri2 = SharePointUri::try_from(place2);
        match sharepoint_uri2 {
            Err(err) => assert_eq!(
                err.to_string(),
                "Invalid Place: Place is not sharepoint://".to_string()
            ),
            Ok(_) => panic!("shouldn't happen"),
        }

        let place3 = Place::from(SharePointUri::try_from("sharepoint://foo/bar")?);
        let s3_uri3 = S3Uri::try_from(place3.clone());
        match s3_uri3 {
            Err(err) => assert_eq!(
                err.to_string(),
                "Invalid Place: Place is not s3://".to_string()
            ),
            Ok(_) => panic!("shouldn't happen"),
        }
        let path_buf3 = PathBuf::try_from(place3.clone());
        match path_buf3 {
            Err(err) => assert_eq!(
                err.to_string(),
                "Invalid Place: Place is not file://".to_string()
            ),
            Ok(_) => panic!("shouldn't happen"),
        }
        Ok(())
    }

    #[test]
    fn test_invalid_parsing() -> Res {
        let place1 = Place::try_from("file://:");
        match place1 {
            Err(err) => assert_eq!(
                err.to_string(),
                Error::Place(url::ParseError::InvalidDomainCharacter.to_string()).to_string()
            ),
            Ok(_) => panic!("shouldn't happen"),
        }
        let place2 = Place::try_from("sharepoint://:");
        match place2 {
            Err(err) => assert_eq!(
                err.to_string(),
                Error::Place(Error::UrlParse(url::ParseError::EmptyHost).to_string()).to_string(),
            ),
            Ok(_) => panic!("shouldn't happen"),
        }
        Ok(())
    }
}
