//! How a catalog host name becomes a [`url::Host`]: two doors, one rule.
//!
//! - [`typed`] is for a name a person types or pastes: the login form in
//!   `QuiltSync`, `quilt login --host`. It is quiltx's `normalize_dns` (the
//!   `quiltx-core` crate), so quilt-rs and quiltx name a catalog the same way and
//!   key its credentials alike. The text is trimmed and lowercased and otherwise
//!   kept: a Unicode name is **not** converted to punycode. `https://host/...` and
//!   `host/path` are reduced to `host`. A port, an IP address (also in its
//!   shorthand forms, `127.1` or `2130706433`), a percent escape, a scheme other
//!   than `https` and an empty name are refused.
//! - [`stored`] is for a name read back from disk or from a URI: a stored account
//!   under `.auth/`, the `catalog=` of a `quilt+s3://` URI, the `origin` in
//!   `.quilt/data.json`. It takes everything [`typed`] takes and, failing that,
//!   what `url::Host::parse` took before, such as an IP address. One unreadable
//!   host must not make a whole lineage or an account unreadable, so this door
//!   stays as lenient as it was.
//!
//! Both refuse a name that is only dots (`..`): it becomes a directory name under
//! `.auth/`, so `..` would name the auth folder's parent. [`typed`] also refuses
//! what `url::Host` would refuse (a space, `@`, `\`, `?` and so on), for the same
//! reason.
//!
//! The tests below pin the rule, and say where it differs from the one it replaced,
//! `url::Host::parse`.

use crate::UriError;

/// The host a person's `input` names, or why it is not one.
pub(super) fn typed(input: &str) -> Result<url::Host, UriError> {
    let name = quiltx_core::dns::normalize_dns(input, false)
        .map_err(|e| UriError::Host(describe(&e.0)))?;
    let refuse = |why: &str| Err(UriError::Host(why.to_string()));
    if is_only_dots(&name) {
        return refuse("not a valid catalog host name");
    }
    // A name is kept as typed, so a character nobody can see would make two
    // catalogs that look alike, with two folders under `.auth/`.
    if name.chars().any(is_invisible) {
        return refuse("an invisible character is not part of a catalog host name");
    }
    // `url` would read a percent escape (`%2e%2e`, `a%41.test`) as the character it
    // stands for; the name is kept as typed, so the escape is refused instead.
    if name.contains('%') {
        return refuse("a percent escape is not part of a catalog host name");
    }
    // Not the value: `url` would turn a Unicode name into punycode. It is asked
    // only whether the name is a host, and which kind: a shorthand such as `127.1`
    // or `2130706433` passes `normalize_dns` as a name but is an IP address to
    // every URL.
    match url::Host::parse(&name) {
        Ok(url::Host::Domain(_)) => Ok(url::Host::Domain(name)),
        Ok(_) => refuse("an IP address is not a catalog host name"),
        Err(_) => refuse("not a valid catalog host name"),
    }
}

/// The host a stored `input` names, or why it is not one.
pub(super) fn stored(input: &str) -> Result<url::Host, UriError> {
    let typed_err = match typed(input) {
        Ok(host) => return Ok(host),
        Err(err) => err,
    };
    match url::Host::parse(input) {
        Ok(url::Host::Domain(name)) if is_only_dots(&name) => Err(typed_err),
        Ok(host) => Ok(host),
        Err(_) => Err(typed_err),
    }
}

/// A control character, a soft hyphen, a zero-width or direction-control character
/// or a byte-order mark: it prints as nothing.
fn is_invisible(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{ad}' | '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}' | '\u{feff}'
        )
}

fn is_only_dots(name: &str) -> bool {
    name.chars().all(|c| c == '.')
}

