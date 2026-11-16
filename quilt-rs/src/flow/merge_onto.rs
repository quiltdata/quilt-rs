//! A copy's change since the revision it shares with `latest`, applied onto
//! `latest` path by path: never within a file, nor within the package metadata.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::PathBuf;

use crate::Error;
use crate::LogicalKey;
use crate::error::ManifestError;
use crate::flow::list_revisions::written_user_meta;
use crate::manifest::DirMeta;
use crate::manifest::Manifest;
use crate::manifest::ManifestHeader;
use crate::manifest::ManifestRow;
use crate::manifest::check_tree;
use crate::manifest::same_entry_meta;

/// Why a merge onto `latest` refused; nothing of it may be published.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MergeRefusal {
    /// Keys both sides changed to different rows, sorted; where there are
    /// none, each key the result would hold as a file and the first key under it.
    /// Folder keys keep their trailing `/`, so a caller can tell a folder from
    /// a file of the same name.
    Paths(Vec<PathBuf>),
    /// Both sides changed the package `user_meta`, to different values.
    UserMeta,
    /// The merged rows and folder metadata do not form a valid tree, for a
    /// reason other than a file and folder sharing a name.
    Invalid(String),
}

/// Why [`merge_dir_meta`] could not produce a merged folder list.
#[derive(Debug, thiserror::Error)]
pub enum MergeDirMetaError {
    /// Folder metadata both sides changed differently, or a file and folder
    /// that share a name. Folder keys keep their trailing `/`.
    #[error("folder metadata changed differently on both sides, or clashes with a file: {0:?}")]
    Paths(Vec<PathBuf>),
    /// The merged tree is invalid for another reason, such as a duplicate
    /// logical key.
    #[error("merged tree is invalid: {0}")]
    Invalid(Error),
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
    let dir_meta = match merge_dir_meta(&base.dir_meta, &ours.dir_meta, &theirs.dir_meta, &rows) {
        Ok(dirs) => dirs,
        Err(MergeDirMetaError::Paths(paths)) => return Err(MergeRefusal::Paths(paths)),
        Err(MergeDirMetaError::Invalid(err)) => return Err(MergeRefusal::Invalid(err.to_string())),
    };
    let [base_meta, ours_meta, theirs_meta] =
        [&base.header, &ours.header, &theirs.header].map(written_user_meta);
    let (user_meta, user_meta_numbers) = if ours_meta == base_meta || ours_meta == theirs_meta {
        (theirs.header.user_meta, theirs.header.user_meta_numbers)
    } else if theirs_meta == base_meta {
        (
            ours.header.user_meta.clone(),
            ours.header.user_meta_numbers.clone(),
        )
    } else {
        return Err(MergeRefusal::UserMeta);
    };
    Ok(Manifest {
        header: ManifestHeader {
            message: ours.header.message.clone(),
            user_meta,
            user_meta_numbers,
            ..theirs.header
        },
        rows,
        dir_meta,
    })
}

/// `theirs` with the folders' metadata that `ours` changed from `base`, as
/// [`merge_onto`] does for `user_meta`: a folder both sides changed to
/// different metadata refuses, with the folder's key (and a trailing `/`).
/// Metadata that says nothing counts as none. Sorted by key. `rows` are the
/// merged rows, which no folder's name may be the file of, or sit under one.
///
/// # Errors
///
/// The keys of the folders both sides changed differently, or else of the
/// file and folder that share a name (the file without a slash, the folder
/// with one), as [`MergeDirMetaError::Paths`]. Other tree errors, such as a
/// duplicate logical key, are [`MergeDirMetaError::Invalid`].
pub fn merge_dir_meta(
    base: &[DirMeta],
    ours: &[DirMeta],
    theirs: &[DirMeta],
    rows: &[ManifestRow],
) -> Result<Vec<DirMeta>, MergeDirMetaError> {
    let meta_of = |dirs: &'_ [DirMeta]| -> BTreeMap<LogicalKey, serde_json::Value> {
        dirs.iter()
            .filter(|dir| dir.holds_meta())
            .map(|dir| (dir.logical_key.clone(), dir.meta.clone()))
            .collect()
    };
    let (base, ours) = (meta_of(base), meta_of(ours));
    let mut merged = meta_of(theirs);
    let keys: BTreeSet<&LogicalKey> = base.keys().chain(ours.keys()).collect();
    let mut collisions = Vec::new();
    for key in keys {
        let (was, now) = (base.get(key), ours.get(key));
        let current = merged.get(key);
        if was == now || current == now {
            continue;
        }
        if current != was {
            collisions.push(dir_path(key));
            continue;
        }
        match now {
            Some(meta) => merged.insert(key.clone(), meta.clone()),
            None => merged.remove(key),
        };
    }
    if !collisions.is_empty() {
        return Err(MergeDirMetaError::Paths(collisions));
    }
    let merged: Vec<DirMeta> = merged
        .into_iter()
        .map(|(logical_key, meta)| DirMeta { logical_key, meta })
        .collect();
    match check_tree(rows, &merged) {
        Ok(()) => Ok(merged),
        Err(Error::Manifest(ManifestError::FileAndFolder(key))) => Err(MergeDirMetaError::Paths(
            file_and_folder_paths(&key, rows, &merged),
        )),
        Err(err) => Err(MergeDirMetaError::Invalid(err)),
    }
}

