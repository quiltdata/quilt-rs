//! How a catalog host name typed or stored by a person becomes a [`url::Host`].
//!
//! [`Host::from_str`](super::Host) is the one door every catalog host comes
//! through: the login form in `QuiltSync`, `quilt login <host>`, the `catalog=` of a
//! `quilt+s3://` URI, and a stored account. It calls only [`normalize`], so the
//! rule is in one place.
//!
//! The rule is quiltx's `normalize_dns` (the `quiltx-core` crate), so quilt-rs and
//! quiltx name a catalog the same way and key its credentials alike:
//!
//! - The text is trimmed and lowercased, and kept as is otherwise: a Unicode name
//!   is **not** converted to punycode.
//! - `https://host/...` is reduced to `host`, and so is `host/path`. `http://` is
//!   refused.
//! - A port, an IP address and an empty name are refused.
//!
//! One check is added here: the name becomes a directory name under `.auth/`, so a
//! name `url::Host` would refuse (a space, `@`, `\`, `%` and so on) or that is only
//! dots (`..`) is refused too.
//!
//! The tests below pin the rule, and say where it differs from the one it replaced,
//! `url::Host::parse`.

use crate::UriError;

/// The host `input` names, or why it is not one.
pub(super) fn normalize(input: &str) -> Result<url::Host, UriError> {
    let name = quiltx_core::dns::normalize_dns(input, false).map_err(|e| UriError::Host(e.0))?;
    if name.chars().all(|c| c == '.') {
        return Err(UriError::Host(format!("{name:?} is not a host name")));
    }
    // Not the value: `url` would turn a Unicode name into punycode. This is only
    // to refuse a name that is not a host.
    url::Host::parse(&name).map_err(|e| UriError::Host(format!("{name:?}: {e}")))?;
    Ok(url::Host::Domain(name))
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

    fn rejected(input: &str) -> String {
        let err = normalize(input).expect_err(&format!("{input:?} should be refused"));
        assert!(matches!(err, UriError::Host(_)), "{input:?}: {err:?}");
        err.to_string()
    }

    // Unchanged by quiltx's rule: the cases that were pinned before it.

    #[test]
    fn a_plain_name_is_kept() {
        assert_eq!(normalize("quilt.test").unwrap(), domain("quilt.test"));
        assert_eq!(shown("nightly.quilttest.com"), "nightly.quilttest.com");
        assert_eq!(shown("localhost"), "localhost");
        assert_eq!(shown("a"), "a");
        assert_eq!(shown("quilt_test.example"), "quilt_test.example");
    }

    #[test]
    fn case_is_folded_to_lower() {
        assert_eq!(shown("Nightly.QuiltTest.COM"), "nightly.quilttest.com");
        assert_eq!(shown("QUILT.TEST"), "quilt.test");
    }

    #[test]
    fn a_trailing_dot_is_kept() {
        // The fully-qualified form is a different directory name, not a spelling
        // of the same one.
        assert_eq!(shown("quilt.test."), "quilt.test.");
        assert_eq!(shown("localhost."), "localhost.");
    }

    #[test]
    fn a_port_is_refused() {
        for input in [
            "quilt.test:443",
            "quilt.test:8443",
            "localhost:3000",
            "127.0.0.1:8080",
            "https://Quilt.TEST:443/",
        ] {
            assert_eq!(
                rejected(input),
                "Invalid host: Custom ports are not supported. Use the bare DNS name."
            );
        }
    }

    #[test]
    fn plain_http_and_other_schemes_are_refused() {
        let message =
            "Invalid host: http:// is not supported. Use https:// or pass the DNS name directly.";
        assert_eq!(rejected("http://quilt.test"), message);
        assert_eq!(rejected("ftp://quilt.test"), message);
    }

    #[test]
    fn empty_is_refused() {
        assert_eq!(rejected(""), "Invalid host: catalog DNS name is required");
        assert_eq!(rejected("  "), "Invalid host: catalog DNS name is required");
    }

    #[test]
    fn a_name_url_would_refuse_is_refused() {
        // Would otherwise become a directory name under `.auth/`.
        rejected("quilt .test");
        rejected("user@quilt.test");
        rejected("quilt.test?x=1");
        rejected("quilt.test#frag");
        rejected("quilt.test\\x");
        rejected("100%.test");
        rejected("a%2Fb.test");
    }

    #[test]
    fn a_name_of_only_dots_is_refused() {
        // `.auth/..` would be the auth folder's parent.
        for input in [".", "..", "..."] {
            assert!(rejected(input).ends_with("is not a host name"), "{input}");
        }
    }

    #[test]
    fn the_error_is_an_invalid_host() {
        assert!(rejected("a b").starts_with("Invalid host: "));
    }

    // Behavior that differs from `url::Host::parse`, the rule this replaced. Each
    // test says what it was.

    #[test]
    fn a_unicode_name_is_kept_not_punycoded() {
        // Was: bücher.example -> xn--bcher-kva.example.
        assert_eq!(shown("bücher.example"), "bücher.example");
        assert_eq!(shown("BÜCHER.example"), "bücher.example");
        // Punycode typed by hand stays punycode, so a catalog stored under it is
        // still found.
        assert_eq!(shown("xn--bcher-kva.example"), "xn--bcher-kva.example");
    }

    #[test]
    fn an_ip_address_is_refused() {
        // Was: accepted as an IPv4 or IPv6 host.
        let message = "Invalid host: IP-address identifiers are not supported. Use the DNS name.";
        assert_eq!(rejected("127.0.0.1"), message);
        assert_eq!(rejected("10.1.2.3"), message);
        rejected("[::1]");
        rejected("[2001:DB8::1]");
        rejected("[::1");
    }

    #[test]
    fn a_scheme_is_dropped_down_to_the_host() {
        // Was: refused.
        assert_eq!(shown("https://quilt.test"), "quilt.test");
        assert_eq!(shown("https://quilt.test/"), "quilt.test");
        assert_eq!(shown("HTTPS://Quilt.TEST/some/path?q=1#f"), "quilt.test");
        assert_eq!(shown("https://user@quilt.test"), "quilt.test");
    }

    #[test]
    fn a_path_after_the_host_is_dropped() {
        // Was: refused.
        assert_eq!(shown("quilt.test/"), "quilt.test");
        assert_eq!(shown("quilt.test/some/path"), "quilt.test");
        assert_eq!(shown("a/../b"), "a");
    }

    #[test]
    fn surrounding_whitespace_is_trimmed() {
        // Was: refused.
        assert_eq!(shown(" quilt.test"), "quilt.test");
        assert_eq!(shown("quilt.test "), "quilt.test");
        assert_eq!(shown("quilt.test\n"), "quilt.test");
    }

    #[test]
    fn an_empty_port_is_ignored() {
        // Was: refused.
        assert_eq!(shown("quilt.test:"), "quilt.test");
    }
}
