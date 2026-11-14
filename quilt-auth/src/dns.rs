//! Catalog DNS normalization.
//!
//! A line-for-line port of `quiltx.identity.normalize_dns` (the Python version
//! is kept in `tests/oracle.py` as the oracle for the parity tests), including
//! the few pieces of `urllib.parse` it leans on: `urlsplit`'s scheme and
//! netloc split, `.hostname`, `.port`, and the way `int()` reads a port.
//!
//! Errors carry the exact message Python raised, because callers print it.
//!
//! Known, deliberate edges (see the parity tests): a port inside a URL that is
//! not plain ASCII digits is rejected the way Python 3.12+ rejects it ("could
//! not be cast"), where 3.10 and 3.11 also accepted a leading sign, spaces or
//! underscores; and Python 3.10 took a scheme that starts with a digit, `+`,
//! `-` or `.`, which 3.11+ does not. Text that is not valid UTF-8 (a lone
//! surrogate from an undecodable command-line argument) cannot cross the FFI
//! boundary; the Python wrapper turns that into a `ValueError`.

use std::fmt::Write as _;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::str::FromStr;

use unicode_normalization::UnicodeNormalization;
use unicode_properties::{GeneralCategory, UnicodeGeneralCategory};

/// The message Python's `ValueError` carried.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsError(pub String);

type Res<T> = Result<T, DnsError>;

fn err<T>(message: impl Into<String>) -> Res<T> {
    Err(DnsError(message.into()))
}

const HTTP_NOT_SUPPORTED: &str =
    "http:// is not supported. Use https:// or pass the DNS name directly.";

/// Python's `str.isspace()` for one character.
fn is_py_space(c: char) -> bool {
    matches!(
        c,
        '\u{9}'..='\u{d}'
            | '\u{1c}'..='\u{20}'
            | '\u{85}'
            | '\u{a0}'
            | '\u{1680}'
            | '\u{2000}'..='\u{200a}'
            | '\u{2028}'
            | '\u{2029}'
            | '\u{202f}'
            | '\u{205f}'
            | '\u{3000}'
    )
}

/// Python's `str.strip()` with no argument.
fn py_strip(s: &str) -> &str {
    s.trim_matches(is_py_space)
}

/// `repr()` of a Python `str`, for the messages that quote one.
fn py_repr(s: &str) -> String {
    let quote = if s.contains('\'') && !s.contains('"') {
        '"'
    } else {
        '\''
    };
    let mut out = String::from(quote);
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            c if c.is_ascii() && !c.is_ascii_control() => out.push(c),
            c if c.is_ascii() => {
                let _ = write!(out, "\\x{:02x}", c as u32);
            }
            c if c.is_control() || is_py_space(c) => {
                let n = c as u32;
                if n < 0x100 {
                    let _ = write!(out, "\\x{n:02x}");
                } else if n < 0x10000 {
                    let _ = write!(out, "\\u{n:04x}");
                } else {
                    let _ = write!(out, "\\U{n:08x}");
                }
            }
            c => out.push(c),
        }
    }
    out.push(quote);
    out
}

/// `quiltx.identity.is_localhost`.
fn is_localhost(dns: &str) -> bool {
    py_strip(dns).to_lowercase() == "localhost"
}

/// A Unicode decimal digit (category Nd): what Python's `\d` and `int()` accept.
fn is_decimal(c: char) -> bool {
    c.general_category() == GeneralCategory::DecimalNumber
}

/// `quiltx.identity._is_ip_address`: a dotted-quad of 1-3 digit groups, or
/// anything with a colon.
fn is_ip_address(host: &str) -> bool {
    let quad = {
        let parts: Vec<&str> = host.split('.').collect();
        parts.len() == 4
            && parts
                .iter()
                .all(|p| (1..=3).contains(&p.chars().count()) && p.chars().all(is_decimal))
    };
    quad || host.contains(':')
}

/// The pieces of `urlsplit(url)` that `normalize_dns` reads.
struct Split {
    scheme: String,
    netloc: String,
}

const SCHEME_CHARS: &str = "abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789+-.";

