//! `package-metadata.json`, a package's metadata as a file in its working
//! folder: a map from `"."` to the package's `user_meta`, and from each
//! logical key under the entry's paths to that entry's `user_meta`.

use serde_json::Map;
use serde_json::Value;
use tracing::warn;

use crate::dataspace::contract::Paths;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::manifest::RawNumbers;
use crate::manifest::Style;
use crate::manifest::escape_pointer;
use crate::manifest::write_json;

/// The metadata file's name, at the root of a working folder. Quilt owns it,
/// so a package holding a file of this name is refused.
pub const METADATA_FILE: &str = "package-metadata.json";

const PACKAGE_KEY: &str = ".";

/// Whether a row or a folder's metadata is [at the metadata file's
/// place](at_metadata_file).
pub(crate) fn holds_metadata_file(manifest: &Manifest) -> bool {
    let rows = manifest.rows.iter().map(|row| &row.logical_key);
    let folders = manifest.dir_meta.iter().map(|dir| &dir.logical_key);
    rows.chain(folders)
        .any(|key| key.to_str().is_some_and(at_metadata_file))
}

/// Whether `path` is at the metadata file's place or inside a folder there,
/// compared ignoring case and reading `\` as a separator, as a working
/// folder's disk may.
pub(crate) fn at_metadata_file(path: &str) -> bool {
    path.split(['/', '\\'])
        .next()
        .is_some_and(|first| first.eq_ignore_ascii_case(METADATA_FILE))
}

/// The metadata file's text for `manifest`, holding the entries `paths`
/// cover that have a `user_meta`. Keys are sorted at every level, so the same
/// metadata is the same text whatever order a manifest held it in. Integers
/// beyond 64 bits keep their digits.
pub(crate) fn render(manifest: &Manifest, paths: &Paths) -> String {
    let mut map = Map::new();
    let mut numbers = RawNumbers::default();
    if let Some(user_meta) = &manifest.header.user_meta {
        map.insert(PACKAGE_KEY.to_string(), sorted(user_meta));
        numbers = manifest
            .header
            .user_meta_numbers
            .under(&format!("/{PACKAGE_KEY}"));
    }
    for row in &manifest.rows {
        let Some(user_meta) = row.meta.as_ref().and_then(|meta| meta.get("user_meta")) else {
            continue;
        };
        if !paths.covers(&row.logical_key) {
            continue;
        }
        let key = row.logical_key.to_string();
        numbers = numbers.union(
            &row.meta_numbers
                .moved("/user_meta", &format!("/{}", escape_pointer(&key))),
        );
        map.insert(key, sorted(user_meta));
    }
    map.sort_keys();
    let mut text = String::new();
    write_json(&mut text, &Value::Object(map), &numbers, Style::Manifest);
    text.push('\n');
    text
}

fn sorted(value: &Value) -> Value {
    match value {
        Value::Object(map) => {
            let mut keys: Map<String, Value> = map
                .iter()
                .map(|(key, item)| (key.clone(), sorted(item)))
                .collect();
            keys.sort_keys();
            Value::Object(keys)
        }
        Value::Array(items) => Value::Array(items.iter().map(sorted).collect()),
        other => other.clone(),
    }
}

/// What the metadata file in a working folder says at publish.
pub(crate) enum MetadataFile {
    /// No file: every metadata is kept.
    Missing,
    /// An empty file: the metadata it covers is cleared.
    Empty,
    /// A map, with its integers beyond 64 bits.
    Map(Map<String, Value>, RawNumbers),
}

impl MetadataFile {
    /// Reads the file's text, or refuses one that is not a JSON object.
    pub(crate) fn parse(text: Option<&str>) -> Result<Self, String> {
        let Some(text) = text else {
            return Ok(Self::Missing);
        };
        if text.trim().is_empty() {
            return Ok(Self::Empty);
        }
        match RawNumbers::parse(text).map_err(|err| err.to_string())? {
            (Value::Object(map), numbers) => Ok(Self::Map(map, numbers)),
            _ => Err("it is not a JSON object".to_string()),
        }
    }

    /// Sets `manifest`'s metadata from the file: the package's under `"."`,
    /// and each row's that `paths` cover under its key. What the file leaves
    /// out is cleared where `whole` and kept otherwise; an empty file clears
    /// all it covers.
    pub(crate) fn apply(&self, manifest: &mut Manifest, paths: &Paths, whole: bool) {
        let (map, numbers) = match self {
            Self::Missing => return,
            Self::Empty => (&Map::new(), &RawNumbers::default()),
            Self::Map(map, numbers) => (map, numbers),
        };
        let clear_absent = whole || matches!(self, Self::Empty);
        let header = &mut manifest.header;
        match map.get(PACKAGE_KEY) {
            Some(value) => {
                header.set_meta(value.clone(), numbers.moved(&format!("/{PACKAGE_KEY}"), ""));
            }
            None if clear_absent => {
                header.user_meta = None;
                header.user_meta_numbers = RawNumbers::default();
            }
            None => {}
        }
        let mut applied = 0;
        for row in manifest
            .rows
            .iter_mut()
            .filter(|row| paths.covers(&row.logical_key))
        {
            let key = row.logical_key.to_string();
            match map.get(&key) {
                Some(value) => {
                    let pointer = format!("/{}", escape_pointer(&key));
                    set_user_meta(row, Some(value), &numbers.moved(&pointer, "/user_meta"));
                    applied += 1;
                }
                None if clear_absent => set_user_meta(row, None, &RawNumbers::default()),
                None => {}
            }
        }
        let package = usize::from(map.contains_key(PACKAGE_KEY));
        if map.len() > applied + package {
            warn!(
                "{METADATA_FILE} names keys that are not files under the entry's paths; they were left out"
            );
        }
    }
}

/// Replaces `row`'s `user_meta`, or removes it with `None`, keeping the rest
/// of its `meta`. A `meta` that is not an object holds no `user_meta` to
/// remove, so `None` leaves it as it is.
fn set_user_meta(row: &mut ManifestRow, value: Option<&Value>, numbers: &RawNumbers) {
    if value.is_none() && !matches!(row.meta, Some(Value::Object(_))) {
        return;
    }
    let mut meta = match row.meta.take() {
        Some(Value::Object(meta)) => meta,
        _ => Map::new(),
    };
    match value {
        Some(value) => {
            meta.insert("user_meta".to_string(), value.clone());
        }
        None => {
            meta.remove("user_meta");
        }
    }
    row.meta_numbers = row.meta_numbers.without("/user_meta").union(numbers);
    row.meta = (!meta.is_empty()).then_some(Value::Object(meta));
}
