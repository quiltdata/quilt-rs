use std::cmp::Ord;
use std::cmp::Ordering;
use std::cmp::PartialOrd;
use std::collections::HashMap;
use std::fmt;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use serde::Serializer;
use serde::de;
use serde::de::Visitor;
use url::Url;
use url::form_urlencoded;

use crate::Host;
use crate::ManifestUri;
use crate::Tag;
use crate::UriError;
use crate::s3::check_bucket;

/// This is the revision (or "hash") of the package.
/// "Package" itself is a handle, but each package has revision.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "_tag", content = "value", try_from = "RawRevisionPointer")]
pub enum RevisionPointer {
    Hash(String),
    Tag(Tag),
}

/// The serialized form of [`RevisionPointer`], read before its hash is checked.
#[derive(Deserialize)]
#[serde(tag = "_tag", content = "value")]
enum RawRevisionPointer {
    Hash(String),
    Tag(Tag),
}

impl TryFrom<RawRevisionPointer> for RevisionPointer {
    type Error = UriError;

    fn try_from(raw: RawRevisionPointer) -> Result<Self, Self::Error> {
        match raw {
            RawRevisionPointer::Hash(hash) => {
                check_stored_hash(&hash)?;
                Ok(Self::Hash(hash))
            }
            RawRevisionPointer::Tag(tag) => Ok(Self::Tag(tag)),
        }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl Default for RevisionPointer {
    fn default() -> Self {
        Self::Tag(Tag::Latest)
    }
}

/// In theory namespace is just a string.
/// But in practice we use "prefix/name".
/// For ease of serializing/deserializing and for validation we put it to a struct.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
#[cfg_attr(any(test, feature = "test-support"), derive(Default))]
pub struct Namespace {
    prefix: String,
    name: String,
}

impl Namespace {
    /// The half before the `/`.
    ///
    /// Readers group by this — a prefix spans the packages one owner or project
    /// published — and without it they re-derive it by splitting [`Display`]'s
    /// output, which is the one split this type exists to have already done.
    ///
    /// [`Display`]: std::fmt::Display
    #[must_use]
    pub fn prefix(&self) -> &str {
        &self.prefix
    }

    /// The half after the `/`.
    #[must_use]
    pub fn name(&self) -> &str {
        &self.name
    }
}

impl Ord for Namespace {
    fn cmp(&self, other: &Self) -> Ordering {
        match self.prefix.cmp(&other.prefix) {
            Ordering::Equal => self.name.cmp(&other.name),
            Ordering::Less => Ordering::Less,
            Ordering::Greater => Ordering::Greater,
        }
    }
}

impl PartialOrd for Namespace {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl fmt::Display for Namespace {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}/{}", self.prefix, self.name)
    }
}

#[cfg(any(test, feature = "test-support"))]
impl From<(String, String)> for Namespace {
    fn from((prefix, name): (String, String)) -> Self {
        Namespace { prefix, name }
    }
}

#[cfg(any(test, feature = "test-support"))]
impl From<(&str, &str)> for Namespace {
    fn from((prefix, name): (&str, &str)) -> Self {
        (prefix.to_string(), name.to_string()).into()
    }
}

/// Each half is a folder name on disk, so it must stay a single folder on
/// every platform: not `.` or `..`, and no separator, drive colon or control
/// character.
fn is_plain_half(half: &str) -> bool {
    half != "."
        && half != ".."
        && !half
            .chars()
            .any(|c| matches!(c, '/' | '\\' | ':') || c.is_control())
}

impl TryFrom<&str> for Namespace {
    type Error = UriError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        let (prefix, name) = input.split_once('/').ok_or_else(|| {
            UriError::Namespace(format!("namespace must be `<prefix>/<name>`; got: {input}"))
        })?;
        if prefix.is_empty() || name.is_empty() {
            return Err(UriError::Namespace(format!(
                "namespace prefix and name must be non-empty; got: {input}"
            )));
        }
        if name.contains('/') {
            return Err(UriError::Namespace(format!(
                "namespace must contain exactly one `/`; got: {input}"
            )));
        }
        if !is_plain_half(prefix) || !is_plain_half(name) {
            return Err(UriError::Namespace(format!(
                "namespace prefix and name must not be `.` or `..` or hold `\\`, `:` or control characters; got: {input:?}"
            )));
        }
        Ok(Namespace {
            prefix: prefix.to_string(),
            name: name.to_string(),
        })
    }
}

impl TryFrom<String> for Namespace {
    type Error = UriError;

    fn try_from(input: String) -> Result<Self, Self::Error> {
        input.as_str().try_into()
    }
}

impl Serialize for Namespace {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

struct NamespaceVisitor;

impl Visitor<'_> for NamespaceVisitor {
    type Value = Namespace;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a string prefix and a string name divided with /")
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        Namespace::try_from(value).map_err(|e| E::custom(format!("Failed parse namespace {e}")))
    }
}

impl<'de> Deserialize<'de> for Namespace {
    fn deserialize<D>(deserializer: D) -> Result<Namespace, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_string(NamespaceVisitor)
    }
}

/// This is kinda URI for the package without revisions.
/// You can use it when you don't know or don't care about revision of the package.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct S3PackageHandle {
    pub bucket: String,
    pub namespace: Namespace,
}

impl From<S3PackageUri> for S3PackageHandle {
    fn from(uri: S3PackageUri) -> S3PackageHandle {
        S3PackageHandle {
            bucket: uri.bucket,
            namespace: uri.namespace,
        }
    }
}

impl From<&S3PackageUri> for S3PackageHandle {
    fn from(uri: &S3PackageUri) -> S3PackageHandle {
        S3PackageHandle {
            bucket: uri.bucket.clone(),
            namespace: uri.namespace.clone(),
        }
    }
}

impl From<ManifestUri> for S3PackageHandle {
    fn from(uri: ManifestUri) -> S3PackageHandle {
        S3PackageHandle {
            bucket: uri.bucket,
            namespace: uri.namespace,
        }
    }
}

