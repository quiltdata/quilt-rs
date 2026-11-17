//! `LocalDomain` on the lineage store `BACKEND`.

use tempfile::TempDir;
use test_log::test;

use super::*;
use crate::lineage::LineageStore;
use crate::lineage::PackageLineage;

fn domain(root: impl AsRef<Path>) -> LocalDomain {
    LocalDomain::new(root).with_lineage_store(BACKEND)
}

/// A domain that was never used, not even its folder, has no home, and
/// reading that creates nothing.
#[test(tokio::test)]
async fn stored_home_on_a_missing_domain_is_none_and_creates_nothing() -> Res<()> {
    let temp_dir = TempDir::new()?;
    let root = temp_dir.path().join("missing");
    let domain = domain(&root);

    assert_eq!(domain.stored_home().await?, None);
    assert!(!root.exists());
    Ok(())
}

/// A domain whose folder can't be looked into is not reported as having
/// no home.
#[cfg(unix)]
#[test(tokio::test)]
async fn stored_home_on_an_unreadable_domain_is_an_error() -> Res<()> {
    use std::os::unix::fs::PermissionsExt;

    let temp_dir = TempDir::new()?;
    let domain = domain(temp_dir.path());
    domain.set_home(temp_dir.path()).await?;
    let folder = domain.paths.dot_quilt_dir();
    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o000))?;
    let readable = std::fs::read_dir(&folder).is_ok();

    let result = domain.stored_home().await;

    std::fs::set_permissions(&folder, std::fs::Permissions::from_mode(0o700))?;
    // Root reads it anyway.
    if !readable {
        assert!(result.is_err(), "{result:?}");
    }
    Ok(())
}

#[test(tokio::test)]
async fn stored_home_reads_the_home_that_was_set() -> Res<()> {
    let temp_dir = TempDir::new()?;
    let domain = domain(temp_dir.path());
    domain.set_home(temp_dir.path()).await?;

    assert_eq!(
        domain.stored_home().await?,
        Some(Home::from(temp_dir.path()))
    );
    Ok(())
}

/// Creating a package that is installed already is refused, and the
/// installed one keeps its lineage.
#[test(tokio::test)]
async fn creating_an_installed_package_again_is_refused() -> Res<()> {
    let temp_dir = TempDir::new()?;
    let domain = domain(temp_dir.path());
    domain.set_home(temp_dir.path()).await?;
    let namespace = Namespace::from(("foo", "bar"));
    let (package, _) = domain
        .create_package(namespace.clone(), None, None, flow::UserMeta::Clear)
        .await?;
    let first = package.lineage().await?;

    let again = domain
        .create_package(
            namespace.clone(),
            None,
            Some("again".to_string()),
            flow::UserMeta::Clear,
        )
        .await;

    assert!(
        matches!(
            again,
            Err(Error::InstallPackage(
                crate::InstallPackageError::AlreadyInstalled(ref ns)
            )) if *ns == namespace
        ),
        "{again:?}"
    );
    assert_eq!(package.lineage().await?, first);
    Ok(())
}

#[test(tokio::test)]
async fn test_list_installed_packages() -> Res<()> {
    // Create a temporary directory for testing
    let temp_dir = TempDir::new()?;
    let local_domain = domain(temp_dir.path());

    // Set home directory
    local_domain.set_home(&temp_dir.path()).await?;

    // Initially there should be no packages
    let packages = local_domain.list_installed_packages().await?;
    assert!(packages.is_empty());

    // Add some packages to the lineage
    let namespaces = [
        Namespace::from(("foo", "bar")),
        Namespace::from(("test", "package")),
        Namespace::from(("abc", "xyz")),
    ];

    let packages = namespaces.iter().map(|namespace| {
        (
            namespace.clone(),
            PackageLineage {
                commit: None,
                remote_uri: Some(ManifestUri {
                    bucket: "test-bucket".to_string(),
                    namespace: namespace.clone(),
                    hash: "abcdef0000000000000000000000000000000000000000000000000000000000"
                        .to_string(),
                    origin: None,
                }),
                base_hash: "abcdef0000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
                latest_hash: "abcdef0000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
                paths: std::collections::BTreeMap::new(),
                ..PackageLineage::default()
            },
        )
    });
    lineage::DomainLineageIo::write_fixture(
        BACKEND,
        &local_domain.storage,
        &local_domain.paths,
        temp_dir.path(),
        packages,
    )
    .await?;

    // Now list_installed_packages should return packages in sorted order
    let packages = local_domain.list_installed_packages().await?;
    assert_eq!(packages.len(), 3);

    // Check that packages are returned in sorted order by namespace
    assert_eq!(packages[0].namespace, Namespace::from(("abc", "xyz")));
    assert_eq!(packages[1].namespace, Namespace::from(("foo", "bar")));
    assert_eq!(packages[2].namespace, Namespace::from(("test", "package")));

    Ok(())
}

