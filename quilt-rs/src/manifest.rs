//!
//! Namespace contains helpers to work with manifest and its content (rows).

mod meta_json;
mod top_hasher;

pub use meta_json::RawNumbers;
pub use top_hasher::TopHasher;
pub(crate) use top_hasher::same_entry_meta;

pub use crate::workflow::Workflow;
pub use crate::workflow::WorkflowId;

use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;
use serde::Deserializer;
use serde::Serialize;
use tokio::io::AsyncBufReadExt;
use tokio::io::AsyncRead;
use tokio::io::BufReader;

use crate::Error;
use crate::LogicalKey;
use crate::Res;
use crate::error::ManifestError;
use crate::io::manifest::RowsStream;
use crate::io::manifest::StreamRowsChunk;
use crate::io::storage::ByteStream;
use crate::manifest::meta_json::Style;
use crate::manifest::meta_json::write_json;
use crate::object_hash;

/// Header (or first row) in JSONL manifest
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq)]
pub struct ManifestHeader {
    pub version: String,
    #[serde(default, skip_serializing_if = "HeaderMessage::is_missing")]
    pub message: HeaderMessage,
    #[serde(skip_serializing_if = "Option::is_none")] // Attempt to be quilt3-compatible.
    pub user_meta: Option<serde_json::Value>,
    /// The integers in `user_meta` beyond 64 bits, as read; see [`RawNumbers`].
    /// Not serialized, so serde output such as the CLI's JSON has them as
    /// floats; compared by `PartialEq`.
    #[serde(skip)]
    pub user_meta_numbers: RawNumbers,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub workflow: Option<Workflow>,
}

impl ManifestHeader {
    /// Whether two headers write the same `user_meta`: the text the top hash
    /// reads, so integers beyond 64 bits count by their digits, `-0.0` is not
    /// `0.0`, and a kept number the value has left behind counts for nothing.
    #[must_use]
    pub fn same_user_meta(&self, other: &Self) -> bool {
        self.user_meta_text() == other.user_meta_text()
    }

    pub(crate) fn user_meta_text(&self) -> Option<String> {
        self.user_meta.as_ref().map(|meta| {
            let mut text = String::new();
            write_json(&mut text, meta, &self.user_meta_numbers, Style::Hash);
            text
        })
    }
}

/// A header's `message`. quilt3 hashes the header as read, so a missing key,
/// `null` and a string each give a different top hash.
#[derive(Debug, Clone, Default, PartialEq)]
pub enum HeaderMessage {
    /// The header has no `message` key.
    #[default]
    Missing,
    /// `"message": null`
    Null,
    /// `"message": "<text>"`
    Text(String),
}

impl HeaderMessage {
    #[must_use]
    pub fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }

    /// The message text; `None` when it is missing or `null`.
    #[must_use]
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(text) => Some(text),
            Self::Missing | Self::Null => None,
        }
    }
}

/// A missing key never reaches this: the field skips it on write.
impl Serialize for HeaderMessage {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.as_str().serialize(serializer)
    }
}

/// A missing key never reaches this: `#[serde(default)]` gives `Missing`.
impl<'de> Deserialize<'de> for HeaderMessage {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Ok(Option::<String>::deserialize(deserializer)?.map_or(Self::Null, Self::Text))
    }
}

impl ManifestHeader {
    /// The commit message, if the header has one.
    #[must_use]
    pub fn message(&self) -> Option<&str> {
        self.message.as_str()
    }
}