/// `urllib.parse.urlsplit`, down to the scheme and the netloc.
fn urlsplit(url: &str) -> Res<Split> {
    // Leading C0 controls and spaces go, then tab, CR and LF wherever they are.
    let cleaned: String = url
        .trim_start_matches(|c: char| c <= ' ')
        .chars()
        .filter(|c| !matches!(c, '\t' | '\r' | '\n'))
        .collect();
    let mut rest: &str = &cleaned;
    let mut scheme = String::new();
    if let Some(i) = rest.find(':') {
        let head = &rest[..i];
        let starts_alpha = head.chars().next().is_some_and(|c| c.is_ascii_alphabetic());
        if i > 0 && starts_alpha && head.chars().all(|c| SCHEME_CHARS.contains(c)) {
            scheme = head.to_lowercase();
            rest = &rest[i + 1..];
        }
    }
    let mut netloc = String::new();
    if let Some(after) = rest.strip_prefix("//") {
        let end = after.find(['/', '?', '#']).unwrap_or(after.len());
        netloc = after[..end].to_string();
        let has_open = netloc.contains('[');
        let has_close = netloc.contains(']');
        if has_open != has_close {
            return err("Invalid IPv6 URL");
        }
        if has_open {
            check_bracketed_netloc(&netloc)?;
        }
        check_netloc_nfkc(&netloc)?;
    }
    Ok(Split { scheme, netloc })
}

/// `urllib.parse._check_netloc`: a non-ASCII netloc must not contain a
/// character that NFKC-normalizes into one of `/?#@:`.
fn check_netloc_nfkc(netloc: &str) -> Res<()> {
    if netloc.is_ascii() {
        return Ok(());
    }
    let stripped: String = netloc
        .chars()
        .filter(|c| !matches!(c, '@' | ':' | '#' | '?'))
        .collect();
    let normalized: String = stripped.nfkc().collect();
    if stripped != normalized && normalized.contains(['/', '?', '#', '@', ':']) {
        return err(format!(
            "netloc '{netloc}' contains invalid characters under NFKC normalization"
        ));
    }
    Ok(())
}

/// `urllib.parse._check_bracketed_netloc`: the part of the netloc after any
/// `user@` must be exactly `[host]` or `[host]:port`, with a valid host.
fn check_bracketed_netloc(netloc: &str) -> Res<()> {
    let hostinfo = netloc.rsplit_once('@').map_or(netloc, |(_, h)| h);
    let hostname = if let Some((before, bracketed)) = hostinfo.split_once('[') {
        if !before.is_empty() {
            return err("Invalid IPv6 URL");
        }
        let (hostname, port) = bracketed.split_once(']').unwrap_or((bracketed, ""));
        if !port.is_empty() && !port.starts_with(':') {
            return err("Invalid IPv6 URL");
        }
        hostname
    } else {
        hostinfo.split_once(':').map_or(hostinfo, |(h, _)| h)
    };
    check_bracketed_host(hostname)
}

/// `urllib.parse._check_bracketed_host`: what sits between `[` and `]` must be
/// an IPv6 address or an `IPvFuture` literal.
fn check_bracketed_host(host: &str) -> Res<()> {
    if let Some(rest) = host.strip_prefix(['v', 'V']) {
        // \Av[a-fA-F0-9]+\..+\Z
        let hex = rest.bytes().take_while(u8::is_ascii_hexdigit).count();
        let tail = &rest[hex..];
        let ok = hex > 0 && tail.strip_prefix('.').is_some_and(|t| !t.is_empty());
        return if ok {
            Ok(())
        } else {
            err("IPvFuture address is invalid")
        };
    }
    let (addr, scope) = match host.split_once('%') {
        Some((a, s)) => (a, Some(s)),
        None => (host, None),
    };
    let scope_ok = scope.is_none_or(|s| !s.is_empty() && !s.contains(['%', '/']));
    if Ipv4Addr::from_str(host).is_ok() {
        return err("An IPv4 address cannot be in brackets");
    }
    if scope_ok && Ipv6Addr::from_str(addr).is_ok() {
        return Ok(());
    }
    err(format!(
        "{} does not appear to be an IPv4 or IPv6 address",
        py_repr(host)
    ))
}