/// A folder's key as a path, with the trailing `/` its manifest row has.
fn dir_path(key: &LogicalKey) -> PathBuf {
    PathBuf::from(format!("{key}/"))
}

/// Paths for a file/folder clash: the file without a slash, the folder with
/// one, so a caller can tell them apart.
fn file_and_folder_paths(key: &str, rows: &[ManifestRow], dirs: &[DirMeta]) -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if rows.iter().any(|row| row.logical_key.to_string() == key) {
        paths.push(PathBuf::from(key));
    }
    if dirs.iter().any(|dir| dir.logical_key.to_string() == key) {
        paths.push(PathBuf::from(format!("{key}/")));
    }
    if let Ok(file) = LogicalKey::new(key) {
        for dir in dirs {
            if file.is_folder_of(&dir.logical_key) {
                if !paths.iter().any(|path| path.as_os_str() == key) {
                    paths.push(PathBuf::from(key));
                }
                paths.push(dir_path(&dir.logical_key));
            }
        }
    }
    if paths.is_empty() {
        paths.push(PathBuf::from(key));
    }
    paths
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
        (Some(a), Some(b)) => a.hash == b.hash && same_entry_meta(a, b),
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
            meta_numbers: crate::manifest::RawNumbers::default(),
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

    /// A row whose `meta` is `text`, read as a manifest reads it.
    fn with_meta_text(row: ManifestRow, text: &str) -> Res<ManifestRow> {
        let (meta, meta_numbers) = crate::manifest::RawNumbers::parse(text)?;
        Ok(ManifestRow {
            meta: Some(meta),
            meta_numbers,
            ..row
        })
    }

    /// Two integers beyond 64 bits read as one float, but they are two values.
    #[test]
    fn entry_meta_with_different_big_integers_is_a_change() -> Res {
        let base = rows(vec![with_meta_text(
            row("a.csv", b"1")?,
            r#"{"n": 18446744073709551616}"#,
        )?]);
        let ours = rows(vec![with_meta_text(
            row("a.csv", b"1")?,
            r#"{"n": 18446744073709551617}"#,
        )?]);
        let theirs = base.clone();

        let merged = merge_onto(&base, &ours, theirs).expect("merged");

        assert_eq!(merged.rows, ours.rows);
        assert_eq!(
            merged.rows[0].meta_numbers, ours.rows[0].meta_numbers,
            "the merged row writes ours' digits"
        );
        Ok(())
    }

    #[test]
    fn package_meta_with_different_big_integers_is_a_change() -> Res {
        let with = |text: &str| -> Res<Manifest> {
            let (user_meta, user_meta_numbers) = crate::manifest::RawNumbers::parse(text)?;
            let mut manifest = Manifest::default();
            manifest.header.user_meta = Some(user_meta);
            manifest.header.user_meta_numbers = user_meta_numbers;
            Ok(manifest)
        };
        let base = with(r#"{"n": 18446744073709551616}"#)?;
        let ours = with(r#"{"n": 18446744073709551617}"#)?;

        let merged = merge_onto(&base, &ours, base.clone()).expect("merged");
        assert_eq!(
            merged.header.user_meta_numbers,
            ours.header.user_meta_numbers
        );

        let theirs = with(r#"{"n": 18446744073709551618}"#)?;
        assert_eq!(
            merge_onto(&base, &ours, theirs),
            Err(MergeRefusal::UserMeta),
            "both sides changed the same number to different digits"
        );
        Ok(())
    }

    /// A number kept from the base, once the value has been edited away from
    /// it, is not a difference: both sides write `{"n":3}`.
    #[test]
    fn package_meta_that_wrote_the_same_is_not_a_change_for_a_number_left_behind() -> Res {
        let with = |meta: serde_json::Value, numbers: crate::manifest::RawNumbers| {
            let mut manifest = Manifest::default();
            manifest.header.user_meta = Some(meta);
            manifest.header.user_meta_numbers = numbers;
            manifest
        };
        let (_, left_behind) =
            crate::manifest::RawNumbers::parse(r#"{"n": 18446744073709551616}"#)?;
        assert!(!left_behind.is_empty());
        let base = with(
            serde_json::json!({"n": 1}),
            crate::manifest::RawNumbers::default(),
        );
        let ours = with(serde_json::json!({"n": 3}), left_behind);
        let theirs = with(
            serde_json::json!({"n": 3}),
            crate::manifest::RawNumbers::default(),
        );

        assert!(merge_onto(&base, &ours, theirs).is_ok());
        Ok(())
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

    fn dir(key: &str, meta: serde_json::Value) -> Res<DirMeta> {
        Ok(DirMeta {
            logical_key: LogicalKey::new(key)?,
            meta,
        })
    }

    fn dirs(dirs: Vec<DirMeta>) -> Manifest {
        Manifest {
            dir_meta: dirs,
            ..Manifest::default()
        }
    }

    #[test]
    fn folder_metadata_changed_on_one_side_lands() -> Res {
        let base = dirs(vec![
            dir("a", json!({"v": 1}))?,
            dir("gone", json!({"v": 1}))?,
        ]);
        let ours = dirs(vec![
            dir("a", json!({"v": 2}))?,
            dir("new", json!({"v": 1}))?,
        ]);
        let theirs = dirs(vec![
            dir("a", json!({"v": 1}))?,
            dir("b", json!({"v": 3}))?,
            dir("gone", json!({"v": 1}))?,
        ]);

        let merged = merge_onto(&base, &ours, theirs).expect("merged");

        assert_eq!(
            merged.dir_meta,
            [
                dir("a", json!({"v": 2}))?,
                dir("b", json!({"v": 3}))?,
                dir("new", json!({"v": 1}))?,
            ]
        );
        Ok(())
    }

    #[test]
    fn folder_metadata_changed_on_both_sides_refuses_with_the_folder() -> Res {
        let base = dirs(vec![dir("a", json!({"v": 1}))?]);
        let ours = dirs(vec![dir("a", json!({"v": 2}))?]);
        let theirs = dirs(vec![dir("a", json!({"v": 3}))?]);

        assert_eq!(merge_onto(&base, &ours, theirs), Err(paths(&["a/"])));
        Ok(())
    }

    #[test]
    fn folder_metadata_that_says_nothing_is_none() -> Res {
        let base = dirs(vec![dir("a", json!({}))?]);
        let ours = dirs(vec![dir("a", json!({"v": 2}))?]);
        let theirs = Manifest::default();

        let merged = merge_onto(&base, &ours, theirs).expect("merged");

        assert_eq!(merged.dir_meta, [dir("a", json!({"v": 2}))?]);
        Ok(())
    }

    /// A file of one side where the other has folder metadata would make a
    /// manifest no reader accepts.
    #[test]
    fn a_file_where_the_other_side_has_folder_metadata_refuses() -> Res {
        let ours = rows(vec![row("a", b"mine")?]);
        let theirs = dirs(vec![dir("a", json!({"v": 1}))?]);

        assert_eq!(
            merge_onto(&Manifest::default(), &ours, theirs),
            Err(paths(&["a", "a/"]))
        );
        Ok(())
    }

    /// A tree error other than a file/folder clash must not be ignored.
    #[test]
    fn merge_dir_meta_returns_a_duplicate_row_as_an_error() -> Res {
        let rows = vec![row("a.csv", b"1")?, row("a.csv", b"2")?];
        let err = merge_dir_meta(&[], &[], &[], &rows)
            .expect_err("a duplicate file key is a tree error, not a merge refusal");
        assert!(
            matches!(
                err,
                MergeDirMetaError::Invalid(Error::Manifest(ManifestError::DuplicateLogicalKey(ref key)))
                    if key == "a.csv"
            ),
            "{err:?}"
        );
        Ok(())
    }

    /// A file that is the folder of a metadata row is reported with the
    /// folder's trailing `/`.
    #[test]
    fn a_file_that_is_the_folder_of_metadata_is_named_with_a_slash() -> Res {
        let ours = rows(vec![row("a", b"mine")?]);
        let theirs = dirs(vec![dir("a/sub", json!({"v": 1}))?]);

        assert_eq!(
            merge_onto(&Manifest::default(), &ours, theirs),
            Err(paths(&["a", "a/sub/"]))
        );
        Ok(())
    }
}
