use super::*;

use test_log::test;

use crate::io::storage::mocks::MockStorage;

type Res = Result<(), Box<dyn std::error::Error>>;

const ROOT: &str = "/work/rnaseq";

const FULL_HASH: &str = "abc1230000000000000000000000000000000000000000000000000000000000";

fn parse_as(text: &str, case: FolderCase) -> Result<Contract, ContractError> {
    Contract::parse(text.as_bytes(), Path::new(ROOT), case)
}

fn parse(text: &str) -> Result<Contract, ContractError> {
    parse_as(text, FolderCase::Sensitive)
}

fn entries(text: &str) -> Result<Vec<Entry>, ContractError> {
    Ok(parse(text)?.entries().to_vec())
}

fn only(text: &str) -> Result<Entry, Box<dyn std::error::Error>> {
    let mut entries = entries(text)?;
    assert_eq!(entries.len(), 1, "{entries:?}");
    Ok(entries.remove(0))
}

fn package(name: &str) -> Namespace {
    Namespace::try_from(name).unwrap()
}

fn bucket(name: &str) -> Registry {
    Registry::S3(name.to_string())
}

fn folder(relative: &str) -> PathBuf {
    Path::new(ROOT).join(relative)
}

fn key(path: &str) -> LogicalKey {
    LogicalKey::new(path).unwrap()
}

fn entry_problem(text: &str) -> EntryProblem {
    match parse(text) {
        Err(ContractError::Entry { problem, .. }) => problem,
        other => panic!("expected an entry's problem, got {other:?}"),
    }
}

fn package_problem(text: &str) -> PackageProblem {
    match parse(text) {
        Err(ContractError::Package { problem, .. }) => problem,
        other => panic!("expected a package's problem, got {other:?}"),
    }
}

const DEFAULTS: &str = "registry = \"s3://quilt-dev\"\n";

fn with_defaults(packages: &str) -> String {
    format!("{DEFAULTS}[packages]\n{packages}")
}

#[test]
fn an_access_reads_back_from_its_list_name_and_nothing_else() -> Res {
    for (name, access) in [
        ("read", Access::Read),
        ("write", Access::Write),
        ("replace", Access::Replace),
        ("sync", Access::Sync),
    ] {
        assert_eq!(name.parse::<Access>()?, access);
        assert_eq!(access.to_string(), name);
    }
    for name in ["Read", "publish", ""] {
        assert!(name.parse::<Access>().is_err(), "{name:?}");
    }
    Ok(())
}

#[test]
fn three_spellings_of_one_package_expand_to_one_entry() -> Res {
    let expected = Entry {
        registry: bucket("quilt-dev"),
        catalog: None,
        package: package("team/raw"),
        revision: RevisionPointer::Tag(Tag::Latest),
        paths: Paths::Whole,
        folder: folder("team/raw"),
        access: Access::Read,
    };
    for list in [
        r#"read = ["team/raw"]"#,
        r#"read = [{ package = "team/raw" }]"#,
        r#"read = [{ uri = "quilt+s3://quilt-dev#package=team/raw", folder = "team/raw" }]"#,
    ] {
        assert_eq!(only(&with_defaults(list))?, expected, "{list}");
    }
    Ok(())
}

