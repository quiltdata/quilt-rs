use std::str::Utf8Error;

use thiserror::Error;
use url::ParseError;
use url::Url;

#[derive(Error, Debug, PartialEq)]
pub enum UriError {
    #[error("Invalid file:// URI: {0}")]
    FileScheme(Url),

    #[error("Invalid bucket: {0}")]
    Bucket(String),

    #[error("Invalid host: {0}")]
    Host(String),

    #[error("Invalid URI scheme: {0}")]
    Scheme(String),

    #[error("Invalid namespace: {0}")]
    Namespace(String),

    #[error("Invalid package URI: {0}")]
    Package(String),

    #[error("Invalid S3 URI: {0}")]
    S3(String),

    #[error("Manifest path error: {0}")]
    ManifestPath(String),

    #[error("Invalid tag: {0}")]
    Tag(String),

    #[error("Error parsing URL: {0}")]
    UrlParse(#[from] ParseError),

    #[error("UTF-8 error: {0}")]
    Utf8(#[from] Utf8Error),
}

/// `input` with any user and password before its host masked, for an error
/// to repeat: `quilt+s3://KEY:secret@b#package=a/b` becomes
/// `quilt+s3://***@b#package=a/b`.
///
/// Tabs and line breaks are dropped first, as the URL parser drops them.
/// Without a `://`, everything up to the last `@` before any `?` or `#` is
/// masked, so an address the parser refuses, such as `s3:KEY:secret@b/k`,
/// is masked too.
#[must_use]
pub fn mask_user(input: &str) -> String {
    let input: String = input
        .chars()
        .filter(|c| !matches!(c, '\t' | '\n' | '\r'))
        .collect();
    let (start, ends) = match input.find("://") {
        Some(i) => (i + 3, &['/', '?', '#'][..]),
        None => (0, &['?', '#'][..]),
    };
    let rest = &input[start..];
    let host_end = rest.find(ends).unwrap_or(rest.len());
    match rest[..host_end].rfind('@') {
        Some(at) => format!("{}***{}", &input[..start], &rest[at..]),
        None => input,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mask_user_masks_only_what_comes_before_the_host() {
        for (input, masked) in [
            (
                "quilt+s3://KEY:secret@b#package=a/b",
                "quilt+s3://***@b#package=a/b",
            ),
            (
                "s3://KEY:secret@b/k?versionId=1",
                "s3://***@b/k?versionId=1",
            ),
            ("s3://KEY:secret@b@c/k", "s3://***@c/k"),
            ("s3:KEY:secret@b/k", "***@b/k"),
            ("KEY:secret@b", "***@b"),
            (
                "quilt+local:\t//KEY:secret@b#package=a/b",
                "quilt+local://***@b#package=a/b",
            ),
            ("s3://KEY:se\ncret@b/k", "s3://***@b/k"),
            ("quilt+local:/KEY:secret@b#package=a/b", "***@b#package=a/b"),
            ("a/KEY:secret@b", "***@b"),
            ("s3://b/k@v", "s3://b/k@v"),
            (
                "quilt+s3://b#package=a/b@abcdef",
                "quilt+s3://b#package=a/b@abcdef",
            ),
            ("team/raw", "team/raw"),
            ("", ""),
        ] {
            assert_eq!(mask_user(input), masked, "{input}");
        }
    }
}