impl From<&ManifestUri> for S3PackageHandle {
    fn from(uri: &ManifestUri) -> S3PackageHandle {
        S3PackageHandle {
            bucket: uri.bucket.clone(),
            namespace: uri.namespace.clone(),
        }
    }
}

/// Struct representation of the general `quilt+s3://url`
/// Package handle + revision is a package.
/// Also, this URI has path, so you can use it as an URI for referencing files in package.
/// You can use this URL for both packages and files in packages.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "RawS3PackageUri")]
pub struct S3PackageUri {
    pub catalog: Option<Host>,
    pub bucket: String,
    pub namespace: Namespace,
    pub revision: RevisionPointer,
    pub path: Option<PathBuf>,
}

/// The serialized form of [`S3PackageUri`], read before its bucket is checked.
#[derive(Deserialize)]
struct RawS3PackageUri {
    catalog: Option<Host>,
    bucket: String,
    namespace: Namespace,
    revision: RevisionPointer,
    path: Option<PathBuf>,
}

impl TryFrom<RawS3PackageUri> for S3PackageUri {
    type Error = UriError;

    fn try_from(raw: RawS3PackageUri) -> Result<Self, Self::Error> {
        check_bucket(&raw.bucket)?;
        Ok(Self {
            catalog: raw.catalog,
            bucket: raw.bucket,
            namespace: raw.namespace,
            revision: raw.revision,
            path: raw.path,
        })
    }
}

impl TryFrom<&str> for S3PackageUri {
    type Error = UriError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        let parsed_url = Url::parse(input)?;
        if parsed_url.scheme() != "quilt+s3" {
            return Err(UriError::Package(format!(
                "expected quilt+s3, got {}",
                parsed_url.scheme()
            )));
        }

        let fragment = parsed_url.fragment().ok_or(UriError::Package(format!(
            "S3 package URI must contain a fragment: {input}"
        )))?;
        let mut params: HashMap<_, _> = form_urlencoded::parse(fragment.as_bytes())
            .into_owned()
            .collect();

        let pkg_spec = params
            .remove("package")
            .ok_or(UriError::Package("missing package in fragment".to_string()))?;

        let (namespace, revision) = if pkg_spec.contains(':') && pkg_spec.contains('@') {
            return Err(UriError::Package(
                "package spec may either contain \":\" or \"@\"".to_string(),
            ));
        } else if let Some((namespace, tag)) = pkg_spec.split_once(':') {
            if tag.is_empty() {
                return Err(UriError::Package("tag must not be empty".to_string()));
            }
            if tag.contains(':') {
                return Err(UriError::Package(
                    "package spec may contain only one \":\"".to_string(),
                ));
            }
            (namespace.into(), RevisionPointer::Tag(tag.parse()?))
        } else if let Some((namespace, top_hash)) = pkg_spec.split_once('@') {
            if top_hash.is_empty() {
                return Err(UriError::Package("hash must not be empty".to_string()));
            }
            if top_hash.contains('@') {
                return Err(UriError::Package(
                    "package spec may contain only one \"@\"".to_string(),
                ));
            }
            check_top_hash(top_hash)?;
            (namespace.into(), RevisionPointer::Hash(top_hash.into()))
        } else {
            (pkg_spec, RevisionPointer::Tag(Tag::Latest))
        };

        let path = params.remove("path").map(PathBuf::from);

        let catalog = match params.remove("catalog") {
            Some(c) => Some(c.parse()?),
            None => None,
        };

        if !params.is_empty() {
            return Err(UriError::Package(format!(
                "unexpected parameters in fragment: {params:?}"
            )));
        }

        let bucket = parsed_url.host_str().ok_or(UriError::Package(format!(
            "expected host in S3 package URI, got {}",
            parsed_url.host_str().unwrap_or_default()
        )))?;
        check_bucket(bucket)?;

        Ok(Self {
            bucket: bucket.to_string(),
            catalog,
            namespace: namespace.try_into()?,
            path,
            revision,
        })
    }
}

