//! `metadata.toml`, everything in a package revision that is not a file, as a
//! file in its working folder: the manifest's `version`, `message` and
//! `workflow`, the package's metadata, and that of each file and folder under
//! the entry's paths, a folder's key ending in `/`.
//!
//! The string `"$NULL"` is JSON `null` at any depth, with no escape, so a JSON
//! string `"$NULL"` reads back as `null`.

use std::collections::BTreeMap;

use serde::Deserialize;
use serde::de::IgnoredAny;
use serde_json::Map;
use serde_json::Value;
use toml::Table;
use tracing::warn;

use crate::LogicalKey;
use crate::dataspace::contract::Paths;
use crate::manifest::DirMeta;
use crate::manifest::HeaderMessage;
use crate::manifest::Manifest;
use crate::manifest::RawNumbers;

/// The metadata file's name, at the root of a working folder. Quilt owns it,
/// so a package holding a file of this name is refused.
pub const METADATA_FILE: &str = "metadata.toml";

const NULL: &str = "$NULL";

/// The one manifest format version quilt-rs writes.
const VERSION: &str = "v0";

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

/// The metadata file's text for `manifest`, holding the files and folders
/// `paths` cover that have a `user_meta`. The same revision is the same text
/// whatever order its manifest held keys in. quilt-rs reads no system
/// metadata from a manifest, so none is written.
pub(crate) fn render(manifest: &Manifest, paths: &Paths) -> Result<String, String> {
    let header = &manifest.header;
    let rows = manifest
        .rows
        .iter()
        .filter(|row| paths.covers(&row.logical_key))
        .map(|row| {
            (
                row.logical_key.to_string(),
                row.meta.as_ref(),
                &row.meta_numbers,
            )
        });
    let folders = manifest
        .dir_meta
        .iter()
        .filter(|dir| paths.covers_folder(&dir.logical_key))
        .map(|dir| {
            (
                format!("{}/", dir.logical_key),
                Some(&dir.meta),
                &dir.meta_numbers,
            )
        });
    let mut entries = Table::new();
    for (key, meta, numbers) in rows.chain(folders) {
        if let Some(user_meta) = meta.and_then(|meta| meta.get("user_meta")) {
            let value = checked_toml(user_meta, &numbers.moved("/user_meta", ""), &key)?;
            entries.insert(key, value);
        }
    }
    let mut metadata = Table::new();
    if !entries.is_empty() {
        metadata.insert("entries".to_string(), toml::Value::Table(entries));
    }
    if let Some(user_meta) = &header.user_meta {
        let value = checked_toml(user_meta, &header.user_meta_numbers, "the package")?;
        metadata.insert("package".to_string(), value);
    }

    let mut document = Table::new();
    match &header.message {
        HeaderMessage::Missing => {}
        HeaderMessage::Null => {
            document.insert("message".to_string(), NULL.into());
        }
        HeaderMessage::Text(text) => {
            document.insert("message".to_string(), text.as_str().into());
        }
    }
    if !metadata.is_empty() {
        document.insert("metadata".to_string(), toml::Value::Table(metadata));
    }
    document.insert("version".to_string(), header.version.as_str().into());
    if let Some(workflow) = &header.workflow {
        let stamp = serde_json::to_value(workflow).map_err(|err| err.to_string())?;
        let value = checked_toml(&stamp, &RawNumbers::default(), "the workflow")?;
        document.insert("workflow".to_string(), value);
    }
    toml::to_string(&document).map_err(|err| err.to_string())
}

/// `value` as TOML, refused where it holds an integer beyond TOML's signed 64
/// bits: one kept in `numbers`, or one above `i64::MAX`.
fn checked_toml(value: &Value, numbers: &RawNumbers, owner: &str) -> Result<toml::Value, String> {
    if numbers.still_in(value).is_empty()
        && let Some(value) = to_toml(value)
    {
        return Ok(value);
    }
    Err(format!(
        "the metadata of {owner} holds an integer beyond TOML's 64 bits"
    ))
}

/// `value` as TOML, `null` as [`NULL`].
fn to_toml(value: &Value) -> Option<toml::Value> {
    Some(match value {
        Value::Null => NULL.into(),
        Value::Bool(flag) => toml::Value::Boolean(*flag),
        Value::Number(number) => match number.as_i64() {
            Some(int) => toml::Value::Integer(int),
            None if number.is_f64() => toml::Value::Float(number.as_f64()?),
            None => return None,
        },
        Value::String(text) => text.as_str().into(),
        Value::Array(items) => {
            toml::Value::Array(items.iter().map(to_toml).collect::<Option<_>>()?)
        }
        Value::Object(fields) => toml::Value::Table(
            fields
                .iter()
                .map(|(key, item)| Some((key.clone(), to_toml(item)?)))
                .collect::<Option<_>>()?,
        ),
    })
}