impl Default for ManifestHeader {
    fn default() -> Self {
        ManifestHeader {
            version: "v0".to_string(),
            message: HeaderMessage::Text(String::new()),
            user_meta: Some(serde_json::Value::Object(serde_json::Map::new())),
            user_meta_numbers: RawNumbers::default(),
            workflow: None,
        }
    }
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
struct Quilt3ManifestRow {
    pub logical_key: LogicalKey,
    pub physical_keys: Vec<String>,
    pub hash: object_hash::ObjectHash,
    // XXX: u64 cannot be safely deserialized by standard JS json parser,
    //      which treats numbers as 64-bit floats.
    //      However, having file size more than ~9PB (max safe/lossless integer - 53 bits)
    //      is quite unlikely ATM, given S3 limitations of 5TB per object.
    #[serde(deserialize_with = "number_to_u64")]
    pub size: u64,
    pub meta: Option<serde_json::Value>,
}

fn number_to_u64<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u64, D::Error> {
    // See note on `Quilt3ManifestRow::size` — JSON numbers come through as f64;
    // sizes within S3's 5TB cap fit losslessly in the f64 mantissa.
    #[allow(clippy::cast_possible_truncation, clippy::cast_sign_loss)]
    f64::deserialize(deserializer).map(|n| n as u64)
}

/// A file's manifest line. Hash and size are parsed here, so a folder row
/// (no physical keys) can ignore them without this type being involved.
#[derive(Deserialize)]
struct Quilt3FileLine {
    logical_key: PathBuf,
    physical_keys: Vec<String>,
    hash: object_hash::ObjectHash,
    #[serde(deserialize_with = "number_to_u64")]
    size: u64,
    #[serde(default)]
    meta: Option<serde_json::Value>,
}

/// A folder's manifest line. `hash` and `size` are ignored, as quilt3 ignores
/// them, including a malformed hash such as `{"type":"nope"}`.
#[derive(Deserialize)]
struct Quilt3FolderLine {
    logical_key: PathBuf,
    #[serde(default)]
    physical_keys: Option<Vec<String>>,
    #[serde(default)]
    meta: Option<serde_json::Value>,
}

enum Line {
    File(Box<ManifestRow>),
    Dir(DirMeta),
}

fn folder_line(logical_key: &Path, meta: Option<serde_json::Value>, line: &str) -> Res<Line> {
    let Some(folder) = logical_key
        .to_str()
        .and_then(|key| key.strip_suffix('/'))
        .map(str::to_owned)
    else {
        return Err(ManifestError::Header("Physical key is missing".to_string()).into());
    };
    let mut meta = meta.unwrap_or(serde_json::Value::Null);
    let meta_numbers = RawNumbers::read(&mut meta, line, "meta");
    Ok(Line::Dir(DirMeta {
        logical_key: LogicalKey::new(folder)?,
        meta,
        meta_numbers,
    }))
}

impl Line {
    /// A file row is parsed first, with a typed hash, so a manifest of only
    /// files is as cheap to read as it was before folder rows were accepted.
    /// If that fails, or the keys are empty, the line is read as a folder and
    /// hash and size are ignored — unless it has physical keys, in which case
    /// the file-row error is kept.
    fn from_jsonl(line: &str) -> Res<Self> {
        match serde_json::from_str::<Quilt3FileLine>(line) {
            Ok(file) if !file.physical_keys.is_empty() => {
                let mut row = ManifestRow::try_from(Quilt3ManifestRow {
                    logical_key: LogicalKey::new(file.logical_key)?,
                    physical_keys: file.physical_keys,
                    hash: file.hash,
                    size: file.size,
                    meta: file.meta,
                })?;
                if let Some(meta) = row.meta.as_mut() {
                    row.meta_numbers = RawNumbers::read(meta, line, "meta");
                }
                Ok(Self::File(Box::new(row)))
            }
            Ok(file) => folder_line(&file.logical_key, file.meta, line),
            Err(file_err) => {
                let folder: Quilt3FolderLine = match serde_json::from_str(line) {
                    Ok(folder) => folder,
                    Err(_) => return Err(file_err.into()),
                };
                if folder
                    .physical_keys
                    .as_ref()
                    .is_some_and(|keys| !keys.is_empty())
                {
                    return Err(file_err.into());
                }
                folder_line(&folder.logical_key, folder.meta, line)
            }
        }
    }
}

impl From<ManifestRow> for Quilt3ManifestRow {
    fn from(row: ManifestRow) -> Self {
        Self {
            logical_key: row.logical_key,
            physical_keys: vec![row.physical_key],
            hash: row.hash,
            size: row.size,
            meta: Some(row.meta.unwrap_or_else(|| serde_json::json!({}))),
        }
    }
}

/// Represents the row in JSONL manifest
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq)]
#[cfg_attr(any(test, feature = "testing"), derive(Default))]
pub struct ManifestRow {
    pub logical_key: LogicalKey,
    // XXX: use Url to have validated string?
    pub physical_key: String,
    pub hash: object_hash::ObjectHash,
    pub size: u64,
    pub meta: Option<serde_json::Value>,
    /// The integers in `meta` beyond 64 bits, as read; see [`RawNumbers`].
    /// Not serialized, so serde output such as the CLI's JSON has them as
    /// floats; compared by `PartialEq`.
    #[serde(skip)]
    pub meta_numbers: RawNumbers,
}

