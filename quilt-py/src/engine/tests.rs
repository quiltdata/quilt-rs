use super::*;

use test_log::test;

use quilt_rs::io::remote::mocks::MockRemote;
use quilt_rs::manifest::Manifest;
use quilt_rs::manifest::ManifestRow;
use quilt_uri::S3Uri;

const FIRST: &str = "1111111111111111111111111111111111111111111111111111111111111111";

type Test = std::result::Result<(), Box<dyn std::error::Error>>;

/// Stores `a.txt` holding `alpha` as revision [`FIRST`] of `team/raw` in
/// bucket `b`, and tags it `latest`.
async fn publish(remote: &MockRemote) -> Test {
    let body = b"alpha";
    let physical_key = format!("s3://b/team/raw/a.txt?versionId={FIRST}");
    remote
        .put_object(
            None,
            &S3Uri::try_from(physical_key.as_str())?,
            body.to_vec(),
        )
        .await?;
    let manifest = Manifest {
        rows: vec![ManifestRow {
            logical_key: quilt_rs::LogicalKey::new("a.txt")?,
            physical_key,
            hash: quilt_rs::checksum::hash_reader(&body[..], body.len() as u64, 0x12).await?,
            size: body.len() as u64,
            ..ManifestRow::default()
        }],
        ..Manifest::default()
    };
    remote
        .put_object(
            None,
            &S3Uri::try_from(format!("s3://b/.quilt/packages/{FIRST}").as_str())?,
            manifest.to_jsonlines().into_bytes(),
        )
        .await?;
    remote
        .put_object(
            None,
            &S3Uri::try_from("s3://b/.quilt/named_packages/team/raw/latest")?,
            FIRST.as_bytes().to_vec(),
        )
        .await?;
    Ok(())
}

fn request(access: &str) -> InstallRequest {
    InstallRequest {
        uri: "quilt+s3://b#package=team/raw".to_string(),
        access: access.to_string(),
        paths: Vec::new(),
        folder: None,
    }
}

/// A dataspace whose contract names bucket `b` and `contract`'s packages.
fn space(dir: &Path, packages: &str) -> std::io::Result<PathBuf> {
    let space = dir.join("space");
    std::fs::create_dir_all(&space)?;
    std::fs::write(
        space.join("data.toml"),
        format!("registry = \"s3://b\"\n{packages}"),
    )?;
    Ok(space)
}

#[test]
fn an_explicit_domain_and_home_are_used_as_given() -> Test {
    let dir = tempfile::TempDir::new()?;

    assert_eq!(domain(Some(dir.path()))?.root(), dir.path());
    assert_eq!(home(Some(dir.path()))?, dir.path());
    assert!(home(None)?.ends_with(HOME_DIR_NAME));
    Ok(())
}

#[test(tokio::test)]
async fn a_dataspace_named_is_used_and_none_takes_the_one_chosen() -> Test {
    let dir = tempfile::TempDir::new()?;
    let home = dir.path().join("home");

    assert_eq!(default_dataspace(&home).await?, None);
    assert!(matches!(
        dataspace(None, &home).await,
        Err(Error::NoDefaultDataspace)
    ));
    let named = dir.path().join("named");
    assert_eq!(dataspace(Some(named.clone()), &home).await?, named);

    let chosen = dir.path().join("chosen");
    quilt_rs::dataspace::home::set_home(&LocalStorage::new(), &home, &chosen).await?;
    assert_eq!(dataspace(None, &home).await?, chosen);
    assert_eq!(dataspace(Some(named.clone()), &home).await?, named);
    Ok(())
}

#[test(tokio::test)]
async fn a_domain_that_is_not_there_holds_nothing_and_is_not_made() -> Test {
    let dir = tempfile::TempDir::new()?;
    let domain = DomainRoot::new(dir.path().join("domain"));

    assert_eq!(list_registries(&domain).await?, Vec::<String>::new());
    assert_eq!(list_stored(&domain).await?, Vec::<Value>::new());
    assert!(!domain.root().exists());
    Ok(())
}

