//! Captures the warning issued when a padded `path=` is accepted and trimmed.

use std::path::PathBuf;

use quilt_uri::S3PackageUri;

/// The URL parser already drops a trailing space, so the warning must name
/// the path once rather than printing the same string twice.
#[test]
fn a_trailing_space_warns_that_padding_was_removed() {
    testing_logger::setup();
    let uri = S3PackageUri::try_from("quilt+s3://bucket#package=a/b&path=a.csv ")
        .expect("quilt3 accepts a trailing space on path");
    assert_eq!(uri.path, Some(PathBuf::from("a.csv")));
    testing_logger::validate(|logs| {
        let bodies: Vec<&str> = logs.iter().map(|log| log.body.as_str()).collect();
        assert!(
            bodies.contains(&"removed trailing whitespace from path=a.csv"),
            "expected a single-path warning, got {bodies:?}"
        );
    });
}