impl ManifestRow {
    /// Content-identity check: two rows describe the same logical file with
    /// the same bytes. Ignores `physical_key` (same content is addressed as
    /// `file:///.../objects/<hash>` locally and `s3://...` remotely, and the
    /// push flow uses this to reuse the remote location instead of
    /// re-uploading) and `meta` (user metadata that does not change the
    /// stored bytes, so a metadata-only edit should not force a re-upload).
    #[must_use]
    pub fn matches_content(&self, other: &Self) -> bool {
        self.logical_key == other.logical_key && self.hash == other.hash && self.size == other.size
    }
}

impl ManifestRow {
    /// Whether two rows write the same `meta`: the text the top hash reads,
    /// so integers beyond 64 bits count by their digits and `-0.0` is not
    /// `0.0`, and absent, empty and false `meta` are one.
    #[must_use]
    pub fn same_meta(&self, other: &Self) -> bool {
        top_hasher::same_entry_meta(self, other)
    }
}

impl TryFrom<Quilt3ManifestRow> for ManifestRow {
    type Error = Error;

    fn try_from(row: Quilt3ManifestRow) -> Result<Self, Self::Error> {
        Ok(ManifestRow {
            logical_key: row.logical_key,
            physical_key: row
                .physical_keys
                .into_iter()
                .next()
                .ok_or(ManifestError::Header("Physical key is missing".to_string()))?,
            hash: row.hash,
            size: row.size,
            meta: row.meta,
            meta_numbers: RawNumbers::default(),
        })
    }
}

/// The metadata of a folder: a `{"logical_key": "dir/", "meta": …}` row, which
/// has no physical key.
///
/// Like quilt3, a manifest keeps it where it is read, writes only metadata that
/// says something (not `null`, `{}`, `0`, `""` or `false`), and leaves it out of
/// the top hash. Whole numbers beyond 64 bits are kept as the text they were
/// read with, and `-0` is `0`, the same as row `meta` and header `user_meta`.
#[derive(Clone, Debug, Deserialize, Eq, Serialize, PartialEq)]
pub struct DirMeta {
    /// The folder, without the trailing `/` its manifest row has.
    pub logical_key: LogicalKey,
    pub meta: serde_json::Value,
    /// The integers in `meta` beyond 64 bits, as read; see [`RawNumbers`].
    /// Not serialized, so serde output such as the CLI's JSON has them as
    /// floats; compared by `PartialEq`.
    #[serde(skip)]
    pub meta_numbers: RawNumbers,
}

impl DirMeta {
    /// A folder's metadata built by hand, with no kept number text. A manifest
    /// that is read fills [`Self::meta_numbers`] from the line.
    #[must_use]
    pub fn new(logical_key: LogicalKey, meta: serde_json::Value) -> Self {
        Self {
            logical_key,
            meta,
            meta_numbers: RawNumbers::default(),
        }
    }

    /// Whether two folders write the same `meta`: the text
    /// [`ManifestRow::same_meta`] uses, so integers beyond 64 bits count by
    /// their digits and a kept number the value has left behind counts for
    /// nothing.
    #[must_use]
    pub fn same_meta(&self, other: &Self) -> bool {
        self.meta_text() == other.meta_text()
    }

    fn meta_text(&self) -> String {
        let mut text = String::new();
        write_json(&mut text, &self.meta, &self.meta_numbers, Style::Hash);
        text
    }

