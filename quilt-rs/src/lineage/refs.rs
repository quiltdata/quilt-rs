//!
//! Per-package lineage: one ref file per package under `.quilt/refs/`, a
//! domain config for the home, and a disposable cache of each remote tip.

#![cfg_attr(
    not(test),
    expect(dead_code, reason = "the ref store that uses them is not built yet")
)]

use std::ffi::OsStr;

use uuid::Uuid;

mod format;

/// What a file in `refs/<owner>/` is, read from its name.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum RefEntry {
    /// `<name>.json`, the package's ref.
    Ref(String),
    /// `<name>.json.latest`, the package's tip cache.
    Tip(String),
    /// `<name>.json.tmp-<uuid>`, a ref write that never landed.
    RefTemp(String),
    /// `<name>.json.latest.tmp-<uuid>`, a tip cache write that never landed.
    TipTemp(String),
    Other,
}

impl RefEntry {
    /// Reads the package name from the end of `file_name`, since a package
    /// name may itself end in `.json` or `.latest`.
    pub(crate) fn classify(file_name: &OsStr) -> Self {
        let Some(file_name) = file_name.to_str() else {
            return Self::Other;
        };
        let (file, temp) = match file_name.rsplit_once(".tmp-") {
            Some((file, uuid)) if Uuid::try_parse(uuid).is_ok() => (file, true),
            _ => (file_name, false),
        };
        let (name, tip) = if let Some(name) = file.strip_suffix(".json.latest") {
            (name, true)
        } else if let Some(name) = file.strip_suffix(".json") {
            (name, false)
        } else {
            return Self::Other;
        };
        if name.is_empty() {
            return Self::Other;
        }
        let name = name.to_string();
        match (tip, temp) {
            (false, false) => Self::Ref(name),
            (true, false) => Self::Tip(name),
            (false, true) => Self::RefTemp(name),
            (true, true) => Self::TipTemp(name),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;
    use crate::paths::DomainPaths;

    const UUID: &str = "67e55044-10b1-426f-9247-bb680e5fe0c8";

    fn classify(file_name: &str) -> RefEntry {
        RefEntry::classify(OsStr::new(file_name))
    }

    fn name(path: &std::path::Path) -> &OsStr {
        path.file_name().unwrap()
    }

    #[test]
    fn the_names_the_paths_give_classify_as_their_package() {
        let paths = DomainPaths::new(PathBuf::from("root"));
        for package in ["demo", "x.json", "x.latest", "x.json.latest", "x.tmp-y"] {
            let namespace = ("acme", package).into();
            assert_eq!(
                RefEntry::classify(name(&paths.package_ref(&namespace))),
                RefEntry::Ref(package.to_string()),
            );
            assert_eq!(
                RefEntry::classify(name(&paths.package_tip(&namespace))),
                RefEntry::Tip(package.to_string()),
            );
        }
    }

    #[test]
    fn a_package_named_like_a_ref_file_classifies_by_its_whole_name() {
        assert_eq!(classify("x.json.json"), RefEntry::Ref("x.json".into()));
        assert_eq!(classify("x.latest.json"), RefEntry::Ref("x.latest".into()));
        assert_eq!(
            classify("x.json.json.latest"),
            RefEntry::Tip("x.json".into())
        );
        assert_eq!(
            classify("x.latest.json.latest"),
            RefEntry::Tip("x.latest".into())
        );
    }

    #[test]
    fn a_leftover_temp_names_its_package() {
        assert_eq!(
            classify(&format!("x.json.json.tmp-{UUID}")),
            RefEntry::RefTemp("x.json".into())
        );
        assert_eq!(
            classify(&format!("x.json.latest.tmp-{UUID}")),
            RefEntry::TipTemp("x".into())
        );
    }

    #[test]
    fn anything_else_is_not_a_package_file() {
        for file_name in [
            "x",
            "x.latest",
            ".json",
            ".json.latest",
            "x.json.tmp-",
            "x.json.tmp-not-a-uuid",
            &format!(".json.tmp-{UUID}"),
            &format!("x.tmp-{UUID}"),
        ] {
            assert_eq!(classify(file_name), RefEntry::Other, "{file_name}");
        }
    }
}
