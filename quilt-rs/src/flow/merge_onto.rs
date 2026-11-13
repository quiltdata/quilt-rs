//! A copy's change since the revision it shares with `latest`, applied onto
//! `latest` path by path: never within a file, nor within the package metadata.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::LogicalKey;
use crate::flow::list_revisions::holds_user_meta;
use crate::manifest::Manifest;
use crate::manifest::ManifestHeader;
use crate::manifest::ManifestRow;
use crate::manifest::normalized_entry_meta;

/// Why a merge onto `latest` refused; nothing of it may be published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeRefusal {
    /// Keys both sides changed to different rows, sorted; where there are
    /// none, each key the result would hold as a file and the first key under it.
    Paths(Vec<PathBuf>),
    /// Both sides changed the package `user_meta`, to different values.
    UserMeta,
}

/// `theirs` with the change from `base` to `ours` applied: its rows by
/// [`merge_rows_onto`], its `user_meta` compared as one value, and `ours`'
/// message under the rest of `theirs`' header.
pub fn merge_onto(
    base: &Manifest,
    ours: &Manifest,
    theirs: Manifest,
) -> Result<Manifest, MergeRefusal> {
    let rows = merge_rows_onto(&base.rows, &ours.rows, theirs.rows).map_err(MergeRefusal::Paths)?;
    let [base_meta, ours_meta, theirs_meta] =
        [&base.header, &ours.header, &theirs.header].map(|header| {
            header
                .user_meta
                .as_ref()
                .filter(|meta| holds_user_meta(meta))
        });
    let user_meta = if ours_meta == base_meta || ours_meta == theirs_meta {
        theirs.header.user_meta
    } else if theirs_meta == base_meta {
        ours.header.user_meta.clone()
    } else {
        return Err(MergeRefusal::UserMeta);
    };
    Ok(Manifest {
        header: ManifestHeader {
            message: ours.header.message.clone(),
            user_meta,
            ..theirs.header
        },
        rows,
    })
}

/// `theirs` changed only at the keys whose row differs from `base` to `ours`,
/// sorted by key. A row is its content hash and its entry `meta` as the top
/// hash reads it. Refuses with [`MergeRefusal::Paths`]' keys.
pub fn merge_rows_onto(
    base: &[ManifestRow],
    ours: &[ManifestRow],
    theirs: Vec<ManifestRow>,
) -> Result<Vec<ManifestRow>, Vec<PathBuf>> {
    let (base, ours) = (by_key(base), by_key(ours));
    let mut merged: BTreeMap<LogicalKey, ManifestRow> = theirs
        .into_iter()
        .map(|row| (row.logical_key.clone(), row))
        .collect();
    let keys: BTreeSet<&LogicalKey> = base.keys().chain(ours.keys()).copied().collect();
    let mut collisions = Vec::new();
    for key in keys {
        let (was, now) = (base.get(key).copied(), ours.get(key).copied());
        let current = merged.get(key);
        if same_row(was, now) || same_row(current, now) {
            continue;
        }
        if !same_row(current, was) {
            collisions.push(key.to_path_buf());
            continue;
        }
        match now {
            Some(row) => merged.insert(key.clone(), row.clone()),
            None => merged.remove(key),
        };
    }
    if !collisions.is_empty() {
        return Err(collisions);
    }
    // Keys sort by their `/` names, so a key that is a folder of others sorts
    // right before one of them.
    let merged_keys: Vec<&LogicalKey> = merged.keys().collect();
    let mut folders: Vec<PathBuf> = merged_keys
        .windows(2)
        .filter(|pair| pair[0].is_folder_of(pair[1]))
        .flat_map(|pair| [pair[0].to_path_buf(), pair[1].to_path_buf()])
        .collect();
    folders.dedup();
    if !folders.is_empty() {
        return Err(folders);
    }
    Ok(merged.into_values().collect())
}

fn by_key(rows: &[ManifestRow]) -> BTreeMap<&LogicalKey, &ManifestRow> {
    rows.iter().map(|row| (&row.logical_key, row)).collect()
}

