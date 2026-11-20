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
use crate::dataspace::contract::FolderCase;
use crate::dataspace::contract::dataspace_path;
use crate::dataspace::contract::folder_case;
use crate::io::storage::Storage;

/// `data.toml` being edited: its text as read, and as edited so far.
pub(crate) struct ContractEdit {
    dataspace: PathBuf,
    case: FolderCase,
    original: Vec<u8>,
    doc: DocumentMut,
}

impl ContractEdit {
    /// `data.toml` in the folder `dataspace`. Where there is none, the edit
    /// starts from an empty contract, and the folder is made if missing.
    pub(crate) async fn open(
        storage: &(impl Storage + Sync),
        dataspace: &Path,
    ) -> Result<Self, ContractError> {
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
            doc,
        })
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
                let mut value = Value::InlineTable(declaration);
                // A list written one declaration a line takes the new one on
                // its own line too.
                if let Some(prefix) = array
                    .iter()
                    .last()
                    .and_then(|last| last.decor().prefix())
                    .cloned()
                {
                    value.decor_mut().set_prefix(prefix);
                }
                array.push_formatted(value);
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

    /// Writes the edited file over `data.toml` by renaming a temporary file
    /// over it.
    pub(crate) async fn write(&self, storage: &(impl Storage + Sync)) -> Result<(), ContractError> {
        let path = self.dataspace.join(CONTRACT_FILE);
        let temp = format!("{CONTRACT_FILE}.tmp-{}", uuid::Uuid::new_v4());
        storage
            .write_byte_stream_via(
                &path,
                temp,
                ByteStream::from(self.text().into_bytes()),
                None,
            )
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

#[cfg(test)]
mod tests;
