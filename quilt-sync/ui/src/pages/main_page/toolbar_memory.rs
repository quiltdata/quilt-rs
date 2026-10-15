//! The toolbar choices the main page keeps between visits.
//!
//! Each `Group:` and `Sort:` choice is kept in `localStorage` under its own
//! key, as a name rather than the label the toolbar shows, so a reworded label
//! does not lose it. The search is never kept: a forgotten query would hide
//! packages.

use super::GROUP_BUCKET;
use super::GROUP_NONE;
use super::GROUP_PACKAGE;
use super::GROUP_PREFIX;
use super::SORT_CHANGED;
use super::SORT_NAME;

/// One remembered `Select`: its key, and each option's stored name beside its
/// label, in the order the toolbar lists them. The first is the default.
pub(super) struct Choice {
    pub(super) key: &'static str,
    options: &'static [(&'static str, &'static str)],
}

pub(super) const PACKAGES_GROUP: Choice = Choice {
    key: "quilt-sync.main-page.group-packages",
    options: &[
        ("bucket", GROUP_BUCKET),
        ("prefix", GROUP_PREFIX),
        ("none", GROUP_NONE),
    ],
};

pub(super) const PACKAGES_SORT: Choice = Choice {
    key: "quilt-sync.main-page.sort",
    options: &[("changed", SORT_CHANGED), ("name", SORT_NAME)],
};

pub(super) const FILES_GROUP: Choice = Choice {
    key: "quilt-sync.main-page.group-files",
    options: &[("none", GROUP_NONE), ("package", GROUP_PACKAGE)],
};

impl Choice {
    /// The labels, as the toolbar's `Select` takes them.
    pub(super) fn labels(&self) -> Vec<String> {
        self.options
            .iter()
            .map(|(_, label)| (*label).to_string())
            .collect()
    }
}

fn local_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.local_storage().ok()?
}

/// The label the reader last chose. The default when nothing usable is
/// stored, or there is no storage to read.
pub(super) fn remembered_choice(choice: &Choice) -> String {
    let stored = local_storage().and_then(|storage| storage.get_item(choice.key).ok()?);
    let (_, label) = stored
        .and_then(|name| choice.options.iter().find(|(known, _)| *known == name))
        .unwrap_or(&choice.options[0]);
    (*label).to_string()
}

/// Store the reader's choice. Every failure is ignored — a quota, a private
/// mode, no storage at all.
pub(super) fn remember_choice(choice: &Choice, label: &str) {
    let Some(storage) = local_storage() else {
        return;
    };
    if let Some((name, _)) = choice.options.iter().find(|(_, known)| *known == label) {
        drop(storage.set_item(choice.key, name));
    }
}
