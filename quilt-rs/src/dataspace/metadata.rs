//! `package-metadata.json`, a package's metadata as a file in its working
//! folder: a map from `"."` to the package's `user_meta`, and from each
//! logical key under the entry's paths to that entry's `user_meta`.

use serde_json::Map;
use serde_json::Value;

use crate::dataspace::contract::Paths;
use crate::manifest::Manifest;
use crate::manifest::RawNumbers;
use crate::manifest::Style;
use crate::manifest::escape_pointer;
use crate::manifest::write_json;

/// The metadata file's name, at the root of a working folder. Quilt owns it,
/// so a package holding a file of this name is refused.
pub const METADATA_FILE: &str = "package-metadata.json";

const PACKAGE_KEY: &str = ".";

/// Whether a row or a folder's metadata is at the metadata file's place or
/// inside a folder there, compared ignoring case and reading `\` as a
/// separator, as a working folder's disk may.
pub(crate) fn holds_metadata_file(manifest: &Manifest) -> bool {
    let rows = manifest.rows.iter().map(|row| &row.logical_key);
    let folders = manifest.dir_meta.iter().map(|dir| &dir.logical_key);
    rows.chain(folders).any(|key| {
        key.to_str()
            .and_then(|key| key.split(['/', '\\']).next())
            .is_some_and(|first| first.eq_ignore_ascii_case(METADATA_FILE))
    })
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
