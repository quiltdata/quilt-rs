use super::*;

use test_log::test;

use crate::io::storage::mocks::MockStorage;

type Res = Result<(), Box<dyn std::error::Error>>;

async fn open(
    storage: &MockStorage,
    text: &str,
) -> Result<ContractEdit, Box<dyn std::error::Error>> {
    let root = storage.temp_dir.path().join("rnaseq");
    std::fs::create_dir_all(&root)?;
    std::fs::write(root.join(CONTRACT_FILE), text)?;
    Ok(ContractEdit::open(storage, &root).await?)
}

fn package(name: &str) -> InlineTable {
    let mut table = InlineTable::new();
    table.insert("package", name.into());
    table
}

/// A declaration added to a list written one a line goes on its own line,
/// and the comments stay.
#[test(tokio::test)]
async fn an_added_declaration_follows_the_lists_layout() -> Res {
    let storage = MockStorage::default();
    let text = "registry = \"s3://b\" # the lab's\n\n[packages]\nsync = [\n  \"team/raw\",\n]\n";
    let mut edit = open(&storage, text).await?;

    edit.add(Access::Sync, package("team/notes"));

    assert_eq!(
        edit.text(),
        "registry = \"s3://b\" # the lab's\n\n[packages]\nsync = [\n  \"team/raw\",\n  { package = \"team/notes\" },\n]\n"
    );
    assert_eq!(edit.edited(None)?.entries().len(), 2);
    Ok(())
}

/// Declarations in an array of tables are removed from it, and added to it
/// as tables.
#[test(tokio::test)]
async fn an_array_of_tables_is_edited_as_one() -> Res {
    let storage = MockStorage::default();
    let text = "registry = \"s3://b\"\n\n[[packages.read]]\npackage = \"team/raw\"\n\n[[packages.read]]\npackage = \"team/other\"\n";
    let mut edit = open(&storage, text).await?;

    edit.remove(&[(Access::Read, 0)]);
    edit.add(Access::Read, package("team/notes"));

    let contract = edit.edited(None)?;
    let packages: Vec<String> = contract
        .entries()
        .iter()
        .map(|entry| entry.package.to_string())
        .collect();
    assert_eq!(packages, ["team/notes", "team/other"]);
    assert!(
        edit.text()
            .contains("[[packages.read]]\npackage = \"team/notes\""),
        "{}",
        edit.text()
    );
    Ok(())
}

/// Removing the first of a list written one a line leaves the next on its
/// line, and nothing changes the file until it is written.
#[test(tokio::test)]
async fn a_removed_declaration_leaves_the_list_as_written() -> Res {
    let storage = MockStorage::default();
    let text = "[packages]\nread = [\n  \"quilt+s3://b#package=team/raw\",\n  \"quilt+s3://b#package=team/other\",\n]\n";
    let mut edit = open(&storage, text).await?;

    edit.remove(&[(Access::Read, 0)]);

    assert!(edit.changed());
    assert_eq!(
        edit.text(),
        "[packages]\nread = [\n  \"quilt+s3://b#package=team/other\",\n]\n"
    );
    let root = storage.temp_dir.path().join("rnaseq");
    assert_eq!(std::fs::read_to_string(root.join(CONTRACT_FILE))?, text);
    edit.write(&storage).await?;
    assert_eq!(
        std::fs::read_to_string(root.join(CONTRACT_FILE))?,
        edit.text()
    );
    Ok(())
}