impl Split {
    /// `SplitResult._hostinfo`: `(hostname, port)` as raw text.
    fn hostinfo(&self) -> (&str, Option<&str>) {
        let hostinfo = self
            .netloc
            .rsplit_once('@')
            .map_or(&*self.netloc, |(_, h)| h);
        let (hostname, port) = if let Some((_, bracketed)) = hostinfo.split_once('[') {
            let (hostname, after) = bracketed.split_once(']').unwrap_or((bracketed, ""));
            (hostname, after.split_once(':').map_or("", |(_, p)| p))
        } else {
            hostinfo.split_once(':').unwrap_or((hostinfo, ""))
        };
        (hostname, if port.is_empty() { None } else { Some(port) })
    }

    /// `SplitResult.hostname`: lowercased, except a `%zone` suffix.
    fn hostname(&self) -> Option<String> {
        let (hostname, _) = self.hostinfo();
        if hostname.is_empty() {
            return None;
        }
        Some(match hostname.split_once('%') {
            Some((host, zone)) => format!("{}%{zone}", host.to_lowercase()),
            None => hostname.to_lowercase(),
        })
    }

    /// `SplitResult.port`, which raises for a bad one.
    fn port(&self) -> Res<Option<()>> {
        let Some(port) = self.hostinfo().1 else {
            return Ok(None);
        };
        if port.is_empty() || !port.bytes().all(|b| b.is_ascii_digit()) {
            return err(format!(
                "Port could not be cast to integer value as {}",
                py_repr(port)
            ));
        }
        // Compare by digits: the value may be longer than any integer type.
        let significant = port.trim_start_matches('0');
        if significant.len() > 5 || (significant.len() == 5 && significant > "65535") {
            return err("Port out of range 0-65535");
        }
        Ok(Some(()))
    }
}

/// Whether `int(text)` would accept `text`.
fn py_int_accepts(text: &str) -> bool {
    // int() strips C whitespace (not the ASCII separators U+001C-001F that
    // str.strip() also strips) and any non-ASCII Python space.
    let body = text.trim_matches(|c: char| {
        if c.is_ascii() {
            matches!(c, '\t'..='\r' | ' ')
        } else {
            is_py_space(c)
        }
    });
    let body = body.strip_prefix(['+', '-']).unwrap_or(body);
    let mut previous_digit = false;
    let mut any = false;
    for c in body.chars() {
        match c {
            c if is_decimal(c) => {
                previous_digit = true;
                any = true;
            }
            '_' if previous_digit => previous_digit = false,
            _ => return false,
        }
    }
    any && previous_digit
}