/// `value` as JSON, [`NULL`] as `null`. A float that is not a number or not
/// finite, or a date or time, has no JSON form.
fn to_json(value: toml::Value) -> Result<Value, String> {
    Ok(match value {
        toml::Value::String(text) if text == NULL => Value::Null,
        toml::Value::String(text) => Value::String(text),
        toml::Value::Integer(int) => Value::from(int),
        toml::Value::Float(float) => serde_json::Number::from_f64(float)
            .map(Value::Number)
            .ok_or_else(|| format!("{float} is not a JSON number"))?,
        toml::Value::Boolean(flag) => Value::Bool(flag),
        toml::Value::Datetime(at) => {
            return Err(format!(
                "{at} is a TOML date or time, which JSON has not; quote it"
            ));
        }
        toml::Value::Array(items) => {
            Value::Array(items.into_iter().map(to_json).collect::<Result<_, _>>()?)
        }
        toml::Value::Table(fields) => Value::Object(
            fields
                .into_iter()
                .map(|(key, item)| Ok((key, to_json(item)?)))
                .collect::<Result<Map<_, _>, String>>()?,
        ),
    })
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawFile {
    version: Option<String>,
    message: Option<String>,
    #[serde(default)]
    metadata: RawMetadata,
    #[serde(rename = "workflow")]
    _workflow: Option<IgnoredAny>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct RawMetadata {
    package: Option<toml::Value>,
    #[serde(default)]
    entries: BTreeMap<String, toml::Value>,
    #[serde(rename = "system")]
    _system: Option<IgnoredAny>,
}

/// What the metadata file in a working folder says at publish.
pub(crate) enum MetadataFile {
    /// No file: every metadata is kept.
    Missing,
    /// The revision's message, the package's metadata and its entries', by
    /// key; what is absent is cleared.
    Supplied {
        message: HeaderMessage,
        package: Option<Value>,
        entries: BTreeMap<String, Value>,
    },
}

impl MetadataFile {
    /// Reads the file's text, or refuses one that is not the metadata file.
    /// Its `workflow` and `metadata.system` are not read.
    pub(crate) fn parse(text: Option<&str>) -> Result<Self, String> {
        let Some(text) = text else {
            return Ok(Self::Missing);
        };
        let RawFile {
            version,
            message,
            metadata,
            ..
        } = toml::from_str(text).map_err(|err| err.to_string())?;
        if let Some(version) = version.filter(|version| version != VERSION) {
            return Err(format!(
                "`version` is {version:?}; only \"{VERSION}\" is written"
            ));
        }
        let message = match message {
            None => HeaderMessage::Missing,
            Some(text) if text == NULL => HeaderMessage::Null,
            Some(text) => HeaderMessage::Text(text),
        };
        let package = metadata
            .package
            .map(|value| to_json(value).map_err(|err| format!("`metadata.package`: {err}")))
            .transpose()?;
        let entries = metadata
            .entries
            .into_iter()
            .map(|(key, value)| {
                let value = to_json(value).map_err(|err| format!("`{key}`: {err}"))?;
                Ok((key, value))
            })
            .collect::<Result<_, String>>()?;
        Ok(Self::Supplied {
            message,
            package,
            entries,
        })
    }

    /// The message the file gives the revision; `None` where there is no file.
    pub(crate) fn message(&self) -> Option<&HeaderMessage> {
        match self {
            Self::Missing => None,
            Self::Supplied { message, .. } => Some(message),
        }
    }

    /// Sets `manifest`'s metadata from the file: the package's, and that of
    /// each file and folder `paths` cover, which loses its metadata where the
    /// file leaves it out.
    pub(crate) fn apply(&self, manifest: &mut Manifest, paths: &Paths) {
        let Self::Supplied {
            package, entries, ..
        } = self
        else {
            return;
        };
        manifest.header.user_meta.clone_from(package);
        manifest.header.user_meta_numbers = RawNumbers::default();
        let mut pending: BTreeMap<&str, &Value> = entries
            .iter()
            .map(|(key, value)| (key.as_str(), value))
            .collect();
        for row in manifest
            .rows
            .iter_mut()
            .filter(|row| paths.covers(&row.logical_key))
        {
            let value = pending.remove(row.logical_key.to_string().as_str());
            row.meta = with_user_meta(row.meta.take(), value);
            row.meta_numbers = row.meta_numbers.without("/user_meta");
        }
        manifest.dir_meta.retain_mut(|dir| {
            if !paths.covers_folder(&dir.logical_key) {
                return true;
            }
            let value = pending.remove(format!("{}/", dir.logical_key).as_str());
            if value.is_none() && dir.meta.get("user_meta").is_none() {
                return true;
            }
            dir.meta_numbers = dir.meta_numbers.without("/user_meta");
            match with_user_meta(Some(std::mem::take(&mut dir.meta)), value) {
                Some(meta) => {
                    dir.meta = meta;
                    true
                }
                None => false,
            }
        });
        // A folder exists only while it holds a file.
        for (key, value) in pending {
            let Some(folder) = key
                .strip_suffix('/')
                .and_then(|folder| LogicalKey::new(folder).ok())
                .filter(|folder| paths.covers_folder(folder))
                .filter(|folder| {
                    manifest
                        .rows
                        .iter()
                        .any(|row| folder.is_folder_of(&row.logical_key))
                })
            else {
                warn!(
                    "{METADATA_FILE} names {key}, which is no file or folder under the entry's paths; it was left out"
                );
                continue;
            };
            let meta = with_user_meta(None, Some(value)).unwrap_or_default();
            manifest.dir_meta.push(DirMeta::new(folder, meta));
        }
    }
}

/// `meta` with its `user_meta` replaced, or removed with `None`, keeping the
/// rest; `None` where nothing is left. A `meta` that is not an object holds no
/// `user_meta` to remove, so `None` leaves it as it is.
fn with_user_meta(meta: Option<Value>, value: Option<&Value>) -> Option<Value> {
    let mut fields = match (meta, value) {
        (Some(Value::Object(fields)), _) => fields,
        (meta, None) => return meta,
        (_, Some(_)) => Map::new(),
    };
    match value {
        Some(value) => {
            fields.insert("user_meta".to_string(), value.clone());
        }
        None => {
            fields.remove("user_meta");
        }
    }
    (!fields.is_empty()).then_some(Value::Object(fields))
}

#[cfg(test)]
mod tests;