    /// Whether the metadata says anything, as Python's `bool` sees it: quilt3
    /// writes a folder row only when it does.
    #[must_use]
    pub fn holds_meta(&self) -> bool {
        match &self.meta {
            serde_json::Value::Null => false,
            serde_json::Value::Bool(flag) => *flag,
            serde_json::Value::Number(number) => number.as_f64() != Some(0.0),
            serde_json::Value::String(text) => !text.is_empty(),
            serde_json::Value::Array(items) => !items.is_empty(),
            serde_json::Value::Object(fields) => !fields.is_empty(),
        }
    }
}

/// A package is a tree: no key twice, and no key that is also a folder. A
/// folder's metadata row is a folder, so it can't share a name with a file or
/// sit under one, and a folder has one such row at most.
///
/// In key order (see [`LogicalKey`]), a key that is also a folder comes right
/// before one of the keys inside it, and a repeated key right before its twin,
/// so comparing neighbours is enough.
pub(crate) fn check_tree(rows: &[ManifestRow], dir_meta: &[DirMeta]) -> Res {
    let mut keys: Vec<(&LogicalKey, bool)> = rows
        .iter()
        .map(|row| (&row.logical_key, false))
        .chain(dir_meta.iter().map(|dir| (&dir.logical_key, true)))
        .collect();
    keys.sort_unstable();
    for pair in keys.windows(2) {
        let ((first, first_is_dir), (next, next_is_dir)) = (pair[0], pair[1]);
        let named = |key: &LogicalKey, is_dir| {
            if is_dir {
                format!("{key}/")
            } else {
                key.to_string()
            }
        };
        if first == next {
            let error = if first_is_dir == next_is_dir {
                ManifestError::DuplicateLogicalKey(named(first, first_is_dir))
            } else {
                ManifestError::FileAndFolder(first.to_string())
            };
            return Err(Error::Manifest(error));
        }
        if !first_is_dir && first.is_folder_of(next) {
            return Err(Error::Manifest(ManifestError::FileAndFolder(
                first.to_string(),
            )));
        }
    }
    Ok(())
}

/// One row as a manifest line, in quilt3's form, field for field as
/// [`Quilt3ManifestRow`] writes it, with the metadata as quilt3 writes it.
fn write_row(buf: &mut String, row: &ManifestRow) -> Res {
    buf.push_str("{\"logical_key\":");
    buf.push_str(&serde_json::to_string(&row.logical_key)?);
    buf.push_str(",\"physical_keys\":[");
    buf.push_str(&serde_json::to_string(&row.physical_key)?);
    buf.push_str("],\"hash\":");
    buf.push_str(&serde_json::to_string(&row.hash)?);
    buf.push_str(",\"size\":");
    buf.push_str(&row.size.to_string());
    buf.push_str(",\"meta\":");
    let meta = row.meta.clone().unwrap_or_else(|| serde_json::json!({}));
    write_json(buf, &meta, &row.meta_numbers, Style::Manifest);
    buf.push('}');
    Ok(())
}

/// One folder-metadata row as a manifest line, with the metadata as quilt3 writes it.
fn write_dir_row(buf: &mut String, dir: &DirMeta) -> Res {
    buf.push_str("{\"logical_key\":");
    buf.push_str(&serde_json::to_string(&format!("{}/", dir.logical_key))?);
    buf.push_str(",\"meta\":");
    write_json(buf, &dir.meta, &dir.meta_numbers, Style::Manifest);
    buf.push('}');
    Ok(())
}

/// Legacy JSONL in-memory manifest
#[derive(Debug, Deserialize, Serialize, Clone, PartialEq, Default)]
pub struct Manifest {
    pub header: ManifestHeader,
    pub rows: Vec<ManifestRow>,
    /// The metadata of folders, which quilt3 keeps beside the rows; see [`DirMeta`].
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub dir_meta: Vec<DirMeta>,
    // XXX: iterators?
}