/// A top hash is the hex SHA-256 of a manifest, as `TopHasher` writes it.
#[must_use]
pub fn is_top_hash(hash: &str) -> bool {
    hash.len() == 64 && hash.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// A stored hash is a top hash, or empty for a remote that is set but not
/// pushed yet.
pub(crate) fn check_stored_hash(hash: &str) -> Result<(), UriError> {
    if hash.is_empty() {
        Ok(())
    } else {
        check_top_hash(hash)
    }
}

fn check_top_hash(hash: &str) -> Result<(), UriError> {
    if is_top_hash(hash) {
        Ok(())
    } else {
        Err(UriError::Package(format!(
            "hash must be 64 lowercase hex characters, got {hash:?}"
        )))
    }
}

/// The characters that would otherwise be read as fragment structure.
///
/// `&`, `=` and `#` delimit; `+` decodes as a space and `%` opens an escape.
/// Everything else is left alone — `/` above all, because a namespace and a
/// logical key are full of them and these addresses are read and pasted by
/// hand.
const FRAGMENT_VALUE: &percent_encoding::AsciiSet = &percent_encoding::CONTROLS
    .add(b'&')
    .add(b'=')
    .add(b'#')
    .add(b'+')
    .add(b'%');

/// [`FRAGMENT_VALUE`] and space, which would end a link pasted into a chat.
const COPY_VALUE: &percent_encoding::AsciiSet = &FRAGMENT_VALUE.add(b' ');

impl S3PackageUri {
    fn format_hash(hash: &str) -> String {
        let chars: Vec<char> = hash.chars().collect();
        if chars.len() <= 12 {
            hash.to_string()
        } else {
            let head: String = chars[..6].iter().collect();
            let tail: String = chars[chars.len() - 6..].iter().collect();
            format!("{head}...{tail}")
        }
    }

    #[must_use]
    pub fn display(&self) -> String {
        self.render(false, FRAGMENT_VALUE)
    }

    /// The address in full for copying: the whole hash, and every fragment
    /// value escaped, spaces too, so a pasted address stays one link.
    #[must_use]
    pub fn to_copy_string(&self) -> String {
        self.render(true, COPY_VALUE)
    }

    /// Each fragment value is escaped with `set`, safe to put beside the
    /// delimiters.
    ///
    /// The parse side reads the fragment with `form_urlencoded`, so anything written
    /// raw comes back changed or truncated: a logical key holding `&` ends the value
    /// early and the rest is read as another parameter.
    fn render(&self, full_hash: bool, set: &'static percent_encoding::AsciiSet) -> String {
        let encode = |value: &str| percent_encoding::utf8_percent_encode(value, set).to_string();
        let hash = match &self.revision {
            RevisionPointer::Tag(Tag::Latest) => String::new(),
            RevisionPointer::Tag(tag) => format!(":{}", encode(&tag.to_string())),
            RevisionPointer::Hash(h) if full_hash => format!("@{}", encode(h)),
            RevisionPointer::Hash(h) => format!("@{}", encode(&Self::format_hash(h))),
        };
        let path_part = match &self.path {
            Some(p) => format!("&path={}", encode(&p.display().to_string())),
            None => String::new(),
        };
        let catalog_part = match &self.catalog {
            Some(p) => format!("&catalog={}", encode(&p.to_string())),
            None => String::new(),
        };
        format!(
            "quilt+s3://{}#package={}{}{}{}",
            self.bucket,
            encode(&self.namespace.to_string()),
            hash,
            path_part,
            catalog_part
        )
    }

    /// Renders this package URI as a catalog web URL on `host`.
    ///
    /// # Errors
    ///
    /// Returns [`UriError::UrlParse`] if `host`, the bucket, or the namespace
    /// yield a string that is not a valid URL.
    pub fn display_for_host(&self, host: &Host) -> Result<url::Url, UriError> {
        let version = match &self.revision {
            RevisionPointer::Tag(tag) => tag.to_string(),
            RevisionPointer::Hash(hash) => hash.clone(),
        };
        let mut url = url::Url::parse(&format!(
            "https://{}/b/{}/packages/{}/tree/{}",
            host, self.bucket, self.namespace, version
        ))?;

        if let Some(path) = &self.path {
            let mut new_path = url.path().to_string();
            new_path.push('/');
            new_path.push_str(&path.display().to_string());
            url.set_path(&new_path);
        }
        Ok(url)
    }

    /// Renders this package URI as a web URL on its own catalog host.
    ///
    /// # Errors
    ///
    /// Returns [`UriError::Package`] if the URI has no catalog host, or any
    /// error from [`display_for_host`](Self::display_for_host).
    pub fn display_for_catalog(&self) -> Result<url::Url, UriError> {
        let host = self.catalog.as_ref().ok_or(UriError::Package(
            "Package URI has no catalog specified".to_string(),
        ))?;
        self.display_for_host(host)
    }
}

impl fmt::Display for S3PackageUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let hash = match &self.revision {
            RevisionPointer::Tag(Tag::Latest) => String::new(),
            RevisionPointer::Tag(tag) => format!(":{tag}"),
            RevisionPointer::Hash(h) => format!("@{h}"),
        };
        let path_part = match &self.path {
            Some(p) => format!("&path={}", p.display()),
            None => String::new(),
        };
        let catalog_part = match &self.catalog {
            Some(p) => format!("&catalog={p}"),
            None => String::new(),
        };
        write!(
            f,
            "quilt+s3://{}#package={}{}{}{}",
            self.bucket, self.namespace, hash, path_part, catalog_part
        )
    }
}

impl std::str::FromStr for S3PackageUri {
    type Err = UriError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        S3PackageUri::try_from(input)
    }
}

impl From<ManifestUri> for S3PackageUri {
    fn from(uri: ManifestUri) -> S3PackageUri {
        S3PackageUri {
            bucket: uri.bucket,
            catalog: uri.origin,
            namespace: uri.namespace,
            path: None,
            revision: RevisionPointer::Hash(uri.hash),
        }
    }
}