/// What `normalize_dns` said, in words that are about a host name rather than a
/// command line, and that do not repeat the input.
fn describe(quiltx_message: &str) -> String {
    if quiltx_message == "catalog DNS name is required" {
        return "a catalog host name is required".to_string();
    }
    if quiltx_message.starts_with("Custom ports") {
        return "a port is not part of a catalog host name".to_string();
    }
    if quiltx_message.starts_with("IP-address") {
        return "an IP address is not a catalog host name".to_string();
    }
    // `http://` and any other scheme alike; the text of the scheme is not echoed.
    if quiltx_message.starts_with("http://") {
        return "only https:// is supported".to_string();
    }
    "not a valid catalog host name".to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn domain(name: &str) -> url::Host {
        url::Host::Domain(name.to_string())
    }

    /// The host the typed `input` becomes, as the string a directory name shows.
    fn shown(input: &str) -> String {
        typed(input)
            .unwrap_or_else(|e| panic!("{input:?} should parse: {e}"))
            .to_string()
    }

    /// The message that refuses the typed `input`.
    fn refused(input: &str) -> String {
        let err = typed(input).expect_err(&format!("{input:?} should be refused"));
        let UriError::Host(message) = err else {
            panic!("{input:?}: not a host error: {err:?}")
        };
        message
    }

    // Unchanged by quiltx's rule: what was pinned before it.

    #[test]
    fn a_plain_name_is_kept() {
        assert_eq!(typed("quilt.test").unwrap(), domain("quilt.test"));
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
                refused(input),
                "a port is not part of a catalog host name",
                "{input}"
            );
        }
    }

    #[test]
    fn a_scheme_but_https_is_refused() {
        // One fixed message: the scheme the person typed is not echoed.
        for input in [
            "http://quilt.test",
            "HTTP://quilt.test",
            "ht\ttp://quilt.test",
            "ftp://quilt.test",
            "FTP://quilt.test",
            "javascript://quilt.test",
        ] {
            assert_eq!(refused(input), "only https:// is supported", "{input:?}");
        }
    }

    #[test]
    fn empty_is_refused() {
        for input in ["", "  ", "https://", "//quilt.test"] {
            assert_eq!(refused(input), "a catalog host name is required", "{input}");
        }
    }

    #[test]
    fn a_name_url_would_refuse_is_refused() {
        // Would otherwise become a directory name under `.auth/`.
        for input in [
            "quilt .test",
            "user@quilt.test",
            "quilt.test?x=1",
            "quilt.test#frag",
            "quilt.test\\x",
            "quilt.test:abc",
        ] {
            assert_eq!(refused(input), "not a valid catalog host name", "{input:?}");
        }
    }

    #[test]
    fn a_name_of_only_dots_is_refused() {
        // `.auth/..` would be the auth folder's parent.
        for input in [".", "..", "..."] {
            assert_eq!(refused(input), "not a valid catalog host name", "{input}");
        }
    }

    #[test]
    fn the_error_is_an_invalid_host_and_does_not_echo_the_input() {
        let err = typed("a b").unwrap_err();
        assert_eq!(
            err.to_string(),
            "Invalid host: not a valid catalog host name"
        );
    }

    // Behavior that differs from `url::Host::parse`, the rule this replaced. Each
    // test says what it was.

    #[test]
    fn a_unicode_name_is_kept_not_punycoded() {
        // Was: bücher.example -> xn--bcher-kva.example.
        assert_eq!(shown("bücher.example"), "bücher.example");
        assert_eq!(shown("BÜCHER.example"), "bücher.example");
        assert_eq!(shown("ß.test"), "ß.test");
        // Punycode typed by hand stays punycode, so a catalog stored under it is
        // still found.
        assert_eq!(shown("xn--bcher-kva.example"), "xn--bcher-kva.example");
    }

    /// Pinned, not endorsed: the name is kept as typed, so a compatibility
    /// character is a different catalog from the letter it resembles. `url` folded
    /// the full-width letters and `İ`, and dropped the soft hyphen.
    #[test]
    fn a_compatibility_character_is_kept_as_typed() {
        assert_eq!(shown("ＱＵＩＬＴ.test"), "ｑｕｉｌｔ.test");
        assert_eq!(shown("İstanbul.test"), "i\u{307}stanbul.test");
    }

    #[test]
    fn an_invisible_character_is_refused() {
        // Was: the soft hyphen and the zero-width space were dropped.
        let message = "an invisible character is not part of a catalog host name";
        assert_eq!(refused("quilt\u{ad}.test"), message);
        assert_eq!(refused("qu\u{200b}ilt.test"), message);
        assert_eq!(refused("quilt\u{feff}.test"), message);
        assert_eq!(refused("quilt\u{202e}.test"), message);
        assert_eq!(refused("qu\u{200d}ilt.test"), message);
    }

    #[test]
    fn an_ip_address_is_refused_in_every_spelling() {
        // Was: accepted as an IPv4 or IPv6 host. The shorthand forms are names to
        // `normalize_dns` and addresses to every URL.
        let message = "an IP address is not a catalog host name";
        for input in [
            "127.0.0.1",
            "10.1.2.3",
            "127.1",
            "0x7f.1",
            "2130706433",
            "0",
            "1.2.3",
        ] {
            assert_eq!(refused(input), message, "{input}");
        }
        for input in ["[::1]", "[2001:DB8::1]", "[::1"] {
            assert_eq!(refused(input), "not a valid catalog host name", "{input}");
        }
    }

    #[test]
    fn a_percent_escape_is_refused() {
        // Was: decoded, so `%2e%2e` was `..` and `a%41.test` was `aa.test`.
        let message = "a percent escape is not part of a catalog host name";
        for input in ["%2e%2e", "a%41.test", "a%2Fb.test", "100%.test"] {
            assert_eq!(refused(input), message, "{input}");
        }
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

    // The door for names read back from disk or a URI.

    fn read_back(input: &str) -> url::Host {
        stored(input).unwrap_or_else(|e| panic!("{input:?} should be read back: {e}"))
    }

    #[test]
    fn a_stored_name_takes_what_a_typed_one_takes() {
        for input in [
            "quilt.test",
            "Quilt.TEST",
            "quilt.test.",
            "bücher.example",
            "xn--bcher-kva.example",
            "https://quilt.test/",
        ] {
            assert_eq!(read_back(input), typed(input).unwrap(), "{input}");
        }
    }

    #[test]
    fn a_stored_ip_address_is_still_read_back() {
        // A catalog on an address can already be in an account folder or a
        // lineage; refusing it there would hide the account or the whole lineage.
        assert_eq!(
            read_back("127.0.0.1"),
            url::Host::<String>::Ipv4("127.0.0.1".parse().unwrap())
        );
        assert_eq!(read_back("127.1").to_string(), "127.0.0.1");
        assert_eq!(read_back("[::1]").to_string(), "[::1]");
        assert_eq!(read_back("[2001:DB8::1]").to_string(), "[2001:db8::1]");
    }

    #[test]
    fn a_stored_name_url_would_decode_is_still_read_back() {
        assert_eq!(read_back("a%41.test"), domain("aa.test"));
    }

    #[test]
    fn a_stored_name_of_only_dots_is_refused() {
        for input in [".", "..", "...", "%2e%2e"] {
            assert!(stored(input).is_err(), "{input}");
        }
    }

    #[test]
    fn a_stored_name_that_is_nothing_is_refused_with_the_typed_message() {
        let UriError::Host(message) = stored("a b").unwrap_err() else {
            panic!("not a host error")
        };
        assert_eq!(message, "not a valid catalog host name");
        assert!(stored("").is_err());
        assert!(stored("[::1").is_err());
    }
}
