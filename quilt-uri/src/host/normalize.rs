//! How a catalog host name typed or stored by a person becomes a [`url::Host`].
//!
//! [`Host::from_str`](super::Host) is the one door every catalog host comes
//! through: the login form in `QuiltSync`, `quilt login <host>`, the `catalog=` of a
//! `quilt+s3://` URI, and a stored account. It calls only [`normalize`], so the
//! rule is in one place and can change without touching a caller.
//!
//! Today the rule is `url::Host::parse`: ASCII letters are lowercased, a Unicode
//! name becomes its IDNA (punycode) form, and an IPv4 or bracketed IPv6 address
//! is a host. A scheme, a path, a port, a space or an empty string is an error.
//! The tests below pin that, so a later change to the rule shows as an edit to
//! them.

use crate::UriError;

/// The host `input` names, or why it is not one.
pub(super) fn normalize(input: &str) -> Result<url::Host, UriError> {
    url::Host::parse(input).map_err(|e| UriError::Host(e.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn domain(name: &str) -> url::Host {
        url::Host::Domain(name.to_string())
    }

    /// The host `input` becomes, as the string a directory name or URI shows.
    fn shown(input: &str) -> String {
        normalize(input)
            .unwrap_or_else(|e| panic!("{input:?} should parse: {e}"))
            .to_string()
    }

    fn rejected(input: &str) {
        let err = normalize(input).expect_err(&format!("{input:?} should be refused"));
        assert!(matches!(err, UriError::Host(_)), "{input:?}: {err:?}");
    }

    #[test]
    fn a_plain_name_is_kept() {
        assert_eq!(normalize("quilt.test").unwrap(), domain("quilt.test"));
        assert_eq!(shown("nightly.quilttest.com"), "nightly.quilttest.com");
        assert_eq!(shown("localhost"), "localhost");
        assert_eq!(shown("a"), "a");
    }

    #[test]
    fn ascii_case_is_folded_to_lower() {
        assert_eq!(shown("Nightly.QuiltTest.COM"), "nightly.quilttest.com");
        assert_eq!(shown("QUILT.TEST"), "quilt.test");
    }

    #[test]
    fn a_trailing_dot_is_kept() {
        // The fully-qualified form is a different directory name, not a spelling
        // of the same one.
        assert_eq!(shown("quilt.test."), "quilt.test.");
    }

    #[test]
    fn a_unicode_name_becomes_punycode() {
        assert_eq!(shown("bücher.example"), "xn--bcher-kva.example");
        assert_eq!(shown("BÜCHER.example"), "xn--bcher-kva.example");
        // Already punycode: unchanged.
        assert_eq!(shown("xn--bcher-kva.example"), "xn--bcher-kva.example");
    }

    #[test]
    fn an_ip_address_is_a_host() {
        assert_eq!(
            normalize("127.0.0.1").unwrap(),
            url::Host::<String>::Ipv4("127.0.0.1".parse().unwrap())
        );
        assert_eq!(shown("10.1.2.3"), "10.1.2.3");
        assert_eq!(shown("[::1]"), "[::1]");
        assert_eq!(shown("[2001:DB8::1]"), "[2001:db8::1]");
    }

    #[test]
    fn a_port_is_refused() {
        rejected("quilt.test:443");
        rejected("quilt.test:8443");
        rejected("localhost:3000");
        rejected("127.0.0.1:8080");
        // A trailing colon too.
        rejected("quilt.test:");
    }

    #[test]
    fn a_url_or_path_is_refused() {
        rejected("https://quilt.test");
        rejected("https://quilt.test/");
        rejected("http://quilt.test");
        rejected("quilt.test/");
        rejected("quilt.test/some/path");
        rejected("quilt.test?x=1");
        rejected("quilt.test#frag");
        rejected("user@quilt.test");
    }

    #[test]
    fn whitespace_and_empty_are_refused() {
        rejected("");
        rejected(" ");
        rejected(" quilt.test");
        rejected("quilt.test ");
        rejected("quilt .test");
        rejected("quilt.test\n");
    }

    #[test]
    fn a_malformed_literal_is_refused() {
        rejected("[::1");
        rejected("[not-an-address]");
        rejected("quilt..test:");
    }

    #[test]
    fn the_error_names_the_host_rule() {
        let err = normalize("a b").unwrap_err();
        assert!(err.to_string().starts_with("Invalid host: "), "{err}");
    }
}
