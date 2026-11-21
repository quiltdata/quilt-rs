//! Captures the warning issued when a padded `path=` is accepted and trimmed.

use std::cell::RefCell;
use std::path::PathBuf;

use quilt_uri::S3PackageUri;

fn warnings_and_path(input: &str) -> (Vec<String>, Option<PathBuf>) {
    testing_logger::setup();
    let uri = S3PackageUri::try_from(input).unwrap_or_else(|e| panic!("{input:?}: {e}"));
    let bodies = RefCell::new(Vec::new());
    testing_logger::validate(|logs| {
        bodies
            .borrow_mut()
            .extend(logs.iter().map(|log| log.body.clone()));
    });
    (bodies.into_inner(), uri.path)
}

fn assert_warned_as(input: &str, path: Option<&str>, warning: &str) {
    let (bodies, got) = warnings_and_path(input);
    assert_eq!(got, path.map(PathBuf::from), "{input:?}");
    assert!(
        bodies.iter().any(|body| body == warning),
        "{input:?}: expected {warning:?}, got {bodies:?}"
    );
}

/// The URL parser already drops a trailing space, so the warning must name
/// the path once rather than printing the same string twice.
#[test]
fn a_trailing_space_warns_that_padding_was_removed() {
    assert_warned_as(
        "quilt+s3://bucket#package=a/b&path=a.csv ",
        Some("a.csv"),
        "removed leading or trailing whitespace from path=a.csv",
    );
}

/// A `#` in the path value, a percent-encoded `path` key, and whitespace the
/// URL parser strips from the middle of the URI must still be warned about.
#[test]
fn padding_is_warned_about_in_accepted_forms_the_url_parser_strips() {
    let warning = "removed leading or trailing whitespace from path=a.csv";
    assert_warned_as(
        "quilt+s3://bucket#package=a/b&path=hash#tag.csv ",
        Some("hash#tag.csv"),
        "removed leading or trailing whitespace from path=hash#tag.csv",
    );
    assert_warned_as(
        "quilt+s3://bucket#package=a/b&pa%74h=a.csv ",
        Some("a.csv"),
        warning,
    );
    assert_warned_as(
        "quilt+s3://bucket#package=a/b&%70ath=a.csv ",
        Some("a.csv"),
        warning,
    );
    assert_warned_as(
        "quilt+s3://bucket#package=a/b&path=\ta.csv",
        Some("a.csv"),
        warning,
    );
    assert_warned_as(
        "quilt+s3://bucket#package=a/b&path=a.csv\t&catalog=quilt.test",
        Some("a.csv"),
        warning,
    );
}
