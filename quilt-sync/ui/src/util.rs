use quilt_uri::S3PackageUri;

use crate::commands::PackageSize;

/// Format the catalog HTTPS URL for a package URI, if its `catalog`
/// host is set. Used to power "Open in catalog" buttons.
pub fn catalog_url(uri: &S3PackageUri) -> Option<String> {
    uri.display_for_catalog().ok().map(|u| u.to_string())
}

/// Format the catalog HTTPS URL for an individual entry inside a package.
/// Returns `None` if the parent URI has no catalog host. Call from the
/// "Open in catalog" click handler so we only build the URL on click,
/// not for every row at render time.
pub fn entry_catalog_url(pkg_uri: &S3PackageUri, filename: &str) -> Option<String> {
    // Existence check: skip the clone when there's no catalog, since
    // the downstream `display_for_catalog()` would return None anyway.
    // `?` propagates the None; the bound `&Host` is discarded.
    pkg_uri.catalog.as_ref()?;
    let entry_uri = S3PackageUri {
        path: Some(std::path::PathBuf::from(filename)),
        ..pkg_uri.clone()
    };
    catalog_url(&entry_uri)
}

/// The `quilt+s3` handle of an installed package, from the parts the main
/// page's payload carries.
///
/// `Latest`, with no hash: a copied address points at the package, not at the
/// revision the copier happens to hold — a recipient asking for a pinned
/// revision would have to be given one deliberately.
///
/// A catalog host that does not parse is dropped rather than failing the whole
/// address, the catalog being the one optional part of it.
#[must_use]
pub fn package_uri(
    bucket: &str,
    namespace: &quilt_uri::Namespace,
    catalog: Option<&str>,
) -> S3PackageUri {
    S3PackageUri {
        catalog: catalog.and_then(|host| host.parse().ok()),
        bucket: bucket.to_string(),
        namespace: namespace.clone(),
        revision: quilt_uri::RevisionPointer::Tag(quilt_uri::Tag::Latest),
        path: None,
    }
}

/// The same handle, pointed at one file inside the package — the form the feed's
/// Copy action puts on the clipboard.
#[must_use]
pub fn file_uri(package: &S3PackageUri, path: &str) -> S3PackageUri {
    S3PackageUri {
        path: Some(std::path::PathBuf::from(path)),
        ..package.clone()
    }
}

/// Stringified catalog host, if set.
pub fn host_str(uri: &S3PackageUri) -> Option<String> {
    uri.catalog.as_ref().map(std::string::ToString::to_string)
}

/// Bucket name, treating an empty string as "unset" — matches the
/// historical `current_bucket` field semantics.
pub fn bucket_str(uri: &S3PackageUri) -> Option<String> {
    Some(uri.bucket.clone()).filter(|b| !b.is_empty())
}

/// Whether the package has a bucket to publish to. A catalog is not needed:
/// without one, the bucket is reached on this computer's AWS credentials.
pub fn has_bucket(uri: Option<&S3PackageUri>) -> bool {
    uri.is_some_and(|u| !u.bucket.is_empty())
}

/// Validate that `value` is a syntactically valid hostname
/// (two or more dot-separated labels, each starting and ending with
/// an ASCII alphanumeric character, with hyphens allowed in the middle).
pub fn is_valid_hostname(value: &str) -> bool {
    let mut count = 0u32;
    for label in value.split('.') {
        if !is_valid_label(label) {
            return false;
        }
        count += 1;
    }
    count >= 2
}

fn is_valid_label(label: &str) -> bool {
    let bytes = label.as_bytes();
    !bytes.is_empty()
        && bytes[0].is_ascii_alphanumeric()
        && bytes[bytes.len() - 1].is_ascii_alphanumeric()
        && bytes
            .iter()
            .all(|&b| b.is_ascii_alphanumeric() || b == b'-')
}

/// A logical size, in decimal units: whole bytes under 1 kB, else one decimal
/// in the largest unit that keeps the figure under 1000 — "999 B", "44.0 kB",
/// "3.4 MB", up to "18.4 EB" for `u64::MAX`. The one formatter for sizes on
/// every page. The space is a no-break one: in a narrow pane "1.2" ending a
/// line with "TB" starting the next reads as two figures.
pub fn format_size(bytes: u64) -> String {
    const UNITS: [&str; 6] = ["kB", "MB", "GB", "TB", "PB", "EB"];
    if bytes < 1000 {
        return format!("{bytes}\u{a0}B");
    }
    // In u128, so `n * 10 + scale / 2` cannot overflow for any u64: the
    // largest is under 2^68. Integers, so no float rounds "999.95" either way.
    let n = u128::from(bytes);
    let mut scale = 1000u128;
    let mut tenths = 0;
    let mut unit = UNITS[0];
    for next in UNITS {
        unit = next;
        tenths = (n * 10 + scale / 2) / scale;
        // Rounding up into the next unit's "1000.0" moves on to that unit.
        if tenths < 10_000 {
            break;
        }
        scale *= 1000;
    }
    format!("{}.{}\u{a0}{unit}", tenths / 10, tenths % 10)
}