impl From<&ManifestUri> for S3PackageUri {
    fn from(uri: &ManifestUri) -> S3PackageUri {
        S3PackageUri {
            bucket: uri.bucket.clone(),
            catalog: uri.origin.clone(),
            namespace: uri.namespace.clone(),
            path: None,
            revision: RevisionPointer::Hash(uri.hash.clone()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Seconds;
    use crate::fixtures;

    type Res<T = ()> = Result<T, UriError>;

    /// The halves a reader wants, without re-splitting `Display`'s output.
    #[test]
    fn a_namespace_lends_its_halves() -> Res {
        let ns: Namespace = "team/plate-07".try_into()?;
        assert_eq!(ns.prefix(), "team");
        assert_eq!(ns.name(), "plate-07");
        // And the whole is still what it always was, so nothing reading the
        // display form has to change.
        assert_eq!(ns.to_string(), "team/plate-07");
        Ok(())
    }

    /// A name may hold a `-`, a `.`, a `_`; only the first `/` divides, and
    /// `try_from` has already rejected a second one. So `name()` never has to be
    /// re-split by a caller either.
    #[test]
    fn only_the_first_slash_divides() -> Res {
        let ns: Namespace = "my.team_1/plate-07.v2".try_into()?;
        assert_eq!(ns.prefix(), "my.team_1");
        assert_eq!(ns.name(), "plate-07.v2");
        assert!(
            Namespace::try_from("a/b/c").is_err(),
            "a second / is refused"
        );
        Ok(())
    }

    /// `Hash`, so a caller can key a map by the package a row is about rather
    /// than by a string spelling of it.
    #[test]
    fn a_namespace_can_key_a_map() -> Res {
        use std::collections::HashMap;

        let mut seen: HashMap<Namespace, u8> = HashMap::new();
        seen.insert("team/plate-07".try_into()?, 1);
        assert_eq!(seen.get(&Namespace::try_from("team/plate-07")?), Some(&1));
        assert_eq!(seen.get(&Namespace::try_from("team/plate-08")?), None);
        Ok(())
    }

    #[test]
    fn test_implicit_str_parsing() -> Res {
        let uri: S3PackageUri =
            format!("quilt+s3://bucket#package=foo/bar@{}", fixtures::TOP_HASH).parse()?;
        assert_eq!(
            uri,
            S3PackageUri {
                bucket: "bucket".to_string(),
                catalog: None,
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Hash(fixtures::TOP_HASH.to_string()),
                path: None,
            }
        );
        Ok(())
    }

    #[test]
    fn test_implicit_string_parsing() -> Res {
        let uri: S3PackageUri = format!("quilt+s3://bucket#package=foo/bar@{}", fixtures::TOP_HASH)
            .to_string()
            .parse()?;
        assert_eq!(
            uri,
            S3PackageUri {
                bucket: "bucket".to_string(),
                catalog: None,
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Hash(fixtures::TOP_HASH.to_string()),
                path: None,
            }
        );
        Ok(())
    }

    #[test]
    fn test_incorrect_scheme() {
        let uri = S3PackageUri::try_from("s3://bucket#packagefoo/bar");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid package URI: expected quilt+s3, got s3".to_string(),
        );
    }

    #[test]
    fn test_no_fragment() {
        let uri = S3PackageUri::try_from("quilt+s3://bucket");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid package URI: S3 package URI must contain a fragment: quilt+s3://bucket"
                .to_string(),
        );
    }

    #[test]
    fn test_no_package() {
        let uri = S3PackageUri::try_from("quilt+s3://bucket#foo=bar");
        assert_eq!(
            uri.unwrap_err().to_string(),
            "Invalid package URI: missing package in fragment".to_string(),
        );
    }

    #[test]
    fn test_unknown_paramter() {
        let uri = S3PackageUri::try_from("quilt+s3://bucket#package=a/b&foo=bar");
        assert_eq!(
            uri.unwrap_err().to_string(),
            r#"Invalid package URI: unexpected parameters in fragment: {"foo": "bar"}"#.to_string(),
        );
    }

    #[test]
    fn test_no_bucket() {
        let uri = S3PackageUri::try_from("quilt+s3://#package=a/b");
        assert_eq!(
            uri.unwrap_err().to_string(),
            r"Invalid package URI: expected host in S3 package URI, got ".to_string(),
        );
    }

    #[test]
    fn test_path() -> Res {
        let uri: S3PackageUri = format!(
            "quilt+s3://bucket#package=foo/bar@{}&path=read/me.md",
            fixtures::TOP_HASH
        )
        .parse()?;
        assert_eq!(
            uri,
            S3PackageUri {
                bucket: "bucket".to_string(),
                catalog: None,
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Hash(fixtures::TOP_HASH.to_string()),
                path: Some(PathBuf::from("read/me.md")),
            }
        );
        Ok(())
    }

    #[test]
    fn test_latest() -> Res {
        let uri: S3PackageUri = "quilt+s3://bucket#package=foo/bar&path=read/me.md".parse()?;
        assert_eq!(
            uri,
            S3PackageUri {
                bucket: "bucket".to_string(),
                catalog: None,
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Tag(Tag::Latest),
                path: Some(PathBuf::from("read/me.md")),
            }
        );
        Ok(())
    }

    #[test]
    fn test_catalog() -> Res {
        let uri: S3PackageUri =
            "quilt+s3://bucket#package=foo/bar&path=read/me.md&catalog=quilt.test".parse()?;
        assert_eq!(
            uri,
            S3PackageUri {
                bucket: "bucket".to_string(),
                catalog: Some(fixtures::host()),
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Tag(Tag::Latest),
                path: Some(PathBuf::from("read/me.md")),
            }
        );
        Ok(())
    }

    #[test]
    fn test_stringify_with_latest() {
        let uri = S3PackageUri {
            bucket: "bucket".to_string(),
            catalog: Some(fixtures::host()),
            namespace: ("foo", "bar").into(),
            revision: RevisionPointer::Tag(Tag::Latest),
            path: Some(PathBuf::from("read/me.md")),
        };
        assert_eq!(
            uri.to_string(),
            "quilt+s3://bucket#package=foo/bar&path=read/me.md&catalog=quilt.test"
        );
    }

    #[test]
    fn test_stringify_with_hash() {
        // Test with short hash
        let uri = S3PackageUri {
            bucket: "bucket".to_string(),
            catalog: None,
            namespace: ("foo", "bar").into(),
            revision: RevisionPointer::Hash("abc123".to_string()),
            path: None,
        };
        assert_eq!(uri.to_string(), "quilt+s3://bucket#package=foo/bar@abc123");

        // Test with long hash
        let uri = S3PackageUri {
            bucket: "bucket".to_string(),
            catalog: None,
            namespace: ("foo", "bar").into(),
            revision: RevisionPointer::Hash("abcdef1234567890xyz".to_string()),
            path: None,
        };
        assert_eq!(
            uri.to_string(),
            "quilt+s3://bucket#package=foo/bar@abcdef1234567890xyz"
        );
    }

    #[test]
    fn test_stringify_with_timestamp_tag() {
        let uri = S3PackageUri {
            bucket: "bucket".to_string(),
            catalog: None,
            namespace: ("foo", "bar").into(),
            revision: RevisionPointer::Tag(Tag::Timestamp(Seconds(1_697_916_638))),
            path: None,
        };
        assert_eq!(
            uri.to_string(),
            "quilt+s3://bucket#package=foo/bar:1697916638"
        );
    }

    #[test]
    fn test_from_manifest_uri() {
        let manifest_uri = ManifestUri {
            bucket: "test-bucket".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "abc123".to_string(),
            origin: None,
        };

        // Test From<ManifestUri>
        assert_eq!(
            S3PackageUri::from(manifest_uri.clone()),
            S3PackageUri {
                bucket: "test-bucket".to_string(),
                catalog: None,
                namespace: ("foo", "bar").into(),
                path: None,
                revision: RevisionPointer::Hash("abc123".to_string()),
            }
        );

        // Test From<&ManifestUri>
        assert_eq!(
            S3PackageUri::from(&manifest_uri),
            S3PackageUri {
                bucket: "test-bucket".to_string(),
                catalog: None,
                namespace: ("foo", "bar").into(),
                path: None,
                revision: RevisionPointer::Hash("abc123".to_string()),
            }
        );
    }

    #[test]
    fn test_namespace_ordering_greater() {
        let ns1 = Namespace::from(("z", "a"));
        let ns2 = Namespace::from(("a", "b"));

        assert!(ns1 > ns2);
        assert_eq!(ns1.cmp(&ns2), Ordering::Greater);

        let ns3 = Namespace::from(("same", "z"));
        let ns4 = Namespace::from(("same", "a"));

        assert!(ns3 > ns4);
        assert_eq!(ns3.cmp(&ns4), Ordering::Greater);
    }

    #[test]
    fn test_namespace_partial_ordering() {
        let ns1 = Namespace::from(("a", "b"));
        let ns2 = Namespace::from(("a", "b"));
        let ns3 = Namespace::from(("c", "d"));

        // Test equality
        assert!(ns1 >= ns2);
        assert!(ns1 <= ns2);
        assert!(ns1 <= ns2);
        assert!(ns1 >= ns2);

        // Test less than
        assert!(ns1 < ns3);
        assert!(ns1 <= ns3);
        assert!(ns1 <= ns3);
        assert!(ns1 < ns3);

        // Test greater than
        assert!(ns3 > ns1);
        assert!(ns3 >= ns1);
        assert!(ns3 >= ns1);
        assert!(ns3 > ns1);
    }

    #[test]
    fn test_display_for_host() -> Res {
        let host = fixtures::host();

        let uri_latest: S3PackageUri =
            "quilt+s3://bucket#package=foo/bar&path=read/me.md".parse()?;
        assert_eq!(
            uri_latest.display_for_host(&host)?.as_str(),
            "https://quilt.test/b/bucket/packages/foo/bar/tree/latest/read/me.md"
        );

        let uri_versioned: S3PackageUri = format!(
            "quilt+s3://bucket#package=foo/bar@{}&path=read/me.md",
            fixtures::TOP_HASH
        )
        .parse()?;
        assert_eq!(
            uri_versioned.display_for_host(&host)?.as_str(),
            format!(
                "https://quilt.test/b/bucket/packages/foo/bar/tree/{}/read/me.md",
                fixtures::TOP_HASH
            )
        );
        Ok(())
    }

    #[test]
    fn test_display_for_catalog() -> Res {
        let uri_with_catalog: S3PackageUri =
            "quilt+s3://bucket#package=foo/bar&path=read/me.md&catalog=quilt.test".parse()?;
        assert_eq!(
            uri_with_catalog.display_for_catalog()?.as_str(),
            "https://quilt.test/b/bucket/packages/foo/bar/tree/latest/read/me.md"
        );

        let uri_without_catalog: S3PackageUri =
            "quilt+s3://bucket#package=foo/bar&path=read/me.md".parse()?;
        assert!(uri_without_catalog.display_for_catalog().is_err());
        Ok(())
    }

    #[test]
    fn test_tag_parsing_timestamp() -> Res {
        let uri: S3PackageUri = "quilt+s3://bucket#package=foo/bar:1697916638".parse()?;
        assert_eq!(
            uri,
            S3PackageUri {
                bucket: "bucket".to_string(),
                catalog: None,
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Tag(Tag::Timestamp(Seconds(1_697_916_638))),
                path: None,
            }
        );
        Ok(())
    }

    #[test]
    fn test_tag_with_path() -> Res {
        let uri: S3PackageUri = "quilt+s3://bucket#package=foo/bar:latest&path=data.csv".parse()?;
        assert_eq!(
            uri,
            S3PackageUri {
                bucket: "bucket".to_string(),
                catalog: None,
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Tag(Tag::Latest),
                path: Some(PathBuf::from("data.csv")),
            }
        );
        Ok(())
    }

    #[test]
    fn test_tag_parsing_unsupported() {
        let result = S3PackageUri::try_from("quilt+s3://bucket#package=foo/bar:my-experiment");
        let err = result.unwrap_err();
        assert!(matches!(err, UriError::Tag(_)), "expected Tag error: {err}");
    }

    #[test]
    fn test_revision_pointer_serde_round_trip_latest() {
        let revision = RevisionPointer::Tag(Tag::Latest);
        let json = serde_json::to_string(&revision).unwrap();
        assert_eq!(json, r#"{"_tag":"Tag","value":"latest"}"#);
        let parsed: RevisionPointer = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, revision);
    }

    #[test]
    fn test_revision_pointer_serde_round_trip_timestamp() {
        let revision = RevisionPointer::Tag(Tag::Timestamp(Seconds(1_697_916_638)));
        let json = serde_json::to_string(&revision).unwrap();
        assert_eq!(json, r#"{"_tag":"Tag","value":"1697916638"}"#);
        let parsed: RevisionPointer = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, revision);
    }

    #[test]
    fn test_revision_pointer_serde_round_trip_hash() {
        let revision = RevisionPointer::Hash(fixtures::TOP_HASH.to_string());
        let json = serde_json::to_string(&revision).unwrap();
        assert_eq!(
            json,
            format!(r#"{{"_tag":"Hash","value":"{}"}}"#, fixtures::TOP_HASH)
        );
        let parsed: RevisionPointer = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, revision);
    }

    #[test]
    fn test_empty_tag_error() {
        let result = S3PackageUri::try_from("quilt+s3://bucket#package=foo/bar:");
        assert_eq!(
            result.unwrap_err().to_string(),
            "Invalid package URI: tag must not be empty"
        );
    }

    #[test]
    fn test_multiple_colons_error() {
        let result = S3PackageUri::try_from("quilt+s3://bucket#package=foo/bar:latest:extra");
        assert_eq!(
            result.unwrap_err().to_string(),
            "Invalid package URI: package spec may contain only one \":\""
        );
    }

    #[test]
    fn test_empty_hash_error() {
        let result = S3PackageUri::try_from("quilt+s3://bucket#package=foo/bar@");
        assert_eq!(
            result.unwrap_err().to_string(),
            "Invalid package URI: hash must not be empty"
        );
    }

    #[test]
    fn test_multiple_at_signs_error() {
        let result = S3PackageUri::try_from("quilt+s3://bucket#package=foo/bar@abc123@def456");
        assert_eq!(
            result.unwrap_err().to_string(),
            "Invalid package URI: package spec may contain only one \"@\""
        );
    }

    /// A top hash is a SHA-256 digest in lowercase hex, so an address carries
    /// it whole.
    #[test]
    fn a_full_hex_hash_parses() -> Res {
        let hash = "0123456789abcdef".repeat(4);
        let uri: S3PackageUri = format!("quilt+s3://bucket#package=foo/bar@{hash}").parse()?;
        assert_eq!(uri.revision, RevisionPointer::Hash(hash));
        Ok(())
    }

    /// Anything other than the full lowercase digest is not a top hash.
    #[test]
    fn a_hash_that_is_not_a_full_hex_digest_is_refused() {
        let digest = "0123456789abcdef".repeat(4);
        let cases = [
            "zzzz".to_string(),
            "deadbeef".to_string(),
            digest.to_uppercase(),
            digest[..63].to_string(),
            format!("{digest}0"),
            format!("{}/{}", &digest[..32], &digest[33..]),
            format!("{}.{}", &digest[..32], &digest[33..]),
            format!("{}é", &digest[..62]),
        ];
        for hash in cases {
            let result = S3PackageUri::try_from(
                format!("quilt+s3://bucket#package=foo/bar@{hash}").as_str(),
            );
            assert!(
                matches!(result, Err(UriError::Package(_))),
                "{hash:?} should be refused, got {result:?}"
            );
        }
    }

    /// Display shortens by characters, so a hash built by hand that holds
    /// multibyte characters or is short still renders.
    #[test]
    fn format_hash_counts_characters() {
        assert_eq!(
            S3PackageUri::format_hash("aééééééééééééé"),
            "aééééé...éééééé"
        );
        assert_eq!(S3PackageUri::format_hash("ab"), "ab");
        assert_eq!(S3PackageUri::format_hash("éa"), "éa");
    }

    #[test]
    fn test_both_tag_and_hash_error() {
        let result = S3PackageUri::try_from("quilt+s3://bucket#package=foo/bar:latest@abc123");
        assert_eq!(
            result.unwrap_err().to_string(),
            "Invalid package URI: package spec may either contain \":\" or \"@\""
        );
    }

    #[test]
    fn test_namespace_valid() -> Res {
        assert_eq!(
            Namespace::try_from("foo/bar")?,
            Namespace::from(("foo", "bar"))
        );
        Ok(())
    }

    #[test]
    fn test_namespace_missing_slash() {
        let err = Namespace::try_from("foo").unwrap_err();
        assert_eq!(
            err.to_string(),
            "Invalid namespace: namespace must be `<prefix>/<name>`; got: foo"
        );
    }

    #[test]
    fn test_namespace_only_slash() {
        let err = Namespace::try_from("/").unwrap_err();
        assert_eq!(
            err.to_string(),
            "Invalid namespace: namespace prefix and name must be non-empty; got: /"
        );
    }

    #[test]
    fn test_namespace_empty_prefix() {
        let err = Namespace::try_from("/bar").unwrap_err();
        assert_eq!(
            err.to_string(),
            "Invalid namespace: namespace prefix and name must be non-empty; got: /bar"
        );
    }

    #[test]
    fn test_namespace_empty_name() {
        let err = Namespace::try_from("foo/").unwrap_err();
        assert_eq!(
            err.to_string(),
            "Invalid namespace: namespace prefix and name must be non-empty; got: foo/"
        );
    }

    #[test]
    fn test_namespace_extra_slash() {
        let err = Namespace::try_from("foo/bar/baz").unwrap_err();
        assert_eq!(
            err.to_string(),
            "Invalid namespace: namespace must contain exactly one `/`; got: foo/bar/baz"
        );
    }

    /// Each half becomes a folder name on disk, so the URI parser refuses any
    /// half that would not stay one folder.
    #[test]
    fn a_uri_refuses_a_namespace_half_that_is_not_a_plain_folder_name() {
        for package in [
            "../x",
            "x/..",
            "./x",
            "%2e%2e/%2e%2e",
            "a/..%5C..%5C..%5Cx",
            "a%5Cb/c",
            "a/b%00",
            "a%0Ab/c",
            "a/b%09c",
            "a/b%7F",
        ] {
            let input = format!("quilt+s3://bucket#package={package}");
            let err = S3PackageUri::try_from(input.as_str())
                .expect_err(&format!("{package} should not parse"));
            assert!(
                matches!(err, UriError::Namespace(_)),
                "{package}: expected a namespace error, got {err}"
            );
        }
    }

    /// Deserialize reaches the same check, so a stored namespace gets it too.
    #[test]
    fn a_deserialized_namespace_gets_the_same_check() {
        for namespace in ["C:/x", "a/C:", "../x", "a/\\x", "a/b\u{0}", "a\nb/c"] {
            let json = serde_json::to_string(namespace).unwrap();
            assert!(
                serde_json::from_str::<Namespace>(&json).is_err(),
                "{namespace:?} should not deserialize"
            );
        }
    }

    /// Dots, dashes and underscores inside a half still parse.
    #[test]
    fn a_dotted_namespace_still_parses() -> Res {
        let uri = S3PackageUri::try_from("quilt+s3://bucket#package=my.team_1/plate-07.v2")?;
        assert_eq!(uri.namespace.prefix(), "my.team_1");
        assert_eq!(uri.namespace.name(), "plate-07.v2");
        let ns: Namespace = serde_json::from_str("\"..a/b..\"").unwrap();
        assert_eq!(ns.to_string(), "..a/b..");
        Ok(())
    }

    #[test]
    fn test_display_method_short_and_long_hash() {
        // Short hash (<= 12 chars) is rendered verbatim.
        let short = S3PackageUri {
            bucket: "bucket".to_string(),
            catalog: None,
            namespace: ("foo", "bar").into(),
            revision: RevisionPointer::Hash("abc123".to_string()),
            path: None,
        };
        assert_eq!(short.display(), "quilt+s3://bucket#package=foo/bar@abc123");

        // Long hash is abbreviated to `first6...last6`.
        let long = S3PackageUri {
            bucket: "bucket".to_string(),
            catalog: None,
            namespace: ("foo", "bar").into(),
            revision: RevisionPointer::Hash("abcdef1234567890".to_string()),
            path: None,
        };
        assert_eq!(
            long.display(),
            "quilt+s3://bucket#package=foo/bar@abcdef...567890"
        );
    }

    #[test]
    fn test_display_method_latest_timestamp_and_catalog() {
        let latest = S3PackageUri {
            bucket: "bucket".to_string(),
            catalog: None,
            namespace: ("foo", "bar").into(),
            revision: RevisionPointer::Tag(Tag::Latest),
            path: Some(PathBuf::from("read/me.md")),
        };
        assert_eq!(
            latest.display(),
            "quilt+s3://bucket#package=foo/bar&path=read/me.md"
        );

        let timestamped = S3PackageUri {
            bucket: "bucket".to_string(),
            catalog: Some(fixtures::host()),
            namespace: ("foo", "bar").into(),
            revision: RevisionPointer::Tag(Tag::Timestamp(Seconds(1_697_916_638))),
            path: None,
        };
        assert_eq!(
            timestamped.display(),
            "quilt+s3://bucket#package=foo/bar:1697916638&catalog=quilt.test"
        );
    }

    #[test]
    fn test_s3_package_handle_from_package_uri() {
        let uri = S3PackageUri {
            bucket: "bucket".to_string(),
            catalog: None,
            namespace: ("foo", "bar").into(),
            revision: RevisionPointer::Hash("abc123".to_string()),
            path: None,
        };
        let expected = S3PackageHandle {
            bucket: "bucket".to_string(),
            namespace: ("foo", "bar").into(),
        };
        assert_eq!(S3PackageHandle::from(&uri), expected);
        assert_eq!(S3PackageHandle::from(uri), expected);
    }

    #[test]
    fn test_s3_package_handle_from_manifest_uri() {
        let manifest = ManifestUri {
            origin: None,
            bucket: "bucket".to_string(),
            namespace: ("foo", "bar").into(),
            hash: "abc123".to_string(),
        };
        let expected = S3PackageHandle {
            bucket: "bucket".to_string(),
            namespace: ("foo", "bar").into(),
        };
        assert_eq!(S3PackageHandle::from(&manifest), expected);
        assert_eq!(S3PackageHandle::from(manifest), expected);
    }

    #[test]
    fn test_namespace_serde_round_trip() {
        let namespace = Namespace::from(("foo", "bar"));
        let json = serde_json::to_string(&namespace).unwrap();
        assert_eq!(json, "\"foo/bar\"");
        let parsed: Namespace = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, namespace);
    }