fn same_row(a: Option<&ManifestRow>, b: Option<&ManifestRow>) -> bool {
    match (a, b) {
        (None, None) => true,
        (Some(a), Some(b)) => {
            a.hash == b.hash
                && normalized_entry_meta(a.meta.as_ref()) == normalized_entry_meta(b.meta.as_ref())
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use multihash::Multihash;

    use crate::Res;
    use crate::manifest::HeaderMessage;
    use serde_json::json;

    fn row(key: &str, content: &[u8]) -> Res<ManifestRow> {
        Ok(ManifestRow {
            logical_key: crate::LogicalKey::new(key)?,
            physical_key: format!("s3://b/{key}"),
            hash: Multihash::<256>::wrap(0x12, content)
                .unwrap()
                .try_into()
                .unwrap(),
            size: content.len() as u64,
            meta: None,
        })
    }

    fn with_entry_meta(row: ManifestRow, meta: serde_json::Value) -> ManifestRow {
        ManifestRow {
            meta: Some(meta),
            ..row
        }
    }

    fn rows(rows: Vec<ManifestRow>) -> Manifest {
        Manifest {
            rows,
            ..Manifest::default()
        }
    }

    fn user_meta(meta: Option<serde_json::Value>) -> Manifest {
        let mut manifest = Manifest::default();
        manifest.header.user_meta = meta;
        manifest
    }

    fn paths(keys: &[&str]) -> MergeRefusal {
        MergeRefusal::Paths(keys.iter().map(PathBuf::from).collect())
    }

    #[test]
    fn changes_to_different_paths_land_together() -> Res {
        let base = rows(vec![row("a.csv", b"1")?, row("gone.csv", b"x")?]);
        let ours = rows(vec![row("a.csv", b"mine")?, row("new.csv", b"n")?]);
        let theirs = rows(vec![
            row("a.csv", b"1")?,
            row("b.csv", b"theirs")?,
            row("gone.csv", b"x")?,
        ]);

        let merged = merge_onto(&base, &ours, theirs).expect("merged");

        assert_eq!(
            merged.rows,
            [
                row("a.csv", b"mine")?,
                row("b.csv", b"theirs")?,
                row("new.csv", b"n")?,
            ]
        );
        Ok(())
    }

    /// A `\\` is part of a name on every OS, so neither pair is one key or a
    /// file and its folder.
    #[test]
    fn keys_holding_a_backslash_merge_as_names() -> Res {
        let ours = rows(vec![row("z", b"mine")?]);
        let theirs = rows(vec![
            row("a/b", b"1")?,
            row("a\\b", b"2")?,
            row("data", b"3")?,
            row("data\\x.csv", b"4")?,
        ]);

        let merged = merge_onto(&Manifest::default(), &ours, theirs).expect("merged");

        assert_eq!(
            merged.rows,
            [
                row("a/b", b"1")?,
                row("a\\b", b"2")?,
                row("data", b"3")?,
                row("data\\x.csv", b"4")?,
                row("z", b"mine")?,
            ]
        );
        Ok(())
    }

    #[test]
    fn a_path_both_sides_changed_differently_refuses() -> Res {
        let base = rows(vec![row("a.csv", b"1")?, row("b.csv", b"1")?]);
        let ours = rows(vec![row("a.csv", b"mine")?, row("b.csv", b"1")?]);
        let theirs = rows(vec![row("a.csv", b"theirs")?, row("b.csv", b"2")?]);

        assert_eq!(merge_onto(&base, &ours, theirs), Err(paths(&["a.csv"])));
        Ok(())
    }

    #[test]
    fn a_path_both_sides_added_differently_refuses() -> Res {
        let ours = rows(vec![row("a.csv", b"mine")?]);
        let theirs = rows(vec![row("a.csv", b"theirs")?]);

        assert_eq!(
            merge_onto(&Manifest::default(), &ours, theirs),
            Err(paths(&["a.csv"]))
        );
        Ok(())
    }

    #[test]
    fn a_path_one_side_removed_and_the_other_changed_refuses() -> Res {
        let base = rows(vec![row("a.csv", b"1")?]);
        let theirs = rows(vec![row("a.csv", b"theirs")?]);

        assert_eq!(
            merge_onto(&base, &Manifest::default(), theirs),
            Err(paths(&["a.csv"]))
        );
        Ok(())
    }

    #[test]
    fn a_change_to_only_an_entrys_meta_collides_with_a_change_to_its_content() -> Res {
        let base = rows(vec![row("a.csv", b"1")?]);
        let ours = rows(vec![row("a.csv", b"mine")?]);
        let theirs = rows(vec![with_entry_meta(
            row("a.csv", b"1")?,
            json!({"reviewed": true}),
        )]);

        assert_eq!(merge_onto(&base, &ours, theirs), Err(paths(&["a.csv"])));
        Ok(())
    }

    #[test]
    fn an_entry_meta_written_as_empty_is_no_change() -> Res {
        let base = rows(vec![row("a.csv", b"1")?]);
        let ours = rows(vec![row("a.csv", b"mine")?]);
        let theirs = rows(vec![with_entry_meta(row("a.csv", b"1")?, json!({}))]);

        let merged = merge_onto(&base, &ours, theirs).expect("merged");

        assert_eq!(merged.rows, [row("a.csv", b"mine")?]);
        Ok(())
    }

    #[test]
    fn an_identical_change_on_both_sides_merges() -> Res {
        let base = rows(vec![row("a.csv", b"1")?]);
        let ours = rows(vec![row("a.csv", b"same")?, row("new.csv", b"n")?]);
        let theirs = rows(vec![row("a.csv", b"same")?, row("new.csv", b"n")?]);

        let merged = merge_onto(&base, &ours, theirs).expect("merged");

        assert_eq!(merged.rows, [row("a.csv", b"same")?, row("new.csv", b"n")?]);
        Ok(())
    }

    #[test]
    fn a_file_where_the_other_side_added_a_folder_refuses() -> Res {
        let ours = rows(vec![row("results", b"mine")?]);
        let theirs = rows(vec![row("results/output.csv", b"theirs")?]);

        assert_eq!(
            merge_onto(&Manifest::default(), &ours, theirs),
            Err(paths(&["results", "results/output.csv"]))
        );
        Ok(())
    }

    #[test]
    fn a_file_replaced_by_a_folder_merges() -> Res {
        let base = rows(vec![row("results", b"old")?]);
        let ours = rows(vec![row("results/output.csv", b"mine")?]);
        let theirs = rows(vec![row("others.csv", b"theirs")?, row("results", b"old")?]);

        let merged = merge_onto(&base, &ours, theirs).expect("merged");

        assert_eq!(
            merged.rows,
            [
                row("others.csv", b"theirs")?,
                row("results/output.csv", b"mine")?
            ]
        );
        Ok(())
    }

    #[test]
    fn metadata_changed_only_here_is_taken() {
        let base = user_meta(Some(json!({"owner": "a"})));
        let ours = user_meta(Some(json!({"owner": "b"})));
        let theirs = user_meta(Some(json!({"owner": "a"})));

        let merged = merge_onto(&base, &ours, theirs).expect("merged");

        assert_eq!(merged.header.user_meta, Some(json!({"owner": "b"})));
    }

    #[test]
    fn metadata_changed_only_in_latest_is_kept() {
        let base = user_meta(Some(json!({"owner": "a"})));
        let ours = user_meta(Some(json!({"owner": "a"})));
        let theirs = user_meta(Some(json!({"owner": "a", "task": "registered"})));

        let merged = merge_onto(&base, &ours, theirs).expect("merged");

        assert_eq!(
            merged.header.user_meta,
            Some(json!({"owner": "a", "task": "registered"})),
        );
    }

    #[test]
    fn metadata_changed_alike_on_both_sides_merges() {
        let base = user_meta(Some(json!({"owner": "a"})));
        let ours = user_meta(Some(json!({"owner": "b"})));
        let theirs = user_meta(Some(json!({"owner": "b"})));

        let merged = merge_onto(&base, &ours, theirs).expect("merged");

        assert_eq!(merged.header.user_meta, Some(json!({"owner": "b"})));
    }

    #[test]
    fn metadata_changed_differently_on_both_sides_refuses() {
        let base = user_meta(Some(json!({"owner": "a"})));
        let ours = user_meta(Some(json!({"owner": "a", "mine": 1})));
        let theirs = user_meta(Some(json!({"owner": "a", "theirs": 2})));

        assert_eq!(
            merge_onto(&base, &ours, theirs),
            Err(MergeRefusal::UserMeta),
            "two edits of one object are not combined key by key"
        );
    }

    #[test]
    fn null_absent_and_empty_metadata_are_one() {
        let base = user_meta(None);
        let ours = user_meta(Some(json!({})));
        let theirs = user_meta(Some(json!({"owner": "theirs"})));

        let merged = merge_onto(&base, &ours, theirs).expect("merged");

        assert_eq!(merged.header.user_meta, Some(json!({"owner": "theirs"})));
    }

    #[test]
    fn the_result_carries_this_message_under_latests_header() {
        let mut ours = Manifest::default();
        ours.header.message = HeaderMessage::Text("Mine".to_string());
        let mut theirs = Manifest::default();
        theirs.header.message = HeaderMessage::Text("Theirs".to_string());
        let workflow = crate::manifest::Workflow {
            config: "s3://b/.quilt/workflows/config.yml".parse().unwrap(),
            id: None,
        };
        theirs.header.workflow = Some(workflow.clone());

        let merged = merge_onto(&Manifest::default(), &ours, theirs).expect("merged");

        assert_eq!(merged.header.message(), Some("Mine"));
        assert_eq!(merged.header.workflow, Some(workflow));
    }
}