impl Manifest {
    pub async fn from_reader<F: AsyncRead + Unpin + Send>(file: F) -> Res<Self> {
        let reader = BufReader::new(file);
        let mut lines = reader.lines();

        let header = lines.next_line().await.map_err(|err| {
            Error::Manifest(ManifestError::Header(format!(
                "Failed to read the manifest header: {err}"
            )))
        })?;

        let Some(header_str) = header else {
            return Err(Error::Manifest(ManifestError::Header(
                "Empty manifest".into(),
            )));
        };

        // Parse the raw JSON to check if user_meta is explicitly null
        let raw_value: serde_json::Value = serde_json::from_str(&header_str)?;

        // Parse the header normally
        let mut header: ManifestHeader = serde_json::from_str(&header_str)?;

        // Handle user_meta field based on the raw JSON
        if let Some(user_meta) = raw_value.get("user_meta")
            && user_meta.is_null()
        {
            header.user_meta = Some(serde_json::Value::Null);
        }
        if let Some(user_meta) = header.user_meta.as_mut() {
            header.user_meta_numbers = RawNumbers::read(user_meta, &header_str, "user_meta");
        }

        if header.version != "v0" {
            return Err(Error::Manifest(ManifestError::Header(format!(
                "Unsupported manifest version: {}",
                header.version
            ))));
        }

        let mut rows = Vec::new();
        let mut dir_meta = Vec::new();
        while let Some(line) = lines.next_line().await? {
            match Line::from_jsonl(&line)? {
                Line::File(row) => rows.push(*row),
                Line::Dir(dir) => dir_meta.push(dir),
            }
        }

        check_tree(&rows, &dir_meta)?;
        Ok(Manifest {
            header,
            rows,
            dir_meta,
        })
    }

    /// Serialize the manifest to JSONL: the header line followed by one line
    /// per row.
    ///
    /// # Panics
    ///
    /// Panics if [`Self::try_to_jsonlines`] would return an error. That is a
    /// programmer error: a well-formed [`Manifest`] always serializes. Prefer
    /// [`Self::try_to_jsonlines`] in library paths that may be handed a
    /// hand-built value.
    #[must_use]
    pub fn to_jsonlines(&self) -> String {
        self.try_to_jsonlines().expect(
            "Could not serialize manifest: a Manifest must form a tree and its header and rows must serialize",
        )
    }

    /// Serialize the manifest to JSONL: the header line followed by one line
    /// per row, folders in key order before the files, and only folder
    /// metadata that says something.
    ///
    /// # Errors
    ///
    /// The rows and folder metadata do not form a tree (a duplicate key, or a
    /// name that is both a file and a folder), or the header or a row could
    /// not be serialized.
    pub fn try_to_jsonlines(&self) -> Res<String> {
        // In quilt3's order: the folders by name, before the files.
        check_tree(&self.rows, &self.dir_meta)?;
        let mut dirs: Vec<&DirMeta> = self
            .dir_meta
            .iter()
            .filter(|dir| dir.holds_meta())
            .collect();
        dirs.sort_by(|a, b| a.logical_key.cmp(&b.logical_key));

        let mut buf = String::new();
        let header = serde_json::to_value(&self.header)?;
        write_json(
            &mut buf,
            &header,
            &self.header.user_meta_numbers.under("/user_meta"),
            Style::Manifest,
        );
        buf.push('\n');
        for dir in dirs {
            write_dir_row(&mut buf, dir)?;
            buf.push('\n');
        }

        for row in &self.rows {
            write_row(&mut buf, row)?;
            buf.push('\n');
        }
        Ok(buf)
    }

    /// Read manifest from a file path, converting from Table format if needed
    pub async fn from_path(
        storage: &impl crate::io::storage::Storage,
        path: &std::path::Path,
    ) -> Res<Self> {
        let file = storage.open_file(path).await?;
        Self::from_reader(file).await.map_err(|e| {
            crate::Error::Manifest(ManifestError::Load {
                path: path.to_path_buf(),
                source: Box::new(e),
            })
        })
    }

    /// Find a record by path (for compatibility with Table API)
    #[must_use]
    pub fn get_record(&self, path: &Path) -> Option<&ManifestRow> {
        self.rows
            .iter()
            .find(|row| row.logical_key.as_path() == path)
    }

    /// Check if manifest contains a record for the given path
    #[must_use]
    pub fn contains_record(&self, path: &Path) -> bool {
        self.rows
            .iter()
            .any(|row| row.logical_key.as_path() == path)
    }

