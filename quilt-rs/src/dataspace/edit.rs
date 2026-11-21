//! Edits to `data.toml` that a command makes because the user asked for them
//! (25.3): the only writes a tool makes to the contract. Each edit keeps the
//! file's comments and layout, and the edited file is read as a whole
//! contract before it is written, so a contract is never written that
//! [`Contract::read`] would refuse.

use std::path::Path;
use std::path::PathBuf;

use aws_sdk_s3::primitives::ByteStream;
use toml_edit::DocumentMut;
use toml_edit::InlineTable;
use toml_edit::Item;
use toml_edit::Value;

use crate::dataspace::contract::Access;
use crate::dataspace::contract::CONTRACT_FILE;
use crate::dataspace::contract::Contract;
use crate::dataspace::contract::ContractError;
use crate::dataspace::contract::ContractFile;
use crate::dataspace::contract::Entry;
use crate::dataspace::contract::EntryProblem;
use crate::dataspace::contract::FolderCase;
use crate::dataspace::contract::PackagePath;
use crate::dataspace::contract::dataspace_path;
use crate::dataspace::contract::folder_case;
use crate::dataspace::install::spelled;
use crate::io::storage::Storage;
use quilt_uri::RevisionPointer;
use quilt_uri::Tag;

/// `data.toml` being edited: its text as read, and as edited so far.
pub(crate) struct ContractEdit {
    dataspace: PathBuf,
    case: FolderCase,
    original: Vec<u8>,
    /// Whether there was a `data.toml` to read.
    found: bool,
    doc: DocumentMut,
}

impl ContractEdit {
    /// `data.toml` in the folder `dataspace`. Where there is none, the edit
    /// starts from an empty contract, and the folder is made if missing.
    pub(crate) async fn open(
        storage: &(impl Storage + Sync),
        dataspace: &Path,
    ) -> Result<Self, ContractError> {
        let mut found = true;
        let file = match ContractFile::read(storage, dataspace).await {
            Ok(file) => file,
            Err(ContractError::Read { source, .. }) if source.is_not_found() => {
                let root = dataspace_path(dataspace).map_err(|err| ContractError::Read {
                    path: dataspace.to_path_buf(),
                    source: Box::new(err.into()),
                })?;
                let case =
                    folder_case(storage, &root)
                        .await
                        .map_err(|source| ContractError::Read {
                            path: root.clone(),
                            source: Box::new(source),
                        })?;
                found = false;
                ContractFile {
                    dataspace: root,
                    bytes: Vec::new(),
                    case,
                }
            }
            Err(err) => return Err(err),
        };
        let text =
            std::str::from_utf8(&file.bytes).map_err(|err| ContractError::Edit(err.to_string()))?;
        let doc = text
            .parse::<DocumentMut>()
            .map_err(|err| ContractError::Edit(err.to_string()))?;
        Ok(Self {
            dataspace: file.dataspace,
            case: file.case,
            original: file.bytes,
            found,
            doc,
        })
    }

    /// Whether the dataspace had a `data.toml` when the edit opened it.
    pub(crate) fn found(&self) -> bool {
        self.found
    }

    /// The dataspace's folder, as [`Contract::dataspace`] names it.
    pub(crate) fn dataspace(&self) -> &Path {
        &self.dataspace
    }

    /// The contract as it was read, before any edit.
    pub(crate) fn original(&self, local_registry: Option<&str>) -> Result<Contract, ContractError> {
        Contract::parse(&self.original, &self.dataspace, self.case, local_registry)
    }

    /// The contract the edits so far make.
    pub(crate) fn edited(&self, local_registry: Option<&str>) -> Result<Contract, ContractError> {
        Contract::parse(
            &self.text().into_bytes(),
            &self.dataspace,
            self.case,
            local_registry,
        )
    }

    /// Whether any edit changed the file.
    pub(crate) fn changed(&self) -> bool {
        self.text().as_bytes() != self.original.as_slice()
    }