#[test(tokio::test)]
async fn an_install_adds_a_sync_entry_reads_it_and_does_nothing_again() -> Test {
    let remote = MockRemote::default();
    publish(&remote).await?;
    let dir = tempfile::TempDir::new()?;
    let space = space(dir.path(), "")?;
    let domain = DomainRoot::new(dir.path().join("domain"));

    let first = install(&remote, &domain, &space, request("sync")).await?;
    let again = install(&remote, &domain, &space, request("sync")).await?;

    assert_eq!(first["package"], "quilt+s3://b#package=team/raw");
    assert_eq!(first["access"], "sync");
    assert_eq!(first["declared"], true);
    assert_eq!(first["outcome"], "read", "{first}");
    assert_eq!(first["completed"], true);
    assert_eq!(first["revision"], FIRST);
    assert_eq!(first["error"], Value::Null);
    assert_eq!(again["declared"], false);
    assert_eq!(again["outcome"], "unchanged", "{again}");
    assert_eq!(std::fs::read(space.join("team/raw/a.txt"))?, b"alpha");
    let contract = std::fs::read_to_string(space.join("data.toml"))?;
    assert!(contract.contains("sync"), "{contract}");
    Ok(())
}

#[test(tokio::test)]
async fn an_install_under_another_access_than_declared_fails() -> Test {
    let remote = MockRemote::default();
    publish(&remote).await?;
    let dir = tempfile::TempDir::new()?;
    let space = space(dir.path(), "[packages]\nread = [\"team/raw\"]\n")?;
    let domain = DomainRoot::new(dir.path().join("domain"));

    let result = install(&remote, &domain, &space, request("sync")).await;

    assert!(matches!(result, Err(Error::Install(_))), "{result:?}");
    Ok(())
}