/// Each store's files appear only in a domain that uses it: refs leave in
/// `data.json` only the stub, and `data.json` never creates a ref or the
/// domain config.
#[test(tokio::test)]
async fn a_domain_writes_only_its_own_stores_files() -> Res<()> {
    let temp_dir = TempDir::new()?;
    let source = temp_dir.path().join("source");
    std::fs::create_dir_all(&source)?;
    std::fs::write(source.join("a.txt"), b"a")?;
    let domain = domain(temp_dir.path().join("domain"));
    domain.set_home(temp_dir.path().join("home")).await?;
    let (package, _) = domain
        .create_package(
            ("foo", "bar").into(),
            Some(source),
            None,
            flow::UserMeta::Clear,
        )
        .await?;
    std::fs::write(package.package_home().await?.join("b.txt"), b"b")?;
    package
        .commit("b".to_string(), flow::UserMeta::Keep, None, None)
        .await?;
    package
        .uninstall_paths(&vec![PathBuf::from("a.txt")])
        .await?;
    domain
        .create_package(("foo", "baz").into(), None, None, flow::UserMeta::Clear)
        .await?;
    domain.uninstall_package(("foo", "baz").into()).await?;
    domain.gc().await?;

    let paths = &domain.paths;
    match BACKEND {
        LineageStore::DataJson => {
            assert!(!paths.refs_dir().exists());
            assert!(!paths.domain_config().exists());
        }
        LineageStore::Refs => assert!(
            std::fs::read(paths.lineage())
                .is_ok_and(|bytes| crate::lineage::refs::state::is_stub(&bytes))
        ),
    }
    assert_eq!(
        domain.list_installed_packages().await?.len(),
        1,
        "the writes landed"
    );
    Ok(())
}

/// The ways a package's stored entry can be one this build cannot use.
#[derive(Clone, Copy, Debug)]
enum Damage {
    Unparsable,
    LaterFormat,
    TwoCopies,
}

async fn damage(domain: &LocalDomain, namespace: &Namespace, how: Damage) -> Res<()> {
    let edit: fn(&mut serde_json::Value) = match how {
        Damage::Unparsable => {
            return domain
                .lineage
                .damage_fixture(&domain.storage, namespace)
                .await;
        }
        Damage::LaterFormat => |stored| stored["format"] = 3.into(),
        Damage::TwoCopies => |stored| {
            let copy = stored["copies"][0].clone();
            stored["copies"]
                .as_array_mut()
                .expect("a copies list")
                .push(copy);
        },
    };
    let path = domain.paths.package_ref(namespace);
    let mut stored = serde_json::from_slice(&std::fs::read(&path)?)?;
    edit(&mut stored);
    std::fs::write(&path, serde_json::to_vec(&stored)?)?;
    Ok(())
}

/// Validation item 1 of <https://github.com/quiltdata/quilt-specs/pull/292>:
/// with one package's entry damaged, every other package is still listed,
/// but on `data.json`, which is read whole, none of them can be used, and on
/// refs all of them can. The listing names the damaged package either way.
#[test(tokio::test)]
async fn a_damaged_package_costs_every_other_on_data_json_and_none_on_refs() -> Res<()> {
    // What the damaged package's error says: on refs, the file it is in.
    let (ways, named, usable_others): (&[Damage], _, _) = match BACKEND {
        LineageStore::DataJson => (&[Damage::Unparsable], "Failed to parse lineage file", 0),
        LineageStore::Refs => (
            &[Damage::Unparsable, Damage::LaterFormat, Damage::TwoCopies],
            "damaged.json",
            3,
        ),
    };
    for &how in ways {
        let temp_dir = TempDir::new()?;
        let domain = domain(temp_dir.path().join("domain"));
        let home = domain.set_home(temp_dir.path().join("home")).await?;
        let damaged = Namespace::from(("acme", "damaged"));
        let others = [("acme", "one"), ("acme", "two"), ("other", "three")].map(Namespace::from);
        for namespace in others.iter().chain([&damaged]) {
            domain
                .create_package(namespace.clone(), None, None, flow::UserMeta::Clear)
                .await?;
        }
        damage(&domain, &damaged, how).await?;

        let listed = domain.read_packages().await?;
        let error = listed[&damaged].as_ref().err().map(ToString::to_string);
        assert!(
            error.as_ref().is_some_and(|err| err.contains(named)),
            "{how:?}: {error:?}"
        );
        let mut usable = 0;
        for namespace in &others {
            std::fs::write(
                paths::package_home(&home, namespace).join("new.txt"),
                b"new",
            )?;
            let committed = domain
                .create_installed_package(namespace.clone())
                .commit("new".to_string(), flow::UserMeta::Keep, None, None)
                .await;
            match committed {
                Ok(flow::Committed::Recorded(_)) => usable += 1,
                // `data.json` is parsed whole, so the damaged entry stops it.
                Err(Error::Lineage(LineageError::Parse(_))) => {}
                other => panic!("{how:?}: {namespace}: {other:?}"),
            }
        }
        let still_listed = others
            .iter()
            .filter(|namespace| listed.get(namespace).is_some_and(Result::is_ok))
            .count();
        assert_eq!(
            (still_listed, usable),
            (others.len(), usable_others),
            "{how:?}"
        );
    }
    Ok(())
}