/// A count, grouped in thousands: "140,000" is read, "140000" is counted.
pub fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// What a package holds here: the current revision's files, those not here,
/// those deleted here, and their bytes when they could be read.
///
/// Only the files the revision lists, at the size its manifest records: a new
/// file is not counted, a modified one counts at its recorded size, and one
/// deleted here counts as downloaded.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Held {
    pub total: usize,
    pub outstanding: usize,
    pub deleted_here: usize,
    pub size: Option<PackageSize>,
}

/// The count, with the size joined to it, and the files deleted here named:
/// Keeping's caption and a package row's tooltip, so the two say it alike.
/// An empty revision says so: "All files are downloaded" is true of nothing.
/// A size that cannot be read says nothing, not a dash: the count is still
/// true.
pub fn counted(held: Held) -> String {
    let Held {
        total,
        outstanding,
        deleted_here,
        size,
    } = held;
    if total == 0 {
        return "This revision has no files".to_string();
    }
    // The read guarantees `outstanding <= total`; a wrong count beats a panic.
    let here = total.saturating_sub(outstanding);
    let counted = match size {
        Some(size) if outstanding == 0 => {
            format!("All files are downloaded · {}", format_size(size.total))
        }
        Some(size) if here == 0 => {
            format!("No files are downloaded · {}", format_size(size.total))
        }
        Some(size) => format!(
            "{} of {} files · {} of {} downloaded",
            thousands(here),
            thousands(total),
            format_size(size.downloaded),
            format_size(size.total),
        ),
        None if outstanding == 0 => "All files are downloaded".to_string(),
        None if here == 0 => "No files are downloaded".to_string(),
        None => format!("{} of {} downloaded", thousands(here), thousands(total)),
    };
    with_deleted_here(counted, deleted_here)
}

