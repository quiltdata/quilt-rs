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

/// An edit is undone only while the file holds what it wrote; a newer save
/// is left alone.
#[test(tokio::test)]
async fn an_edit_is_undone_only_while_the_file_is_as_written() -> Res {
    let storage = MockStorage::default();
    let text = "registry = \"s3://b\"\n";
    let mut edit = open(&storage, text).await?;
    let path = storage.temp_dir.path().join("rnaseq").join(CONTRACT_FILE);
    edit.add(Access::Sync, package("team/notes"));
    edit.write(&storage).await?;

    edit.restore(&storage).await?;
    assert_eq!(std::fs::read_to_string(&path)?, text);

    edit.write(&storage).await?;
    std::fs::write(&path, "registry = \"s3://other\"\n")?;
    assert!(edit.restore(&storage).await.is_err());
    assert_eq!(
        std::fs::read_to_string(&path)?,
        "registry = \"s3://other\"\n"
    );
    Ok(())
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

/// Declarations put in place of others take the first one's place in its
/// list, on lines of their own where the list is written one a line, and
/// the comments stay.
#[test(tokio::test)]
async fn redeclared_packages_keep_their_place() -> Res {
    let storage = MockStorage::default();
    let text = "registry = \"s3://b\"\n[packages]\nsync = [\n  \"team/first\", # ours\n  \"team/raw\",\n  \"team/last\",\n]\n";
    let mut edit = open(&storage, text).await?;
    let mut one = package("team/raw");
    one.insert("path", "a/".into());
    let mut two = package("team/raw");
    two.insert("path", "b/".into());

    edit.redeclare(&[(Access::Sync, 1)], Access::Sync, vec![one, two]);

    assert_eq!(
        edit.text(),
        "registry = \"s3://b\"\n[packages]\nsync = [\n  \"team/first\", # ours\n  { package = \"team/raw\", path = \"a/\" },\n  { package = \"team/raw\", path = \"b/\" },\n  \"team/last\",\n]\n"
    );
    Ok(())
}

/// Declarations moved to another list go at its end, a list written on one
/// line staying on one line, and in an array of tables they go in as
/// tables where the old ones were.
#[test(tokio::test)]
async fn redeclared_packages_move_between_lists() -> Res {
    let storage = MockStorage::default();
    let text = "registry = \"s3://b\"\n[packages]\nread = [\"team/a\", \"team/raw\"]\nsync = [\"team/b\"]\n";
    let mut edit = open(&storage, text).await?;

    edit.redeclare(
        &[(Access::Read, 1)],
        Access::Sync,
        vec![package("team/raw")],
    );

    assert_eq!(
        edit.text(),
        "registry = \"s3://b\"\n[packages]\nread = [\"team/a\"]\nsync = [\"team/b\", { package = \"team/raw\" }]\n"
    );

    let text = "registry = \"s3://b\"\n\n[[packages.read]]\npackage = \"team/a\"\n\n[[packages.read]]\npackage = \"team/raw\"\n\n[[packages.read]]\npackage = \"team/z\"\n";
    let mut edit = open(&storage, text).await?;
    let mut pinned = package("team/raw");
    pinned.insert("revision", "abcdef1234".into());

    edit.redeclare(&[(Access::Read, 1)], Access::Read, vec![pinned]);

    let packages: Vec<String> = edit
        .edited(None)?
        .entries()
        .iter()
        .map(|entry| format!("{} {:?}", entry.package, entry.revision))
        .collect();
    assert_eq!(packages.len(), 3);
    assert!(
        edit.text()
            .contains("[[packages.read]]\npackage = \"team/raw\"\nrevision = \"abcdef1234\"\n\n[[packages.read]]\npackage = \"team/z\""),
        "{}",
        edit.text()
    );
    Ok(())
}
