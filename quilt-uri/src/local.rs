use std::fmt;
use std::path::PathBuf;

use url::Url;

use crate::Namespace;
use crate::RevisionPointer;
use crate::UriError;
use crate::mask_user;
use crate::package::COPY_VALUE;
use crate::package::parse_after_authority;
use crate::package::render_fragment;

/// A local registry's name: one or more ASCII letters, digits, `-` or `.`,
/// the characters of a host name, and not `.` or `..`.
///
/// # Errors
///
/// Returns [`UriError::Package`] for any other name.
pub fn check_local_registry_name(name: &str) -> Result<(), UriError> {
    let valid = !name.is_empty()
        && name != "."
        && name != ".."
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'.'));
    if valid {
        Ok(())
    } else {
        Err(UriError::Package(format!(
            "a local registry's name is letters, digits, `-` and `.`; got {name:?}"
        )))
    }
}

/// A package in a local registry: `quilt+local://<name>#package=owner/name`,
/// with a revision and a path as in [`S3PackageUri`](crate::S3PackageUri). A
/// local package has no catalog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct LocalPackageUri {
    /// The registry's name. `None` for `quilt+local://#…`, which names this
    /// machine's local registry.
    pub registry: Option<String>,
    pub namespace: Namespace,
    pub revision: RevisionPointer,
    pub path: Option<PathBuf>,
}

impl TryFrom<&str> for LocalPackageUri {
    type Error = UriError;

    fn try_from(input: &str) -> Result<Self, Self::Error> {
        // The URL parser drops these, which before the fragment can name
        // another registry. The fragment is read as `quilt+s3://` reads it.
        let before_fragment = input.split('#').next().unwrap_or_default();
        if before_fragment.contains(['\t', '\n', '\r']) {
            return Err(UriError::Package(format!(
                "a local package URI holds no tab or line break before `#`: {:?}",
                mask_user(input)
            )));
        }
        let parsed_url = Url::parse(input)?;
        if parsed_url.scheme() != "quilt+local" {
            return Err(UriError::Package(format!(
                "expected quilt+local, got {}",
                parsed_url.scheme()
            )));
        }
        // The URL parser drops an empty user or port, so the name is read
        // from the text as written, right after the scheme's `//`.
        let Some(after_slashes) = input
            .split_once(':')
            .and_then(|(_, rest)| rest.strip_prefix("//"))
        else {
            return Err(UriError::Package(format!(
                "a local package URI starts `quilt+local://`: {:?}",
                mask_user(input)
            )));
        };
        let fragment = parse_after_authority(&parsed_url, input, "local", "the registry's name")?;
        if fragment.catalog.is_some() {
            return Err(UriError::Package(
                "a local package has no catalog".to_string(),
            ));
        }
        let written = after_slashes
            .split(['#', '/', '?'])
            .next()
            .unwrap_or_default();
        let registry = (!written.is_empty()).then_some(written);
        if let Some(name) = registry {
            check_local_registry_name(name)?;
        }
        Ok(Self {
            registry: registry.map(str::to_owned),
            namespace: fragment.namespace.try_into()?,
            revision: fragment.revision,
            path: fragment.path,
        })
    }
}

impl std::str::FromStr for LocalPackageUri {
    type Err = UriError;

    fn from_str(input: &str) -> Result<Self, Self::Err> {
        Self::try_from(input)
    }
}