/// `quiltx.identity.normalize_dns`.
///
/// # Errors
///
/// A [`DnsError`] carrying the message Python's `ValueError` carried.
pub fn normalize_dns(value: &str, insecure: bool) -> Res<String> {
    let value = py_strip(value);
    if value.is_empty() {
        return err("catalog DNS name is required");
    }

    let http_prefix = value
        .get(..7)
        .is_some_and(|p| p.eq_ignore_ascii_case("http://"));
    if http_prefix && !insecure {
        return err(HTTP_NOT_SUPPORTED);
    }

    let host: String;
    let has_port: bool;
    if value.contains("://") {
        let parsed = urlsplit(value)?;
        if parsed.scheme != "https" && parsed.scheme != "http" {
            return err(HTTP_NOT_SUPPORTED);
        }
        host = parsed.hostname().unwrap_or_default();
        has_port = parsed.port()?.is_some();
        if parsed.scheme == "http" && !(insecure && is_localhost(&host)) {
            return err("http:// is only supported with --insecure for localhost.");
        }
    } else {
        let host_part = value.split('/').next().unwrap_or("");
        if host_part.contains(':') {
            let mut pieces = host_part.split(':');
            host = pieces.next().unwrap_or("").to_string();
            let port_text = pieces.next().unwrap_or("");
            if port_text.is_empty() {
                has_port = false;
            } else if py_int_accepts(port_text) {
                has_port = true;
            } else {
                return err(format!(
                    "invalid literal for int() with base 10: {}",
                    py_repr(port_text)
                ));
            }
        } else {
            host = host_part.to_string();
            has_port = false;
        }
    }

    let host = host.to_lowercase();
    if host.is_empty() {
        return err("catalog DNS name is required");
    }
    if has_port {
        return err("Custom ports are not supported. Use the bare DNS name.");
    }
    if is_ip_address(&host) {
        return err("IP-address identifiers are not supported. Use the DNS name.");
    }
    if insecure && !is_localhost(&host) {
        return err(format!(
            "--insecure is only supported for localhost; refusing to use insecure transport for {host}."
        ));
    }
    Ok(host)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok(value: &str) -> String {
        normalize_dns(value, false).unwrap()
    }

    fn fails(value: &str, insecure: bool) -> String {
        normalize_dns(value, insecure).unwrap_err().0
    }

    #[test]
    fn plain_names_are_lowercased_and_trimmed() {
        assert_eq!(
            ok("customer-acme.quilt.example"),
            "customer-acme.quilt.example"
        );
        assert_eq!(ok("  Nightly.QuiltTest.com \n"), "nightly.quilttest.com");
        assert_eq!(ok("host/some/path"), "host");
        assert_eq!(ok("host:"), "host");
    }

    #[test]
    fn https_urls_reduce_to_the_host() {
        assert_eq!(
            ok("https://customer-acme.quilt.example"),
            "customer-acme.quilt.example"
        );
        assert_eq!(
            ok("https://customer-acme.quilt.example/"),
            "customer-acme.quilt.example"
        );
        assert_eq!(ok("https://x.example/some/path?q=1#f"), "x.example");
        assert_eq!(
            ok("HTTPS://Customer-Acme.Quilt.Example"),
            "customer-acme.quilt.example"
        );
        assert_eq!(ok("https://user:pw@x.example"), "x.example");
        assert_eq!(ok("https://x.example:"), "x.example");
        assert_eq!(ok("https://x.example\t\n"), "x.example");
        assert_eq!(ok("ht\ttps://x.example"), "x.example");
    }

    #[test]
    fn http_is_refused_without_insecure() {
        let msg = HTTP_NOT_SUPPORTED;
        assert_eq!(fails("http://x.example", false), msg);
        assert_eq!(fails("HTTP://x.example", false), msg);
        assert_eq!(fails("ftp://x.example", false), msg);
        assert_eq!(fails("://x.example", false), msg);
        assert_eq!(fails("ht tp://x.example", false), msg);
    }

    #[test]
    fn insecure_allows_only_localhost() {
        assert_eq!(normalize_dns("localhost", true).unwrap(), "localhost");
        assert_eq!(
            normalize_dns("http://LocalHost", true).unwrap(),
            "localhost"
        );
        assert_eq!(
            fails("http://x.example", true),
            "http:// is only supported with --insecure for localhost."
        );
        assert_eq!(
            fails("x.example", true),
            "--insecure is only supported for localhost; refusing to use insecure transport for x.example."
        );
        assert_eq!(
            fails("https://x.example", true),
            "--insecure is only supported for localhost; refusing to use insecure transport for x.example."
        );
    }

    #[test]
    fn empty_input_and_empty_hosts() {
        let msg = "catalog DNS name is required";
        assert_eq!(fails("", false), msg);
        assert_eq!(fails("   \u{a0}\u{3000}", false), msg);
        assert_eq!(fails("https://", false), msg);
        assert_eq!(fails("https:///path", false), msg);
        assert_eq!(fails(":443", false), msg);
    }

    #[test]
    fn ports_are_refused() {
        let msg = "Custom ports are not supported. Use the bare DNS name.";
        assert_eq!(fails("host:8443", false), msg);
        assert_eq!(fails("host:0", false), msg);
        assert_eq!(fails("host:+1", false), msg);
        assert_eq!(fails("host:1_0", false), msg);
        assert_eq!(fails("host: 7 ", false), msg);
        assert_eq!(fails("https://x.example:8443", false), msg);
        assert_eq!(fails("https://x.example:65535", false), msg);
        assert_eq!(fails("https://x.example:000065535", false), msg);
    }

    #[test]
    fn bad_ports_keep_pythons_messages() {
        assert_eq!(
            fails("host:abc", false),
            "invalid literal for int() with base 10: 'abc'"
        );
        assert_eq!(
            fails("host:1__0", false),
            "invalid literal for int() with base 10: '1__0'"
        );
        assert_eq!(
            fails("host:_1", false),
            "invalid literal for int() with base 10: '_1'"
        );
        assert_eq!(
            fails("host:-", false),
            "invalid literal for int() with base 10: '-'"
        );
        assert_eq!(
            fails("a:b:c", false),
            "invalid literal for int() with base 10: 'b'"
        );
        assert_eq!(
            fails("https://x.example:abc", false),
            "Port could not be cast to integer value as 'abc'"
        );
        assert_eq!(
            fails("https://x.example:65536", false),
            "Port out of range 0-65535"
        );
        assert_eq!(
            fails("https://x.example:99999999999999999999", false),
            "Port out of range 0-65535"
        );
    }

    #[test]
    fn ip_literals_are_refused() {
        let msg = "IP-address identifiers are not supported. Use the DNS name.";
        assert_eq!(fails("10.0.0.5", false), msg);
        assert_eq!(fails("https://10.0.0.5", false), msg);
        assert_eq!(fails("999.1.1.1", false), msg);
        assert_eq!(fails("https://[::1]", false), msg);
        assert_eq!(
            fails("https://[::1]:443", false),
            "Custom ports are not supported. Use the bare DNS name."
        );
        // Unicode decimal digits count, as Python's \d does.
        assert_eq!(fails("١.٢.٣.٤", false), msg);
        // Not a dotted quad, so a name.
        assert_eq!(ok("10.0.0"), "10.0.0");
        assert_eq!(ok("1.2.3.4.5"), "1.2.3.4.5");
        assert_eq!(ok("1234.1.1.1"), "1234.1.1.1");
    }

    #[test]
    fn brackets_follow_python() {
        assert_eq!(fails("https://[::1", false), "Invalid IPv6 URL");
        assert_eq!(fails("https://::1]", false), "Invalid IPv6 URL");
        assert_eq!(
            fails("https://[abc]", false),
            "'abc' does not appear to be an IPv4 or IPv6 address"
        );
        assert_eq!(
            fails("https://[1.2.3.4]", false),
            "An IPv4 address cannot be in brackets"
        );
        assert_eq!(
            fails("https://[vx.y]", false),
            "IPvFuture address is invalid"
        );
        assert_eq!(ok("https://[v1.x]"), "v1.x");
        // The plain-name branch takes the first segment before a colon, brackets and all.
        assert_eq!(ok("[::1]"), "[");
    }

    #[test]
    fn hostname_keeps_the_case_of_a_zone() {
        assert_eq!(
            fails("https://[FE80::1%ETH0]", false),
            "IP-address identifiers are not supported. Use the DNS name."
        );
    }

    #[test]
    fn python_whitespace_is_stripped() {
        assert_eq!(ok("\u{1c}x.example\u{85}"), "x.example");
        assert_eq!(ok("\u{2003}x.example\u{2028}"), "x.example");
        // Not whitespace to Python: kept, then lowercased.
        assert_eq!(ok("\u{200b}X"), "\u{200b}x");
    }

    #[test]
    fn repr_matches_python() {
        assert_eq!(py_repr("abc"), "'abc'");
        assert_eq!(py_repr("a'b"), "\"a'b\"");
        assert_eq!(py_repr("a'b\"c"), "'a\\'b\"c'");
        assert_eq!(py_repr("a\\b\n\t\r"), "'a\\\\b\\n\\t\\r'");
        assert_eq!(py_repr("\u{1}\u{7f}"), "'\\x01\\x7f'");
        assert_eq!(py_repr("\u{a0}\u{2003}"), "'\\xa0\\u2003'");
        assert_eq!(py_repr("é"), "'é'");
    }

    #[test]
    fn int_acceptance_matches_python() {
        for good in [
            "0",
            "007",
            "+1",
            "-1",
            " 12 ",
            "1_0",
            "1_0_0",
            "\u{3000}5",
            "٣",
            "１２",
        ] {
            assert!(py_int_accepts(good), "{good:?}");
        }
        for bad in [
            "", " ", "+", "_1", "1_", "1__0", "1 2", "0x10", "1.0", "²", "٣x",
        ] {
            assert!(!py_int_accepts(bad), "{bad:?}");
        }
    }
}