    /// Appends `declaration` to the list of `access`, spelled as the list's
    /// others are: an inline table in an array, or a table in an array of
    /// tables. The `[packages]` table and the list are made if missing.
    pub(crate) fn add(&mut self, access: Access, declaration: InlineTable) {
        let list = access.to_string();
        let packages = self
            .doc
            .as_table_mut()
            .entry("packages")
            .or_insert_with(toml_edit::table);
        let Some(packages) = packages.as_table_like_mut() else {
            return;
        };
        match packages.get_mut(&list) {
            Some(Item::ArrayOfTables(tables)) => tables.push(declaration.into_table()),
            Some(Item::Value(Value::Array(array))) => {
                // A list written one declaration a line takes the new one on
                // its own line too.
                match line_prefix(array.iter().last()) {
                    Some(prefix) => {
                        let mut value = Value::InlineTable(declaration);
                        value.decor_mut().set_prefix(prefix);
                        array.push_formatted(value);
                    }
                    None => array.push(declaration),
                }
            }
            _ => {
                let mut array = toml_edit::Array::new();
                array.push(declaration);
                packages.insert(&list, Item::Value(Value::Array(array)));
            }
        }
    }

    /// Removes the declarations at `places`, each a list and an index in it
    /// as [`Contract::declared`] gives them.
    pub(crate) fn remove(&mut self, places: &[(Access, usize)]) {
        let mut places = places.to_vec();
        // From the end of each list, so each index still names its own.
        places.sort_unstable_by(|one, other| other.cmp(one));
        let Some(packages) = self
            .doc
            .as_table_mut()
            .get_mut("packages")
            .and_then(Item::as_table_like_mut)
        else {
            return;
        };
        for (access, index) in places {
            match packages.get_mut(&access.to_string()) {
                Some(Item::ArrayOfTables(tables)) if index < tables.len() => {
                    tables.remove(index);
                }
                Some(Item::Value(Value::Array(array))) if index < array.len() => {
                    let removed = array.remove(index);
                    // The next declaration takes the removed one's place on
                    // the line.
                    if let (Some(prefix), Some(next)) =
                        (removed.decor().prefix().cloned(), array.get_mut(index))
                    {
                        next.decor_mut().set_prefix(prefix);
                    }
                }
                _ => {}
            }
        }
    }

    /// Replaces the declarations at `places`, as [`ContractEdit::remove`]
    /// takes them, with `declarations` in the list of `access`: where the
    /// first of `places` in that list was, so a package keeps its place, or
    /// at the list's end where none was in it. What keep changes, a
    /// package's paths, pin or access, is made this way.
    pub(crate) fn redeclare(
        &mut self,
        places: &[(Access, usize)],
        access: Access,
        declarations: Vec<InlineTable>,
    ) {
        let first = places
            .iter()
            .filter(|(list, _)| *list == access)
            .map(|(_, index)| *index)
            .min();
        // The first is replaced where it is, with what comes before it on
        // its line; no other removed is before it in its list.
        let others: Vec<(Access, usize)> = places
            .iter()
            .filter(|place| Some(**place) != first.map(|first| (access, first)))
            .copied()
            .collect();
        self.remove(&others);
        let mut declarations = declarations.into_iter();
        if let Some(first) = first {
            self.replace(access, first, &mut declarations);
        }
        for declaration in declarations {
            self.add(access, declaration);
        }
    }

    /// Replaces the declaration at `at` in the list of `access` with
    /// `declarations`, the first taking its place and the rest following it,
    /// spelled as [`ContractEdit::add`] spells them.
    fn replace(
        &mut self,
        access: Access,
        at: usize,
        declarations: &mut impl Iterator<Item = InlineTable>,
    ) {
        let Some(list) = self
            .doc
            .as_table_mut()
            .get_mut("packages")
            .and_then(Item::as_table_like_mut)
            .and_then(|packages| packages.get_mut(&access.to_string()))
        else {
            return;
        };
        match list {
            Item::ArrayOfTables(tables) if at < tables.len() => {
                let mut rebuilt = toml_edit::ArrayOfTables::new();
                for (index, table) in tables.iter().enumerate() {
                    if index != at {
                        rebuilt.push(table.clone());
                        continue;
                    }
                    let mut taken = declarations.by_ref().map(InlineTable::into_table);
                    if let Some(mut first) = taken.next() {
                        // The comments above the table stay above it.
                        *first.decor_mut() = table.decor().clone();
                        rebuilt.push(first);
                    }
                    for table in taken {
                        rebuilt.push(table);
                    }
                }
                *tables = rebuilt;
            }
            Item::Value(Value::Array(array)) if at < array.len() => {
                let line = line_prefix(array.get(at));
                let Some(old) = array.get(at) else {
                    return;
                };
                let decor = old.decor().clone();
                for (index, declaration) in (at..).zip(declarations.by_ref()) {
                    let mut value = Value::InlineTable(declaration);
                    if index == at {
                        *value.decor_mut() = decor.clone();
                        array.replace_formatted(index, value);
                        continue;
                    }
                    match &line {
                        Some(line) => {
                            value.decor_mut().set_prefix(line.as_str());
                            array.insert_formatted(index, value);
                        }
                        None => array.insert(index, value),
                    }
                }
            }
            _ => {}
        }
    }