    /// Create a stream of rows compatible with Table API
    /// Returns a stream of Row chunks for compatibility with `io::manifest` streaming functions
    /// Sorted by `logical_key` to match `Table`'s `BTreeMap` behavior and uses proper `TryFrom` conversion
    #[allow(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "kept async for the streaming, Table-compatible manifest API; a streamed backend will await here"
    )]
    pub async fn records_stream(&self) -> impl RowsStream {
        // Sort by logical_key to match Table's BTreeMap ordering
        let mut indices: Vec<usize> = (0..self.rows.len()).collect();
        indices.sort_by(|&a, &b| self.rows[a].logical_key.cmp(&self.rows[b].logical_key));

        let rows: StreamRowsChunk = indices
            .into_iter()
            .map(|i| Ok(self.rows[i].clone()))
            .collect();
        tokio_stream::iter(vec![Ok(rows)])
    }

    /// Get the number of records in the manifest
    #[allow(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "kept async for the streaming, Table-compatible manifest API; a streamed backend will await here"
    )]
    pub async fn records_len(&self) -> usize {
        self.rows.len()
    }

    /// Insert a record into the manifest (for compatibility with Table API)
    #[allow(
        clippy::unused_async,
        clippy::unused_async_trait_impl,
        reason = "kept async for the streaming, Table-compatible manifest API; a streamed backend will await here"
    )]
    pub async fn insert_record(&mut self, row: ManifestRow) -> Res<Option<ManifestRow>> {
        // Check if row already exists
        let existing_pos = self
            .rows
            .iter()
            .position(|r| r.logical_key == row.logical_key);

        if let Some(pos) = existing_pos {
            Ok(Some(std::mem::replace(&mut self.rows[pos], row)))
        } else {
            self.rows.push(row);
            Ok(None)
        }
    }
}

impl From<&Manifest> for ByteStream {
    fn from(manifest: &Manifest) -> Self {
        // Prefer `try_to_jsonlines` in library paths that may see a hand-built
        // Manifest; this conversion is for a value already known to be valid.
        manifest.to_jsonlines().into_bytes().into()
    }
}