#[test(tokio::test)]
async fn an_access_that_is_not_one_is_refused_before_anything_is_written() -> Test {
    let remote = MockRemote::default();
    let dir = tempfile::TempDir::new()?;
    let space = space(dir.path(), "")?;
    let domain = DomainRoot::new(dir.path().join("domain"));

    let result = install(&remote, &domain, &space, request("pull")).await;

    assert!(matches!(result, Err(Error::Access(_))), "{result:?}");
    assert_eq!(
        std::fs::read_to_string(space.join("data.toml"))?,
        "registry = \"s3://b\"\n"
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_commit_is_kept_in_the_domain_and_listed_with_its_registry() -> Test {
    let dir = tempfile::TempDir::new()?;
    let space = space(dir.path(), "[packages]\nsync = [\"team/raw\"]\n")?;
    std::fs::create_dir_all(space.join("team/raw"))?;
    std::fs::write(space.join("team/raw/a.txt"), b"alpha")?;
    let domain = DomainRoot::new(dir.path().join("domain"));
    let home = dir.path().join("home");

    let committed = commit(
        &MockRemote::default(),
        &domain,
        &home,
        &space,
        "team/raw",
        "first",
    )
    .await?;

    let hash = committed["commit"].as_str().ok_or("no commit")?.to_string();
    assert_eq!(committed["package"], "quilt+s3://b#package=team/raw");
    assert_eq!(committed["base"], Value::Null);
    assert_eq!(committed["commits"], json!([hash]));
    assert_eq!(committed["warnings"], json!([]));
    let registries = list_registries(&domain).await?;
    assert_eq!(registries.first().map(String::as_str), Some("s3://b"));
    assert!(
        registries
            .get(1)
            .is_some_and(|local| local.starts_with("quilt+local://")),
        "{registries:?}"
    );
    let packages = list_stored(&domain).await?;
    assert_eq!(packages.len(), 1, "{packages:?}");
    assert_eq!(packages[0]["package"], "quilt+s3://b#package=team/raw");
    assert_eq!(packages[0]["registry"], "s3://b");
    assert_eq!(packages[0]["name"], "team/raw");
    Ok(())
}

#[test(tokio::test)]
async fn a_name_the_contract_does_not_give_one_package_is_refused() -> Test {
    let dir = tempfile::TempDir::new()?;
    let space = space(dir.path(), "[packages]\nsync = [\"team/raw\"]\n")?;
    let storage = LocalStorage::new();
    let contract = Contract::read(&storage, &space, None).await?;

    assert!(matches!(
        package_key(&contract, "team/other"),
        Err(Error::NotDeclared(_))
    ));
    assert_eq!(
        package_key(&contract, "team/raw")?.to_string(),
        "quilt+s3://b#package=team/raw"
    );
    assert_eq!(
        package_key(&contract, "quilt+s3://c#package=team/raw")?.to_string(),
        "quilt+s3://c#package=team/raw"
    );
    Ok(())
}

#[test(tokio::test)]
async fn a_created_package_is_declared_under_sync_and_committed_once() -> Test {
    let dir = tempfile::TempDir::new()?;
    let space = space(dir.path(), "")?;
    std::fs::create_dir_all(space.join("team/new"))?;
    std::fs::write(space.join("team/new/a.txt"), b"alpha")?;
    let domain = DomainRoot::new(dir.path().join("domain"));
    let home = dir.path().join("home");
    let remote = MockRemote::default();
    let request = CreateRequest {
        uri: "quilt+s3://b#package=team/new".to_string(),
        access: "sync".to_string(),
        folder: None,
        message: Some("first".to_string()),
    };

    let created = create(&remote, &domain, &home, &space, request.clone()).await?;
    let again = create(&remote, &domain, &home, &space, request).await;

    assert_eq!(created["package"], "quilt+s3://b#package=team/new");
    assert_eq!(created["access"], "sync");
    assert!(created["commit"].is_string(), "{created}");
    assert_eq!(created["warnings"], json!([]));
    assert!(
        matches!(again, Err(Error::Create(CreateError::Declared(_)))),
        "{again:?}"
    );
    let contract = std::fs::read_to_string(space.join("data.toml"))?;
    assert!(contract.contains("team/new"), "{contract}");
    let packages = list_stored(&domain).await?;
    assert_eq!(packages.len(), 1, "{packages:?}");
    assert_eq!(packages[0]["name"], "team/new");
    Ok(())
}

#[test(tokio::test)]
async fn a_package_is_not_created_under_read() -> Test {
    let dir = tempfile::TempDir::new()?;
    let space = space(dir.path(), "")?;
    let domain = DomainRoot::new(dir.path().join("domain"));
    let request = CreateRequest {
        uri: "quilt+s3://b#package=team/new".to_string(),
        access: "read".to_string(),
        folder: None,
        message: None,
    };

    let result = create(
        &MockRemote::default(),
        &domain,
        &dir.path().join("home"),
        &space,
        request,
    )
    .await;

    assert!(
        matches!(result, Err(Error::Create(CreateError::Read))),
        "{result:?}"
    );
    assert_eq!(
        std::fs::read_to_string(space.join("data.toml"))?,
        "registry = \"s3://b\"\n"
    );
    Ok(())
}

#[test(tokio::test)]
async fn an_install_whose_read_fails_is_an_error() -> Test {
    let remote = MockRemote::default();
    publish(&remote).await?;
    let dir = tempfile::TempDir::new()?;
    // A `pre-read` hook that exits 1 leaves the entry declared and the
    // read failed, as the engine does; the binding must raise, not return
    // a dict with `"outcome": "failed"`.
    let space = space(dir.path(), "\n[hooks.pre-read]\ncommand = \"false\"\n")?;
    let domain = DomainRoot::new(dir.path().join("domain"));

    let result = install(&remote, &domain, &space, request("sync")).await;

    assert!(matches!(result, Err(Error::FailedRead(_))), "{result:?}");
    let contract = std::fs::read_to_string(space.join("data.toml"))?;
    assert!(contract.contains("sync"), "{contract}");
    assert!(!space.join("team/raw/a.txt").exists());
    Ok(())
}

/// Makes bucket `b`'s default workflow `gate`, which requires `owner` in
/// the package metadata.
async fn gate(remote: &MockRemote) -> Test {
    let config = "version: \"1\"\ndefault_workflow: gate\nworkflows:\n  gate:\n    name: Gate\n    metadata_schema: meta\nschemas:\n  meta:\n    url: s3://b/schemas/meta.json\n";
    let schema = r#"{"type": "object", "required": ["owner"]}"#;
    for (uri, body) in [
        ("s3://b/.quilt/workflows/config.yml", config),
        ("s3://b/schemas/meta.json", schema),
    ] {
        remote
            .put_object(None, &S3Uri::try_from(uri)?, body.as_bytes().to_vec())
            .await?;
    }
    Ok(())
}

/// A commit the bucket's workflow refuses is still made, and says so in
/// `warnings` rather than raising.
#[test(tokio::test)]
async fn a_commit_the_workflow_refuses_is_made_and_warns() -> Test {
    let dir = tempfile::TempDir::new()?;
    let space = space(dir.path(), "[packages]\nsync = [\"team/raw\"]\n")?;
    std::fs::create_dir_all(space.join("team/raw"))?;
    std::fs::write(space.join("team/raw/a.txt"), b"alpha")?;
    let domain = DomainRoot::new(dir.path().join("domain"));
    let remote = MockRemote::default();
    gate(&remote).await?;

    let committed = commit(
        &remote,
        &domain,
        &dir.path().join("home"),
        &space,
        "team/raw",
        "first",
    )
    .await?;

    assert!(committed["commit"].is_string(), "{committed}");
    let warnings = committed["warnings"].as_array().ok_or("no warnings")?;
    assert_eq!(warnings.len(), 1, "{committed}");
    assert_eq!(warnings[0]["kind"], "inconsistent");
    assert_eq!(warnings[0]["bucket"], "b");
    assert!(
        warnings[0]["reason"]
            .as_str()
            .is_some_and(|reason| reason.contains("owner")),
        "{committed}"
    );
    assert!(
        warnings[0]["message"]
            .as_str()
            .is_some_and(|message| message.contains("s3://b")),
        "{committed}"
    );
    Ok(())
}

/// A package created where the workflows config cannot be read is created,
/// with an `unchecked` warning.
#[test(tokio::test)]
async fn a_package_created_unchecked_warns() -> Test {
    let dir = tempfile::TempDir::new()?;
    let space = space(dir.path(), "")?;
    std::fs::create_dir_all(space.join("team/new"))?;
    std::fs::write(space.join("team/new/a.txt"), b"alpha")?;
    let remote = MockRemote::default();
    remote.deny_reads("s3://b/.quilt/workflows/config.yml");
    let request = CreateRequest {
        uri: "quilt+s3://b#package=team/new".to_string(),
        access: "sync".to_string(),
        folder: None,
        message: None,
    };

    let created = create(
        &remote,
        &DomainRoot::new(dir.path().join("domain")),
        &dir.path().join("home"),
        &space,
        request,
    )
    .await?;

    assert!(created["commit"].is_string(), "{created}");
    let warnings = created["warnings"].as_array().ok_or("no warnings")?;
    assert_eq!(warnings.len(), 1, "{created}");
    assert_eq!(warnings[0]["kind"], "unchecked");
    assert_eq!(warnings[0]["bucket"], "b");
    Ok(())
}

/// The contract's entries come back with their access, folder and paths,
/// and listing them names no local registry.
#[test(tokio::test)]
async fn the_entries_are_listed_and_nothing_is_written() -> Test {
    let dir = tempfile::TempDir::new()?;
    let space = space(
        dir.path(),
        "[packages]\nsync = [\"team/raw\"]\nread = [{ package = \"team/other\", path = \"src/\" }]\n",
    )?;
    let domain = DomainRoot::new(dir.path().join("domain"));

    let declared = entries(&domain, &space).await?;

    // Join each package segment: Path::join("team/other") keeps the slash
    // as one component on Windows, while the contract's folder uses the OS
    // separator between owner and name.
    let folder = |owner: &str, name: &str| space.join(owner).join(name).display().to_string();
    assert_eq!(
        declared,
        [
            json!({
                "package": "quilt+s3://b#package=team/other",
                "registry": "s3://b",
                "name": "team/other",
                "access": "read",
                "folder": folder("team", "other"),
                "paths": ["src/"],
            }),
            json!({
                "package": "quilt+s3://b#package=team/raw",
                "registry": "s3://b",
                "name": "team/raw",
                "access": "sync",
                "folder": folder("team", "raw"),
                "paths": null,
            }),
        ]
    );
    assert!(!domain.root().exists());
    Ok(())
}

/// The metadata set is what the next commit publishes.
#[test(tokio::test)]
async fn the_metadata_set_is_published_by_the_next_commit() -> Test {
    let dir = tempfile::TempDir::new()?;
    let space = space(dir.path(), "[packages]\nsync = [\"team/raw\"]\n")?;
    let folder = space.join("team/raw");
    std::fs::create_dir_all(&folder)?;
    std::fs::write(folder.join("a.txt"), b"alpha")?;
    let domain = DomainRoot::new(dir.path().join("domain"));
    let home = dir.path().join("home");

    set_package_metadata(&folder, &json!({"assay": "rna", "n": 3})).await?;
    let committed = commit(
        &MockRemote::default(),
        &domain,
        &home,
        &space,
        "team/raw",
        "first",
    )
    .await?;

    let hash = committed["commit"].as_str().ok_or("no commit")?;
    let manifest = Manifest::from_path(
        &LocalStorage::new(),
        &domain
            .registry("b")?
            .installed_manifest(&Namespace::try_from("team/raw")?, hash),
    )
    .await?;
    assert_eq!(
        serde_json::to_value(&manifest.header.user_meta)?,
        json!({"assay": "rna", "n": 3})
    );
    assert!(
        !manifest
            .rows
            .iter()
            .any(|row| row.logical_key.to_string() == "metadata.toml"),
        "{manifest:?}"
    );
    Ok(())
}

/// A committed package opens at its commit, its hash resolves from a prefix
/// in its store, and deleting it takes it out of every store, after which
/// a second delete finds nothing.
#[test(tokio::test)]
async fn a_commit_opens_resolves_and_is_deleted_from_the_stores() -> Test {
    let dir = tempfile::TempDir::new()?;
    let space = space(dir.path(), "[packages]\nsync = [\"team/raw\"]\n")?;
    std::fs::create_dir_all(space.join("team/raw"))?;
    std::fs::write(space.join("team/raw/a.txt"), b"alpha")?;
    let domain = DomainRoot::new(dir.path().join("domain"));
    let home = dir.path().join("home");
    let remote = MockRemote::default();
    let committed = commit(&remote, &domain, &home, &space, "team/raw", "first").await?;
    let hash = committed["commit"].as_str().ok_or("no commit")?.to_string();

    let browsed = browse(&remote, &domain, &space, "team/raw").await?;
    let resolved = resolve_hash(&domain, "quilt+s3://b#package=team/raw", &hash[..10]).await?;
    let deleted = delete(&domain, "team/raw", None).await?;
    let again = delete(&domain, "team/raw", None).await;

    assert_eq!(browsed["package"], "quilt+s3://b#package=team/raw");
    assert_eq!(browsed["registry"], "s3://b");
    assert_eq!(browsed["name"], "team/raw");
    assert_eq!(browsed["hash"], hash.as_str());
    let manifest = browsed["manifest"].as_str().ok_or("no manifest")?;
    assert!(manifest.contains("\"a.txt\""), "{manifest}");
    assert_eq!(resolved, hash);
    assert_eq!(
        deleted,
        [json!({
            "package": "quilt+s3://b#package=team/raw",
            "registry": "s3://b",
            "revisions": [hash],
            "latest": {"after": "kept", "hash": null},
        })]
    );
    assert_eq!(list_stored(&domain).await?, Vec::<Value>::new());
    assert!(
        matches!(again, Err(Error::Delete(DeleteError::NotFound(_)))),
        "{again:?}"
    );
    assert_eq!(std::fs::read(space.join("team/raw/a.txt"))?, b"alpha");
    Ok(())
}

/// A package not yet read has nothing to open, and an ambiguous or unknown
/// prefix is refused.
#[test(tokio::test)]
async fn nothing_to_open_and_no_revision_are_errors() -> Test {
    let dir = tempfile::TempDir::new()?;
    let space = space(dir.path(), "[packages]\nsync = [\"team/raw\"]\n")?;
    let domain = DomainRoot::new(dir.path().join("domain"));

    let browsed = browse(&MockRemote::default(), &domain, &space, "team/raw").await;
    let resolved = resolve_hash(&domain, "quilt+s3://b#package=team/raw", "1111111").await;

    assert!(
        matches!(browsed, Err(Error::Browse(BrowseError::NotRead(_)))),
        "{browsed:?}"
    );
    assert!(matches!(resolved, Err(Error::Quilt(_))), "{resolved:?}");
    Ok(())
}