    /// Writes the edited file over `data.toml` by renaming a temporary file
    /// over it.
    pub(crate) async fn write(&self, storage: &(impl Storage + Sync)) -> Result<(), ContractError> {
        self.write_bytes(storage, self.text().into_bytes()).await
    }

    /// Puts the file back as it was when opened, or removes it where there
    /// was none: undoes [`Self::write`], and only while the file still holds
    /// what it wrote.
    pub(crate) async fn restore(
        &self,
        storage: &(impl Storage + Sync),
    ) -> Result<(), ContractError> {
        let written = ContractFile::read(storage, &self.dataspace)
            .await
            .is_ok_and(|file| file.bytes == self.text().into_bytes());
        if !written {
            return Err(ContractError::Edit(format!(
                "{CONTRACT_FILE} changed since this edit wrote it"
            )));
        }
        if self.found {
            return self.write_bytes(storage, self.original.clone()).await;
        }
        let path = self.dataspace.join(CONTRACT_FILE);
        storage
            .remove_file(&path)
            .await
            .map_err(|source| ContractError::Write {
                path,
                source: Box::new(crate::Error::from(source)),
            })
    }

    async fn write_bytes(
        &self,
        storage: &(impl Storage + Sync),
        bytes: Vec<u8>,
    ) -> Result<(), ContractError> {
        let path = self.dataspace.join(CONTRACT_FILE);
        let temp = format!("{CONTRACT_FILE}.tmp-{}", uuid::Uuid::new_v4());
        storage
            .write_byte_stream_via(&path, temp, ByteStream::from(bytes), None)
            .await
            .map_err(|source| ContractError::Write {
                path,
                source: Box::new(source),
            })
    }

    fn text(&self) -> String {
        self.doc.to_string()
    }
}

/// What starts `value`'s line in a list written one declaration a line: its
/// prefix from the last line break, without a comment before it. `None`
/// where the prefix holds no line break.
fn line_prefix(value: Option<&Value>) -> Option<String> {
    let prefix = value?.decor().prefix()?.as_str()?;
    prefix.rfind('\n').map(|at| prefix[at..].to_string())
}

/// One declaration of `entry` in `contract`, covering `path`, or the whole
/// package for `None`: its registry where it is not the contract's default,
/// its catalog where it is not the default, its package, its revision where
/// it is pinned, the path, and its folder where the package would not be
/// placed there. It expands to `entry` with that path.
pub(crate) fn declaration(
    contract: &Contract,
    entry: &Entry,
    path: Option<&PackagePath>,
) -> Result<InlineTable, EntryProblem> {
    let key = entry.key();
    let stated = contract.default_registry() != Some(&key.registry);
    let mut table = InlineTable::new();
    if stated {
        table.insert("registry", key.registry.spelled().into());
    }
    if let Some(catalog) = &entry.catalog
        && contract.default_catalog() != Some(catalog)
    {
        table.insert("catalog", catalog.to_string().into());
    }
    table.insert("package", key.package.to_string().into());
    if let Some(revision) = pin(&entry.revision) {
        table.insert("revision", revision.into());
    }
    if let Some(path) = path {
        table.insert("path", path.spelled().into());
    }
    let placed = contract.placed(stated, &key)?;
    if !contract.same_folder(&placed, &entry.folder) {
        let folder = entry
            .folder
            .strip_prefix(contract.dataspace())
            .unwrap_or(&entry.folder);
        table.insert("folder", spelled(folder).into());
    }
    Ok(table)
}

/// A revision as a contract's `revision` field spells it: a hash, or a tag
/// after a colon; `None` for `latest`, which needs no field.
pub(crate) fn pin(revision: &RevisionPointer) -> Option<String> {
    match revision {
        RevisionPointer::Tag(Tag::Latest) => None,
        RevisionPointer::Hash(hash) | RevisionPointer::HashPrefix(hash) => Some(hash.clone()),
        RevisionPointer::Tag(tag) => Some(format!(":{tag}")),
    }
}

#[cfg(test)]
mod tests;