#[test]
fn a_plain_string_implies_its_package_from_its_last_two_segments() -> Res {
    let entry = only(&with_defaults(r#"read = ["data/team/raw"]"#))?;
    assert_eq!(entry.package, package("team/raw"));
    assert_eq!(entry.folder, folder("data/team/raw"));
    Ok(())
}

#[test]
fn a_short_folder_takes_its_owner_from_the_path_above_it() -> Res {
    let entry = only(&with_defaults(r#"sync = ["analysis"]"#))?;
    assert_eq!(entry.package, package("rnaseq/analysis"));
    assert_eq!(entry.folder, folder("analysis"));
    Ok(())
}

#[test]
fn an_implied_name_has_its_other_characters_replaced() -> Res {
    let entry = only(&with_defaults(r#"read = ["team/My Analysis (v2)"]"#))?;
    assert_eq!(entry.package, package("team/My_Analysis__v2_"));
    assert_eq!(entry.folder, folder("team/My Analysis (v2)"));
    Ok(())
}

#[test]
fn a_folder_that_runs_out_of_path_takes_the_default_prefix() -> Res {
    let contract = format!(
        "default_prefix = \"me\"\n{}",
        with_defaults(r#"read = ["/data"]"#)
    );
    assert_eq!(only(&contract)?.package, package("me/data"));
    assert!(matches!(
        entry_problem(&with_defaults(r#"read = ["/data"]"#)),
        EntryProblem::FolderTooShort
    ));
    Ok(())
}

#[test]
fn an_entry_that_states_its_bucket_is_placed_under_it() -> Res {
    let stated = only(&with_defaults(
        r#"read = [{ registry = "s3://quilt-dev", package = "team/raw" }]"#,
    ))?;
    assert_eq!(stated.folder, folder("quilt-dev/team/raw"));
    let uri = only(&with_defaults(
        r#"sync = ["quilt+s3://other-bucket#package=team/notes"]"#,
    ))?;
    assert_eq!(uri.registry, bucket("other-bucket"));
    assert_eq!(uri.folder, folder("other-bucket/team/notes"));
    Ok(())
}

#[test]
fn an_entry_naming_no_registry_needs_a_default() -> Res {
    for list in [
        r#"read = ["team/raw"]"#,
        r#"read = [{ package = "team/raw" }]"#,
    ] {
        let problem = entry_problem(&format!("[packages]\n{list}"));
        assert!(
            matches!(problem, EntryProblem::NoDefaultRegistry),
            "{list}: {problem}"
        );
    }
    let entry = only("[packages]\nread = [\"quilt+s3://quilt-dev#package=team/raw\"]")?;
    assert_eq!(entry.registry, bucket("quilt-dev"));
    Ok(())
}

#[test]
fn a_folder_overlapping_the_contract_or_the_lock_is_refused() {
    for list in [
        r#"read = ["."]"#,
        r#"read = ["./"]"#,
        r#"read = [".."]"#,
        r#"read = [{ package = "team/raw", folder = "." }]"#,
        r#"read = ["data.toml"]"#,
        r#"read = [{ package = "team/raw", folder = "data.toml/x" }]"#,
        r#"read = [{ package = "data.toml/x" }]"#,
        r#"read = ["data.lock.toml"]"#,
        r#"read = [{ package = "data.lock.toml/x" }]"#,
    ] {
        let problem = entry_problem(&with_defaults(list));
        assert!(
            matches!(problem, EntryProblem::DataspaceFolder),
            "{list}: {problem}"
        );
    }
}

#[test]
fn a_plain_string_in_a_local_registry_folder_is_refused() {
    assert!(matches!(
        entry_problem(&with_defaults(r#"read = ["host@local/me/analysis"]"#)),
        EntryProblem::LocalFolder
    ));
}

#[test]
fn overlapping_folders_are_refused() {
    for list in [
        r#"read = ["team/raw", "team/raw/inner"]"#,
        r#"read = [{ package = "a/b", folder = "x" }, { package = "c/d", folder = "x" }]"#,
    ] {
        assert!(
            matches!(
                parse(&with_defaults(list)),
                Err(ContractError::FoldersOverlap { .. })
            ),
            "{list}"
        );
    }
}

#[test]
fn folders_differing_in_case_overlap_where_names_ignore_case() {
    let contract = with_defaults(r#"read = ["Team/Raw", "team/raw"]"#);
    assert!(parse_as(&contract, FolderCase::Sensitive).is_ok());
    assert!(matches!(
        parse_as(&contract, FolderCase::Insensitive),
        Err(ContractError::FoldersOverlap { .. })
    ));
}

/// Whether two files whose names differ in case are one, counted by listing
/// the folder rather than by asking for a name.
fn ignores_case(dir: &Path) -> std::io::Result<bool> {
    let probe = dir.join("probe");
    std::fs::create_dir(&probe)?;
    std::fs::write(probe.join("a"), "")?;
    std::fs::write(probe.join("A"), "")?;
    Ok(std::fs::read_dir(&probe)?.count() == 1)
}

#[test(tokio::test)]
async fn read_compares_folders_as_the_dataspace_file_system_does() -> Res {
    let storage = MockStorage::default();
    let dataspace = storage.temp_dir.path().join("rnaseq");
    std::fs::create_dir(&dataspace)?;
    // A file of its own where names keep case, and the contract where they don't.
    std::fs::write(dataspace.join("DATA.TOML"), "")?;
    std::fs::write(
        dataspace.join(CONTRACT_FILE),
        with_defaults(r#"read = ["Team/Raw", "team/raw"]"#),
    )?;
    let read = Contract::read(&storage, &dataspace).await;
    if ignores_case(storage.temp_dir.path())? {
        assert!(matches!(read, Err(ContractError::FoldersOverlap { .. })));
    } else {
        let folders: Vec<PathBuf> = read?.entries().iter().map(|e| e.folder.clone()).collect();
        assert_eq!(
            folders,
            [dataspace.join("Team/Raw"), dataspace.join("team/raw")]
        );
    }
    Ok(())
}

#[test]
fn a_uri_and_parts_that_disagree_are_refused() -> Res {
    let pinned = format!("revision = \"{FULL_HASH}\"");
    for (field, extra) in [
        ("package", r#"package = "team/other""#),
        ("registry", r#"registry = "s3://other-bucket""#),
        ("revision", pinned.as_str()),
        ("path", r#"path = "src/""#),
        ("catalog", r#"catalog = "nightly.quilttest.com""#),
    ] {
        let list =
            format!(r#"read = [{{ uri = "quilt+s3://quilt-dev#package=team/raw", {extra} }}]"#);
        let problem = entry_problem(&with_defaults(&list));
        assert!(
            matches!(problem, EntryProblem::Disagrees(f) if f == field),
            "{field}: {problem}"
        );
    }
    let agreeing = only(&with_defaults(
        r#"read = [{ uri = "quilt+s3://quilt-dev#package=team/raw&path=src/", package = "team/raw", path = "src/" }]"#,
    ))?;
    assert_eq!(
        agreeing.paths,
        Paths::Only(vec![PackagePath::Folder(key("src"))])
    );
    Ok(())
}

#[test]
fn a_pin_is_allowed_under_read_only() -> Res {
    let read = only(&with_defaults(&format!(
        r#"read = [{{ package = "team/reference", revision = "{FULL_HASH}" }}]"#
    )))?;
    assert_eq!(read.revision, RevisionPointer::Hash(FULL_HASH.to_string()));
    for access in ["write", "replace", "sync"] {
        let problem = package_problem(&with_defaults(&format!(
            r#"{access} = [{{ package = "team/p", revision = "abc123" }}]"#
        )));
        assert!(
            matches!(problem, PackageProblem::PinOutsideRead(a) if a.to_string() == access),
            "{access}: {problem}"
        );
    }
    let shared = package_problem(&with_defaults(
        r#"read = [{ package = "team/p", path = "in/", revision = "abc123" }]
write = [{ package = "team/p", path = "out/" }]"#,
    ));
    assert!(matches!(
        shared,
        PackageProblem::PinOutsideRead(Access::Write)
    ));
    Ok(())
}

#[test]
fn replace_takes_no_path() {
    assert!(matches!(
        entry_problem(&with_defaults(
            r#"replace = [{ package = "team/snapshot", path = "src/" }]"#
        )),
        EntryProblem::PathUnderReplace
    ));
}

#[test]
fn a_package_under_sync_or_replace_is_listed_under_nothing_else() {
    for (exclusive, other) in [("sync", "read"), ("replace", "write"), ("sync", "replace")] {
        let problem = package_problem(&with_defaults(&format!(
            "{other} = [\"team/p\"]\n{exclusive} = [\"team/p\"]"
        )));
        assert!(
            matches!(problem, PackageProblem::ExclusiveAccess(..)),
            "{exclusive} and {other}: {problem}"
        );
    }
}

#[test]
fn a_package_may_be_read_and_written_at_paths_that_do_not_overlap() -> Res {
    let both = entries(&with_defaults(
        r#"read = [{ package = "team/p", path = "data/" }]
write = [{ package = "team/p", path = "out/" }]"#,
    ))?;
    let accesses: Vec<Access> = both.iter().map(|e| e.access).collect();
    assert_eq!(accesses, [Access::Read, Access::Write]);
    assert_eq!(both[0].folder, both[1].folder);

    for (read, write) in [
        (r#", path = "data/""#, r#", path = "data/x.csv""#),
        (r#", path = "data/x.csv""#, r#", path = "data/x.csv""#),
        (r#", path = "data""#, r#", path = "data/x/""#),
        ("", r#", path = "out/""#),
    ] {
        let contract = with_defaults(&format!(
            "read = [{{ package = \"team/p\"{read} }}]\nwrite = [{{ package = \"team/p\"{write} }}]"
        ));
        assert!(
            matches!(
                package_problem(&contract),
                PackageProblem::PathsOverlap(Access::Read, Access::Write)
            ),
            "{read} and {write}"
        );
    }
    Ok(())
}

#[test]
fn repeated_paths_under_one_access_are_their_union() -> Res {
    let union = |paths: &[&str]| -> Result<Paths, Box<dyn std::error::Error>> {
        let list: Vec<String> = paths
            .iter()
            .map(|path| match *path {
                "" => r#"{ package = "team/p" }"#.to_string(),
                path => format!(r#"{{ package = "team/p", path = "{path}" }}"#),
            })
            .collect();
        Ok(only(&with_defaults(&format!("sync = [{}]", list.join(", "))))?.paths)
    };
    assert_eq!(
        union(&["src/", "docs/"])?,
        Paths::Only(vec![
            PackagePath::Folder(key("docs")),
            PackagePath::Folder(key("src"))
        ])
    );
    assert_eq!(
        union(&["src/a.py", "src/", "src/a.py"])?,
        Paths::Only(vec![PackagePath::Folder(key("src"))])
    );
    assert_eq!(union(&["src/", ""])?, Paths::Whole);
    let collisions: [&[&str]; 6] = [
        &["a/b", "a/b/c/"],
        &["a/b/c/", "a/b"],
        &["src", "src/"],
        &["src/", "src/item", "src/item/"],
        &["src/item", "src/item/", "src/"],
        &["", "a/b", "a/b/c/"],
    ];
    for paths in collisions {
        let list: Vec<String> = paths
            .iter()
            .map(|path| match *path {
                "" => r#"{ package = "team/p" }"#.to_string(),
                path => format!(r#"{{ package = "team/p", path = "{path}" }}"#),
            })
            .collect();
        let problem = package_problem(&with_defaults(&format!("sync = [{}]", list.join(", "))));
        assert!(
            matches!(problem, PackageProblem::PathsCollide(Access::Sync)),
            "{paths:?}: {problem}"
        );
    }
    assert_eq!(
        union(&["src/a.py"])?,
        Paths::Only(vec![PackagePath::File(key("src/a.py"))])
    );
    Ok(())
}

#[test]
fn the_declarations_of_a_package_share_one_pin() -> Res {
    let shared = only(&with_defaults(
        r#"read = [{ package = "team/p", path = "a/", revision = "abc123" }, { package = "team/p", path = "b/" }]"#,
    ))?;
    assert_eq!(
        shared.revision,
        RevisionPointer::HashPrefix("abc123".to_string())
    );
    for other in [
        r#"{ package = "team/p", path = "b/", revision = "def456" }"#,
        r#"{ package = "team/p", path = "b/", revision = ":latest" }"#,
        r#"{ uri = "quilt+s3://quilt-dev#package=team/p:latest&path=b/", folder = "team/p" }"#,
        r#"{ uri = "quilt+s3://quilt-dev#%70ackage=team/p:latest&path=b/", folder = "team/p" }"#,
        r#"{ uri = "quilt+s3://quilt-dev#pack\tage=team/p:latest&path=b/", folder = "team/p" }"#,
    ] {
        let problem = package_problem(&with_defaults(&format!(
            r#"read = [{{ package = "team/p", path = "a/", revision = "abc123" }}, {other}]"#,
        )));
        assert!(
            matches!(problem, PackageProblem::Revisions),
            "{other}: {problem}"
        );
    }
    Ok(())
}

#[test]
fn one_package_written_two_ways_is_refused() {
    let problem = package_problem(&with_defaults(
        r#"read = ["team/raw", "quilt+s3://quilt-dev#package=team/raw"]"#,
    ));
    assert!(
        matches!(&problem, PackageProblem::Folders(one, other)
            if *one == folder("team/raw") && *other == folder("quilt-dev/team/raw")),
        "{problem}"
    );
}

#[test]
fn the_default_catalog_applies_only_where_an_entry_names_none() -> Res {
    let contract = |list: &str| {
        format!(
            "catalog = \"nightly.quilttest.com\"\n{}",
            with_defaults(list)
        )
    };
    let host = |name: &str| Host::from_str(name).unwrap();
    for list in [
        r#"read = ["team/raw"]"#,
        r#"read = [{ package = "team/raw" }]"#,
    ] {
        assert_eq!(
            only(&contract(list))?.catalog,
            Some(host("nightly.quilttest.com")),
            "{list}"
        );
    }
    assert_eq!(
        only(&contract(
            r#"read = [{ package = "team/raw", catalog = "open.quilt.bio" }]"#
        ))?
        .catalog,
        Some(host("open.quilt.bio"))
    );
    assert_eq!(
        only(&contract(
            r#"read = ["quilt+s3://quilt-dev#package=team/raw&catalog=open.quilt.bio"]"#
        ))?
        .catalog,
        Some(host("open.quilt.bio"))
    );
    assert!(matches!(
        package_problem(&contract(
            r#"read = [{ package = "team/raw", path = "a/" }, { package = "team/raw", path = "b/", catalog = "open.quilt.bio" }]"#
        )),
        PackageProblem::Catalogs
    ));
    Ok(())
}

#[test]
fn a_revision_is_a_hash_or_a_tag() -> Res {
    let revision = |text: &str| -> Result<RevisionPointer, Box<dyn std::error::Error>> {
        Ok(only(&with_defaults(&format!(
            r#"read = [{{ package = "team/p", revision = "{text}" }}]"#
        )))?
        .revision)
    };
    assert_eq!(revision(":latest")?, RevisionPointer::Tag(Tag::Latest));
    assert_eq!(
        revision(FULL_HASH)?,
        RevisionPointer::Hash(FULL_HASH.to_string())
    );
    assert_eq!(
        revision("abc123")?,
        RevisionPointer::HashPrefix("abc123".to_string())
    );
    for bad in ["latest", "ABC123", "abc12", ":", "xyz123"] {
        let problem = entry_problem(&with_defaults(&format!(
            r#"read = [{{ package = "team/p", revision = "{bad}" }}]"#
        )));
        assert!(
            matches!(problem, EntryProblem::Revision(_)),
            "{bad}: {problem}"
        );
    }
    Ok(())
}

#[test]
fn a_mistyped_uri_is_refused_rather_than_read_as_a_folder() {
    assert!(matches!(
        entry_problem(&with_defaults(
            r#"read = ["quilt+s3:/quilt-dev#package=team/raw"]"#
        )),
        EntryProblem::Uri(_)
    ));
}

#[test]
fn the_local_registry_is_refused_until_it_is_supported() {
    for list in [
        r#"read = ["quilt+local://host#package=me/analysis"]"#,
        r#"read = ["quilt+local:host#package=me/analysis"]"#,
        r#"read = [{ registry = "quilt+local://host", package = "me/analysis" }]"#,
    ] {
        let problem = entry_problem(&with_defaults(list));
        assert!(
            matches!(problem, EntryProblem::LocalRegistry),
            "{list}: {problem}"
        );
    }
}

#[test]
fn hooks_are_left_to_their_reader_and_unknown_keys_are_refused() -> Res {
    let hooked = format!(
        "{}\n[hooks.post-read]\ncommand = \"after-read\"\n",
        with_defaults(r#"read = ["team/raw"]"#)
    );
    assert_eq!(parse(&hooked)?.entries().len(), 1);
    assert!(matches!(
        parse("regsitry = \"s3://quilt-dev\""),
        Err(ContractError::Syntax(_))
    ));
    assert!(matches!(
        entry_problem(&with_defaults(r#"read = [{ pakage = "team/raw" }]"#)),
        EntryProblem::Table(_)
    ));
    assert!(matches!(
        entry_problem(&with_defaults("read = [1]")),
        EntryProblem::Spelling("integer")
    ));
    Ok(())
}

#[test]
fn a_folder_may_be_outside_the_dataspace() -> Res {
    let entry = only(&with_defaults(
        r#"read = [{ package = "me/analysis", folder = "/elsewhere/analysis" }]"#,
    ))?;
    assert_eq!(entry.folder, Path::new("/elsewhere/analysis"));
    Ok(())
}

#[test]
fn a_package_whose_name_holds_an_at_sign_is_refused() {
    for list in [
        r#"read = [{ package = "team/raw@abc123" }]"#,
        r#"read = [{ package = "me@host/raw" }]"#,
    ] {
        let problem = entry_problem(&with_defaults(list));
        assert!(
            matches!(problem, EntryProblem::AtInPackage),
            "{list}: {problem}"
        );
    }
}

#[test]
fn a_package_key_is_its_canonical_uri() -> Res {
    let entry = only(&with_defaults(
        r#"read = [{ uri = "quilt+s3://quilt-dev#package=team/raw:latest&path=a.csv&catalog=open.quilt.bio" }]"#,
    ))?;
    assert_eq!(
        entry.key().to_string(),
        "quilt+s3://quilt-dev#package=team/raw"
    );
    Ok(())
}