/// `words · 3 deleted here`, or `words` alone with nothing deleted here. One
/// suffix for [`counted`] and the file pane's, so the two say it alike.
pub fn with_deleted_here(words: String, deleted_here: usize) -> String {
    if deleted_here == 0 {
        words
    } else {
        format!("{words} · {} deleted here", thousands(deleted_here))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ns(text: &str) -> quilt_uri::Namespace {
        quilt_uri::Namespace::try_from(text).expect("a namespace")
    }

    /// The one claim a copied address makes: the string. Round-tripped rather
    /// than only compared, because the point of the `quilt+s3` form is that
    /// something else parses it back — `S3PackageUri::try_from` is the reader on
    /// the other end of a paste.
    #[test]
    fn a_file_address_names_its_bucket_package_path_and_catalog() {
        let package = package_uri("team-bucket", &ns("user/plate-07"), Some("quilt.test"));
        let file = file_uri(&package, "runs/a/one.csv");

        assert_eq!(
            file.display(),
            "quilt+s3://team-bucket#package=user/plate-07&path=runs/a/one.csv&catalog=quilt.test"
        );
        assert_eq!(
            S3PackageUri::try_from(file.display().as_str()).expect("it parses back"),
            file
        );
    }

    /// No catalog is not no address: the bucket and the package are what a
    /// `quilt+s3` URI needs, and a deployment the payload never named must not
    /// stop a copy.
    #[test]
    fn a_package_with_no_catalog_still_has_an_address() {
        let package = package_uri("team-bucket", &ns("user/plate-07"), None);

        assert_eq!(
            file_uri(&package, "one.csv").display(),
            "quilt+s3://team-bucket#package=user/plate-07&path=one.csv"
        );
    }

    /// The revision is deliberately absent — `Latest`, which `display` renders
    /// as nothing. A copied address points at the package, not at whatever
    /// revision the copier happened to hold.
    #[test]
    fn a_copied_address_pins_no_revision() {
        let package = package_uri("b", &ns("user/p"), None);

        let address = package.display();
        let package_spec = address
            .split("#package=")
            .nth(1)
            .expect("a package in the fragment");

        assert_eq!(
            package_spec, "user/p",
            "the package spec carries no `:tag` and no `@hash`: {address}"
        );
    }

    #[test]
    fn valid_hostnames() {
        assert!(is_valid_hostname("example.com"));
        assert!(is_valid_hostname("sub.example.com"));
        assert!(is_valid_hostname("my-host.example.co.uk"));
        assert!(is_valid_hostname("a.b"));
        assert!(is_valid_hostname("123.456"));
        assert!(is_valid_hostname("a-1.b-2.c-3"));
    }

    #[test]
    fn rejects_single_label() {
        assert!(!is_valid_hostname("localhost"));
        assert!(!is_valid_hostname("example"));
    }

    #[test]
    fn rejects_empty_and_dot_only() {
        assert!(!is_valid_hostname(""));
        assert!(!is_valid_hostname("."));
        assert!(!is_valid_hostname(".."));
    }

    #[test]
    fn rejects_leading_trailing_hyphen() {
        assert!(!is_valid_hostname("-example.com"));
        assert!(!is_valid_hostname("example-.com"));
        assert!(!is_valid_hostname("example.-com"));
        assert!(!is_valid_hostname("example.com-"));
    }

    #[test]
    fn rejects_invalid_characters() {
        assert!(!is_valid_hostname("exam ple.com"));
        assert!(!is_valid_hostname("exam_ple.com"));
        assert!(!is_valid_hostname("example!.com"));
    }

    #[test]
    fn rejects_empty_labels() {
        assert!(!is_valid_hostname(".example.com"));
        assert!(!is_valid_hostname("example..com"));
        assert!(!is_valid_hostname("example.com."));
    }

    /// The words as read, with the no-break spaces shown as spaces.
    fn plain(words: &str) -> String {
        words.replace('\u{a0}', " ")
    }

    #[test]
    fn sizes_take_one_decimal_in_the_largest_unit() {
        assert_eq!(plain(&format_size(0)), "0 B");
        assert_eq!(plain(&format_size(512)), "512 B");
        assert_eq!(plain(&format_size(999)), "999 B");
        assert_eq!(plain(&format_size(1500)), "1.5 kB");
        assert_eq!(plain(&format_size(44_000)), "44.0 kB");
        assert_eq!(plain(&format_size(114_100)), "114.1 kB");
        assert_eq!(plain(&format_size(1_900_000)), "1.9 MB");
        assert_eq!(plain(&format_size(2_500_000)), "2.5 MB");
        assert_eq!(plain(&format_size(999_960)), "1.0 MB");
        assert_eq!(plain(&format_size(1_200_000_000_000)), "1.2 TB");
    }

    #[test]
    fn the_unit_is_held_to_its_number_by_a_no_break_space() {
        assert_eq!(format_size(512), "512\u{a0}B");
        assert_eq!(format_size(3_400_000), "3.4\u{a0}MB");
    }

    /// No u64 overflows the arithmetic, and rounding at each unit's edge moves
    /// on to the next unit rather than reading "1000.0".
    #[test]
    fn sizes_hold_at_the_unit_edges_and_at_the_top() {
        assert_eq!(plain(&format_size(1_000)), "1.0 kB");
        assert_eq!(plain(&format_size(999_949)), "999.9 kB");
        assert_eq!(plain(&format_size(999_950)), "1.0 MB");
        assert_eq!(plain(&format_size(999_949_999)), "999.9 MB");
        assert_eq!(plain(&format_size(999_950_000)), "1.0 GB");
        assert_eq!(plain(&format_size(999_950_000_000)), "1.0 TB");
        assert_eq!(plain(&format_size(999_949_999_999_999)), "999.9 TB");
        assert_eq!(plain(&format_size(999_950_000_000_000)), "1.0 PB");
        assert_eq!(plain(&format_size(999_950_000_000_000_000)), "1.0 EB");
        assert_eq!(plain(&format_size(u64::MAX - 1)), "18.4 EB");
        assert_eq!(plain(&format_size(u64::MAX)), "18.4 EB");
    }

    #[test]
    fn counts_carry_thousands_separators() {
        assert_eq!(thousands(0), "0");
        assert_eq!(thousands(999), "999");
        assert_eq!(thousands(1_000), "1,000");
        assert_eq!(thousands(4_312), "4,312");
        assert_eq!(thousands(140_000), "140,000");
        assert_eq!(thousands(1_234_567), "1,234,567");
    }

    #[test]
    fn a_complete_package_names_its_size_and_the_files_deleted_here() {
        let held = Held {
            total: 340,
            outstanding: 0,
            deleted_here: 2,
            size: Some(PackageSize {
                total: 3_400_000,
                downloaded: 3_400_000,
                backlog: 0,
            }),
        };
        assert_eq!(
            counted(held),
            "All files are downloaded \u{b7} 3.4\u{a0}MB \u{b7} 2 deleted here"
        );
    }
}