/// The whole hash, and every fragment value escaped, as
/// [`S3PackageUri`](crate::S3PackageUri) writes it.
impl fmt::Display for LocalPackageUri {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let fragment = render_fragment(
            &self.namespace,
            &self.revision,
            self.path.as_deref(),
            None,
            true,
            COPY_VALUE,
        );
        write!(
            f,
            "quilt+local://{}#{fragment}",
            self.registry.as_deref().unwrap_or_default()
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::Tag;
    use crate::fixtures;

    /// Two slashes and no name: this machine's local registry.
    #[test]
    fn an_empty_authority_names_this_machine() {
        let expected = LocalPackageUri {
            registry: None,
            namespace: ("me", "analysis").into(),
            revision: RevisionPointer::Tag(Tag::Latest),
            path: None,
        };
        assert_eq!(
            LocalPackageUri::try_from("quilt+local://#package=me/analysis"),
            Ok(expected.clone())
        );
        assert_eq!(expected.to_string(), "quilt+local://#package=me/analysis");
    }

    #[test]
    fn a_named_registry_parses_and_round_trips() {
        let input = format!(
            "quilt+local://lab-mac.lan#package=me/analysis@{}&path=data/a%26b.csv",
            fixtures::TOP_HASH
        );
        let expected = LocalPackageUri {
            registry: Some("lab-mac.lan".to_string()),
            namespace: ("me", "analysis").into(),
            revision: RevisionPointer::Hash(fixtures::TOP_HASH.to_string()),
            path: Some(PathBuf::from("data/a&b.csv")),
        };
        assert_eq!(
            LocalPackageUri::try_from(input.as_str()),
            Ok(expected.clone())
        );
        assert_eq!(expected.to_string(), input);
    }

    /// The name is kept as written, so it can be compared with the one the
    /// registry keeps.
    #[test]
    fn a_name_keeps_its_case() {
        let uri = LocalPackageUri::try_from("quilt+local://Lab-Mac#package=me/a");
        assert_eq!(uri.map(|uri| uri.registry), Ok(Some("Lab-Mac".to_string())));
    }

    #[test]
    fn a_name_outside_host_name_characters_is_refused() {
        for input in [
            "quilt+local://lab_mac#package=me/a",
            "quilt+local://lab%20mac#package=me/a",
            "quilt+local://lab@local#package=me/a",
            "quilt+local://@name#package=me/a",
            "quilt+local://name:#package=me/a",
            "quilt+local://na\tme#package=me/a",
            "quilt+local://na\nme#package=me/a",
            "quilt+local:/\t/name#package=me/a",
            "quilt+local:\n//name#package=me/a",
            "quilt+local:\t//name#package=me/a&path=x://name",
            "quilt+local:\t//#package=me/a",
            "quilt+local:/\n/#package=me/a",
            "quilt+lo\rcal://name#package=me/a",
        ] {
            assert!(
                LocalPackageUri::try_from(input).is_err(),
                "{input} should not parse"
            );
        }
        for name in ["", ".", "..", "a_b", "a b", "a/b", "@local"] {
            assert!(check_local_registry_name(name).is_err(), "{name:?}");
        }
        for name in ["a", "lab-mac", "Lab-Mac.local", "10.0.0.1"] {
            assert!(check_local_registry_name(name).is_ok(), "{name:?}");
        }
    }

    /// No path follows the name, three slashes included, and the URI needs
    /// its `//`.
    #[test]
    fn only_a_name_comes_before_the_fragment() {
        for input in [
            "quilt+local:///#package=me/a",
            "quilt+local://name/#package=me/a",
            "quilt+local://name:1#package=me/a",
            "quilt+local://name?x#package=me/a",
            "quilt+local:#package=me/a",
            "quilt+local://name",
        ] {
            let err =
                LocalPackageUri::try_from(input).expect_err(&format!("{input} should not parse"));
            assert!(matches!(err, UriError::Package(_)), "{input}: {err}");
        }
    }

    /// An error that repeats the input masks a user and password in it.
    #[test]
    fn an_error_masks_a_user_and_password() {
        for input in [
            "quilt+local://KEY:secret@\tname#package=a/b",
            "quilt+local:KEY:secret@name#package=a/b",
        ] {
            let err = LocalPackageUri::try_from(input)
                .expect_err(input)
                .to_string();
            assert!(!err.contains("secret") && !err.contains("KEY"), "{err}");
            assert!(err.contains("***@"), "{err}");
        }
    }

    /// The fragment is read as `quilt+s3://` reads it, padding on a path
    /// trimmed.
    #[test]
    fn a_padded_path_is_trimmed_as_in_an_s3_uri() {
        let uri = LocalPackageUri::try_from("quilt+local://name#package=me/a&path=a.csv\t");
        assert_eq!(uri.map(|uri| uri.path), Ok(Some(PathBuf::from("a.csv"))));
    }

    #[test]
    fn a_local_package_has_no_catalog() {
        assert!(
            LocalPackageUri::try_from("quilt+local://name#package=me/a&catalog=quilt.test")
                .is_err()
        );
    }

    #[test]
    fn another_scheme_is_refused() {
        assert!(LocalPackageUri::try_from("quilt+s3://bucket#package=me/a").is_err());
    }
}