    #[test]
    fn test_namespace_deserialize_owned_string() {
        // A `/` escape forces serde to hand us an owned `String`; the
        // `Visitor` default `visit_string` delegates it to `visit_str`.
        let parsed: Namespace = serde_json::from_str("\"foo\\u002fbar\"").unwrap();
        assert_eq!(parsed, Namespace::from(("foo", "bar")));
    }

    #[test]
    fn test_namespace_deserialize_invalid() {
        let result: Result<Namespace, _> = serde_json::from_str("\"noslash\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_revision_pointer_default() {
        assert_eq!(
            RevisionPointer::default(),
            RevisionPointer::Tag(Tag::Latest)
        );
    }

    #[test]
    fn test_namespace_deserialize_owned_string_invalid() {
        // Escaped, but no `/` after decoding: the owned `String` reaches
        // `visit_str` (via the default `visit_string`) and errors there.
        let result: Result<Namespace, _> = serde_json::from_str("\"no\\u0061slash\"");
        assert!(result.is_err());
    }

    #[test]
    fn test_namespace_deserialize_wrong_type() {
        // A non-string JSON value drives the visitor's `expecting` message.
        let result: Result<Namespace, _> = serde_json::from_str("123");
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("prefix and a string name")
        );
    }

    #[test]
    fn test_s3_package_uri_serde_round_trip() {
        let uri = S3PackageUri {
            bucket: "bucket".to_string(),
            catalog: Some(fixtures::host()),
            namespace: ("foo", "bar").into(),
            revision: RevisionPointer::Hash(fixtures::TOP_HASH.to_string()),
            path: Some(PathBuf::from("read/me.md")),
        };
        let json = serde_json::to_string(&uri).unwrap();
        let parsed: S3PackageUri = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed, uri);
    }

    /// A logical key is user data and may hold anything a filesystem allows.
    /// The fragment is read with `form_urlencoded`, so every character that
    /// means something there has to survive the trip out and back.
    #[test]
    fn a_path_holding_fragment_delimiters_round_trips() {
        for path in [
            "a&catalog=x.csv",
            "plus+one.csv",
            "a=b.csv",
            "hash#tag.csv",
            "already%2Fencoded.csv",
            "with space.csv",
            "runs/2026/plate-07/A01.fcs",
        ] {
            let uri = S3PackageUri {
                catalog: None,
                bucket: "b".to_string(),
                namespace: ("user", "p").into(),
                revision: RevisionPointer::Tag(Tag::Latest),
                path: Some(PathBuf::from(path)),
            };

            let back = S3PackageUri::try_from(uri.display().as_str())
                .unwrap_or_else(|e| panic!("{path:?} -> {} : {e}", uri.display()));

            assert_eq!(back, uri, "{path:?} came back as {}", uri.display());
        }
    }

    /// The copied form comes back whole whatever the values hold, pinned or at
    /// latest: every delimiter and space escaped, and the hash never shortened.
    #[test]
    fn the_copy_string_round_trips() {
        let hash = "0123456789abcdef".repeat(4);
        for revision in [
            RevisionPointer::Tag(Tag::Latest),
            RevisionPointer::Hash(hash.clone()),
        ] {
            for value in [
                "a+b",
                "a&catalog=x",
                "100%",
                "hash#tag",
                "a=b",
                "with space",
            ] {
                let uri = S3PackageUri {
                    catalog: Some("quilt.test".parse().unwrap()),
                    bucket: "b".to_string(),
                    namespace: ("user", value).into(),
                    revision: revision.clone(),
                    path: Some(PathBuf::from(format!("dir/{value}.csv"))),
                };
                let copied = uri.to_copy_string();

                assert!(!copied.contains(' '), "{copied}");
                let back = S3PackageUri::try_from(copied.as_str())
                    .unwrap_or_else(|e| panic!("{value:?} -> {copied} : {e}"));
                assert_eq!(back, uri, "{value:?} came back from {copied}");
            }
        }
    }

    /// Slashes stay literal in the copied form too, and the hash is whole.
    #[test]
    fn the_copy_string_keeps_slashes_and_the_whole_hash() {
        let hash = "0123456789abcdef".repeat(4);
        let uri = S3PackageUri {
            catalog: Some("quilt.test".parse().unwrap()),
            bucket: "team-bucket".to_string(),
            namespace: ("user", "plate-07").into(),
            revision: RevisionPointer::Hash(hash.clone()),
            path: Some(PathBuf::from("runs/a/one.csv")),
        };

        assert_eq!(
            uri.to_copy_string(),
            format!(
                "quilt+s3://team-bucket#package=user/plate-07@{hash}&path=runs/a/one.csv&catalog=quilt.test"
            )
        );
    }

    /// The readable part stays readable: a plain address carries its slashes
    /// literally, because these are pasted into a terminal and a chat window.
    #[test]
    fn an_ordinary_address_is_not_escaped() {
        let uri = S3PackageUri {
            catalog: None,
            bucket: "team-bucket".to_string(),
            namespace: ("user", "plate-07").into(),
            revision: RevisionPointer::Tag(Tag::Latest),
            path: Some(PathBuf::from("runs/a/one.csv")),
        };

        assert_eq!(
            uri.display(),
            "quilt+s3://team-bucket#package=user/plate-07&path=runs/a/one.csv"
        );
    }

    /// Deserializing a hash revision applies the parser's top hash rule.
    #[test]
    fn a_deserialized_hash_revision_must_be_a_top_hash() {
        for hash in [
            "../../../../tmp/x",
            "abc123",
            "0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF0123456789ABCDEF",
        ] {
            let json = format!(
                r#"{{"_tag":"Hash","value":{}}}"#,
                serde_json::to_string(hash).unwrap()
            );
            assert!(
                serde_json::from_str::<RevisionPointer>(&json).is_err(),
                "{hash:?} should not deserialize"
            );
        }
    }

    /// A remote set but not pushed yet has an empty hash, and its package URI
    /// still reads.
    #[test]
    fn a_package_uri_not_pushed_yet_reads() {
        let uri = S3PackageUri::from(ManifestUri {
            origin: None,
            bucket: "team-bucket".to_string(),
            namespace: ("user", "plate-07").into(),
            hash: String::new(),
        });
        let json = serde_json::to_string(&uri).unwrap();
        assert_eq!(serde_json::from_str::<S3PackageUri>(&json).unwrap(), uri);
    }

    /// The parser refuses a bucket name S3 never allowed.
    #[test]
    fn a_uri_refuses_a_bucket_name_s3_does_not_allow() {
        for bucket in ["..", ".", "Bad_Bucket!", "a%20b", "a%2Fb"] {
            let input = format!("quilt+s3://{bucket}#package=foo/bar");
            let err = S3PackageUri::try_from(input.as_str())
                .expect_err(&format!("{bucket} should not parse"));
            assert!(
                matches!(err, UriError::Bucket(_)),
                "{bucket}: expected a bucket error, got {err}"
            );
        }
    }

    /// Deserializing a package URI applies the same bucket rule.
    #[test]
    fn a_deserialized_package_uri_gets_the_bucket_check() {
        for bucket in ["../../x", "..", ".", "", "a b", "a/b", "a#b", "a@b", "a:b"] {
            let uri = S3PackageUri {
                catalog: None,
                bucket: bucket.to_string(),
                namespace: ("foo", "bar").into(),
                revision: RevisionPointer::Tag(Tag::Latest),
                path: None,
            };
            let json = serde_json::to_string(&uri).unwrap();
            assert!(
                serde_json::from_str::<S3PackageUri>(&json).is_err(),
                "{bucket:?} should not deserialize"
            );
        }
    }

    /// Names from before S3's 2018 rules, with uppercase and `_`, still read.
    #[test]
    fn a_legacy_bucket_name_still_reads() -> Res {
        let uri = S3PackageUri::try_from("quilt+s3://My_Old.Bucket-1#package=foo/bar")?;
        assert_eq!(uri.bucket, "My_Old.Bucket-1");
        let json = serde_json::to_string(&uri).unwrap();
        assert_eq!(serde_json::from_str::<S3PackageUri>(&json).unwrap(), uri);
        Ok(())
    }

    /// The JSON form is what lineage files and the app hold, so it stays as is.
    #[test]
    fn a_package_uri_keeps_its_json_form() {
        let json = format!(
            r#"{{"catalog":"quilt.test","bucket":"team-bucket","namespace":"user/plate-07","revision":{{"_tag":"Hash","value":"{}"}},"path":"runs/one.csv"}}"#,
            fixtures::TOP_HASH
        );
        let uri = S3PackageUri {
            catalog: Some(fixtures::host()),
            bucket: "team-bucket".to_string(),
            namespace: ("user", "plate-07").into(),
            revision: RevisionPointer::Hash(fixtures::TOP_HASH.to_string()),
            path: Some(PathBuf::from("runs/one.csv")),
        };
        assert_eq!(serde_json::to_string(&uri).unwrap(), json);
        assert_eq!(serde_json::from_str::<S3PackageUri>(&json).unwrap(), uri);

        let latest = r#"{"catalog":null,"bucket":"team-bucket","namespace":"user/plate-07","revision":{"_tag":"Tag","value":"latest"},"path":null}"#;
        let uri = S3PackageUri {
            catalog: None,
            path: None,
            revision: RevisionPointer::Tag(Tag::Latest),
            ..uri
        };
        assert_eq!(serde_json::to_string(&uri).unwrap(), latest);
        assert_eq!(serde_json::from_str::<S3PackageUri>(latest).unwrap(), uri);
    }
}
