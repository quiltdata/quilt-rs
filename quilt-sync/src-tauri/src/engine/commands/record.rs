//! Commands over the [record](crate::engine::record).

use std::collections::BTreeSet;

use quilt_uri::Namespace;
use serde::Serialize;

use crate::engine::model::QuiltModel;
use crate::engine::record::Entry;
use crate::engine::services::Services;

/// An entry as the window gets it: with whether its package is installed
/// now, since a package page cannot open one that is not.
#[derive(Serialize, Debug, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct RecordRow {
    #[serde(flatten)]
    pub entry: Entry,
    pub installed: bool,
}

/// Every entry kept, oldest first: the window's read on mount, on each
/// `record-changed` and on each `packages-changed`.
pub async fn get_record(s: &Services) -> Result<Vec<RecordRow>, String> {
    let entries = s.record.entries().await;
    // Unknown, every entry keeps its link.
    let installed: Option<BTreeSet<Namespace>> = s
        .model
        .get_installed_packages_list()
        .await
        .ok()
        .map(|all| all.into_iter().map(|p| p.namespace).collect());
    Ok(rows(entries, installed.as_ref()))
}

fn rows(entries: Vec<Entry>, installed: Option<&BTreeSet<Namespace>>) -> Vec<RecordRow> {
    entries
        .into_iter()
        .map(|entry| RecordRow {
            installed: installed.is_none_or(|all| all.contains(&entry.namespace)),
            entry,
        })
        .collect()
}

/// The reader has looked at the entries up to `up_to`: all of them from the
/// inbox, or `namespace`'s from its package page.
pub async fn mark_read(
    s: &Services,
    up_to: u64,
    namespace: Option<Namespace>,
) -> Result<(), String> {
    s.record.mark_read(up_to, namespace).await;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::engine::record::Kind;

    fn entry(namespace: (&str, &str)) -> Entry {
        Entry {
            id: 1,
            kind: Kind::Got,
            namespace: namespace.into(),
            revision: None,
            files: Vec::new(),
            outcome: None,
            at: chrono::DateTime::UNIX_EPOCH,
            read: false,
        }
    }

    #[test]
    fn a_row_says_whether_its_package_is_installed() {
        let installed = BTreeSet::from([Namespace::from(("acme", "here"))]);
        let rows = rows(
            vec![entry(("acme", "here")), entry(("acme", "gone"))],
            Some(&installed),
        );
        assert_eq!(
            rows.iter().map(|r| r.installed).collect::<Vec<_>>(),
            [true, false]
        );
        let json = serde_json::to_value(&rows[1]).unwrap();
        assert_eq!(json["installed"], false);
        assert_eq!(json["kind"], "got");
        // Unknown, every entry keeps its link.
        assert!(super::rows(vec![entry(("acme", "gone"))], None)[0].installed);
    }
}