#[cfg(test)]
mod quilt3_fixtures_tests;

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use std::path::PathBuf;

    use crate::fixtures;
    use aws_sdk_s3::primitives::ByteStream;

    use crate::io::storage::LocalStorage;
    use crate::io::storage::Storage;
    use crate::io::storage::mocks::MockStorage;

    #[test]
    fn test_matches_content_identical_rows() -> Res {
        let left = ManifestRow {
            logical_key: crate::LogicalKey::new("A")?,
            physical_key: "B".to_string(),
            hash: object_hash::Sha256Hash::try_from("deadbeef")?.into(),
            size: 1,
            meta: None,
            meta_numbers: crate::manifest::RawNumbers::default(),
        };
        let right = ManifestRow {
            logical_key: crate::LogicalKey::new("A")?,
            physical_key: "B".to_string(),
            hash: object_hash::Sha256Hash::try_from("deadbeef")?.into(),
            size: 1,
            meta: None,
            meta_numbers: crate::manifest::RawNumbers::default(),
        };
        assert!(left.matches_content(&right));
        Ok(())
    }

    #[test]
    fn test_matches_content_ignores_physical_key_and_meta() -> Res {
        let mut meta = serde_json::Map::new();
        meta.insert("foo".to_string(), serde_json::json!("bar"));
        let left = ManifestRow {
            logical_key: crate::LogicalKey::new("A")?,
            physical_key: "FOO".to_string(),
            hash: object_hash::Sha256Hash::try_from("deadbeef")?.into(),
            size: 1,
            meta: Some(serde_json::Value::Object(meta)),
            meta_numbers: crate::manifest::RawNumbers::default(),
        };
        let right = ManifestRow {
            logical_key: crate::LogicalKey::new("A")?,
            physical_key: "BAR".to_string(),
            hash: object_hash::Sha256Hash::try_from("deadbeef")?.into(),
            size: 1,
            meta: None,
            meta_numbers: crate::manifest::RawNumbers::default(),
        };
        assert!(left.matches_content(&right));
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_manifest_from_reader_invalid() -> Res {
        let storage = MockStorage::default();
        let invalid_content = r#"{"invalid": "json"}"#;
        let path = PathBuf::from("invalid_manifest.jsonl");
        storage
            .write_byte_stream(&path, ByteStream::from_static(invalid_content.as_bytes()))
            .await?;
        let file = storage.open_file(&path).await?;

        let result = Manifest::from_reader(file).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing field `version`")
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_manifest_from_reader_empty() -> Res {
        let storage = MockStorage::default();
        let path = PathBuf::from("empty_manifest.jsonl");
        storage
            .write_byte_stream(&path, ByteStream::default())
            .await?;
        let file = storage.open_file(&path).await?;

        let result = Manifest::from_reader(file).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Manifest header: Empty manifest"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_manifest_from_reader_invalid_utf8() -> Res {
        let storage = MockStorage::default();
        let invalid_content = b"\xFF\xFF\xFF\xFF"; // Invalid UTF-8 bytes
        let path = PathBuf::from("invalid_utf8_manifest.jsonl");
        storage
            .write_byte_stream(&path, ByteStream::from_static(invalid_content))
            .await?;
        let file = storage.open_file(&path).await?;

        let result = Manifest::from_reader(file).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("Failed to read the manifest header")
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_manifest_from_reader_unsupported_version() -> Res {
        let storage = MockStorage::default();
        let invalid_content = r#"{"version": "v1"}"#;
        let path = PathBuf::from("unsupported_version_manifest.jsonl");
        storage
            .write_byte_stream(&path, ByteStream::from_static(invalid_content.as_bytes()))
            .await?;
        let file = storage.open_file(&path).await?;

        let result = Manifest::from_reader(file).await;
        if let Err(Error::Manifest(ManifestError::Header(error_string))) = result {
            assert_eq!(error_string, "Unsupported manifest version: v1");
        } else {
            panic!("Expected ManifestHeader error, got: {result:?}");
        }
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_manifest_from_reader_invalid_row() -> Res {
        let storage = MockStorage::default();
        let invalid_content = r#"{"version": "v0"}
{"invalid": "row"}"#;
        let path = PathBuf::from("invalid_row_manifest.jsonl");
        storage
            .write_byte_stream(&path, ByteStream::from_static(invalid_content.as_bytes()))
            .await?;
        let file = storage.open_file(&path).await?;

        let result = Manifest::from_reader(file).await;
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .to_string()
                .contains("missing field `logical_key`")
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_manifest_from_reader_empty_physical_keys() -> Res {
        let storage = MockStorage::default();
        let invalid_content = r#"{"version": "v0"}
{"logical_key": "test.txt", "physical_keys": [], "size": 0, "hash": {"type": "SHA256", "value": "abc123"}, "meta": {}}"#;
        let path = PathBuf::from("empty_physical_keys_manifest.jsonl");
        storage
            .write_byte_stream(&path, ByteStream::from_static(invalid_content.as_bytes()))
            .await?;
        let file = storage.open_file(&path).await?;

        let result = Manifest::from_reader(file).await;
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "Manifest header: Physical key is missing"
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_manifest_from_reader_valid() -> Res {
        let storage = LocalStorage::default();
        let file = storage.open_file(fixtures::manifest::path()?).await?;
        let checksummed_manifest = Manifest::from_reader(file).await?;

        assert_eq!(
            checksummed_manifest.header,
            ManifestHeader {
                message: HeaderMessage::Text("Initial".to_string()),
                user_meta: None,
                ..ManifestHeader::default()
            }
        );
        assert_eq!(
            checksummed_manifest.rows[0],
                    ManifestRow {
                        logical_key: crate::LogicalKey::new("e0-0.txt".to_string())?,
                        physical_key: "s3://data-yaml-spec-tests/scale/10u/e0-0.txt?versionId=jHb6DGN43Ex7EhbxZc2G9JnAkWSeTfEY".to_string(),
                        size: 29,
                        hash: object_hash::Sha256ChunkedHash::try_from("/UMjH1bsbrMLBKdd9cqGGvtjhWzawhz1BfrxgngUhVI=")?.into(),
                        meta: Some(serde_json::json!({})),
                        meta_numbers: crate::manifest::RawNumbers::default(),
                    }
        );
        assert_eq!(
            checksummed_manifest.rows[9],
                    ManifestRow {
                        logical_key: crate::LogicalKey::new("e0-9.txt".to_string())?,
                        physical_key: "s3://data-yaml-spec-tests/scale/10u/e0-9.txt?versionId=T5tkWkC.7PVcpiFYRoCQKhhKC249fdBC".to_string(),
                        size: 29,
                        hash: object_hash::Sha256ChunkedHash::try_from("/UMjH1bsbrMLBKdd9cqGGvtjhWzawhz1BfrxgngUhVI=")?.into(),
                        meta: Some(serde_json::json!({})),
                        meta_numbers: crate::manifest::RawNumbers::default(),
                    }
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_null_message_stays_null_and_a_missing_one_stays_missing() -> Res {
        for (line, message) in [
            (r#"{"version":"v0","message":null}"#, HeaderMessage::Null),
            (r#"{"version":"v0"}"#, HeaderMessage::Missing),
        ] {
            let manifest = Manifest::from_reader(format!("{line}\n").as_bytes()).await?;
            assert_eq!(manifest.header.message, message, "{line}");
            assert_eq!(manifest.to_jsonlines(), format!("{line}\n"));
        }
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_row_with_no_meta_reads_as_empty_and_is_written_with_meta() -> Res {
        let header = r#"{"version":"v0"}"#;
        let row = concat!(
            r#"{"logical_key":"test1.txt","physical_keys":["s3://bucket/test1.txt"],"#,
            r#""size":0,"hash":{"type":"sha2-256-chunked","value":"47DEQpj8HBSa+/TImW+5JCeuQeRkm5NMpJWZG3hSuFU="}}"#,
        );
        let manifest = Manifest::from_reader(format!("{header}\n{row}\n").as_bytes()).await?;
        assert_eq!(manifest.rows[0].meta, None);
        let written = manifest.to_jsonlines();
        assert!(
            written.contains(r#""meta":{}"#),
            "written row must include meta: {written}"
        );
        let again = Manifest::from_reader(written.as_bytes()).await?;
        assert_eq!(again.rows[0].meta, Some(serde_json::json!({})));
        Ok(())
    }

    #[test]
    fn test_manifest_header_default() -> Res {
        let header = ManifestHeader::default();
        assert_eq!(header.version, "v0");
        assert_eq!(header.message, HeaderMessage::Text(String::new()));
        assert_eq!(header.user_meta, Some(serde_json::json!({})));
        assert_eq!(header.workflow, None);
        Ok(())
    }

    /// The same value with other integers beyond 64 bits, or a `-0.0` for a
    /// `0.0`, writes other text, so it is a different `user_meta`; a number the
    /// value has left behind writes nothing, so it is not.
    #[test]
    fn same_user_meta_compares_what_is_written() -> Res {
        let header = |meta: serde_json::Value, numbers: RawNumbers| ManifestHeader {
            user_meta: Some(meta),
            user_meta_numbers: numbers,
            ..ManifestHeader::default()
        };
        let (a, a_numbers) = RawNumbers::parse(r#"{"n": 18446744073709551616}"#)?;
        let (b, b_numbers) = RawNumbers::parse(r#"{"n": 18446744073709551617}"#)?;
        let (a, b) = (header(a, a_numbers.clone()), header(b, b_numbers));
        assert_eq!(a.user_meta, b.user_meta);
        assert!(!a.same_user_meta(&b));
        assert!(a.same_user_meta(&a.clone()));

        let three = serde_json::json!({"n": 3});
        assert!(
            header(three.clone(), a_numbers).same_user_meta(&header(three, RawNumbers::default()))
        );

        let (zero, _) = RawNumbers::parse(r#"{"z": 0.0}"#)?;
        let (negative, _) = RawNumbers::parse(r#"{"z": -0.0}"#)?;
        assert_eq!(zero, negative, "serde_json cannot tell them apart");
        assert!(
            !header(zero, RawNumbers::default())
                .same_user_meta(&header(negative, RawNumbers::default()))
        );
        Ok(())
    }
}
