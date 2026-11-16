//! A package tracked with no working copy, driven through [`LocalDomain`] and
//! [`InstalledPackage`] against a mock remote: no flow makes or reads its
//! folder until it is opened as a working copy.

use super::older_revision_tests::BUCKET;
use super::older_revision_tests::publish_manifest;
use super::older_revision_tests::publish_row;
use super::*;

use test_log::test;

use crate::InstallPackageError;
use crate::LineageError;
use crate::LocalDomain;
use crate::io::manifest::tag_latest;
use crate::io::remote::mocks::MockRemote;
use crate::lineage::DomainLineage;
use crate::lineage::Home;
use crate::paths::DomainPaths;
use quilt_uri::Namespace;

struct Tracked {
    domain: LocalDomain<LocalStorage, MockRemote>,
    package: InstalledPackage<LocalStorage, MockRemote>,
    paths: DomainPaths,
    home: Home,
    _dirs: [tempfile::TempDir; 3],
}

impl Tracked {
    /// `f/a` tracked with no working copy, at a revision of `a.txt` and
    /// `b.txt` that is also the remote's latest.
    async fn new() -> Res<Self> {
        let scratch = tempfile::TempDir::new()?;
        let remote = MockRemote::default();
        let namespace: Namespace = ("f", "a").into();
        let a = publish_row(&remote, scratch.path(), "a.txt", "v1", b"alpha").await?;
        let b = publish_row(&remote, scratch.path(), "b.txt", "v1", b"beta").await?;
        let manifest_uri = ManifestUri {
            bucket: BUCKET.to_string(),
            namespace,
            hash: publish_manifest(&remote, vec![a, b]).await?,
            origin: None,
        };
        tag_latest(&remote, &manifest_uri).await?;

        let (paths, paths_dir) = DomainPaths::from_temp_dir()?;
        let (home, home_dir) = Home::from_temp_dir()?;
        let domain = LocalDomain::with_parts(paths.clone(), LocalStorage::new(), Arc::new(remote));
        domain.set_home(home.as_ref()).await?;
        let package = domain.track_package(&manifest_uri).await?;
        Ok(Self {
            domain,
            package,
            paths,
            home,
            _dirs: [scratch, paths_dir, home_dir],
        })
    }

    fn folder(&self) -> PathBuf {
        paths::package_home(&self.home, &self.package.namespace)
    }

    /// `data.json` as a build that predates working copies reads it.
    fn data_json(&self) -> Res<DomainLineage> {
        DomainLineage::from_slice(&std::fs::read(self.paths.lineage())?)
    }
}

fn is_no_working_copy<T>(result: &Res<T>) -> bool {
    matches!(result, Err(Error::Lineage(LineageError::NoWorkingCopy(_))))
}

#[test(tokio::test)]
async fn a_tracked_package_is_listed_with_no_folder() -> Res {
    let t = Tracked::new().await?;

    let listed = t.domain.list_installed_packages().await?;
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].namespace, t.package.namespace);
    assert_eq!(
        t.package.lineage().await?.working_copy,
        WorkingCopyLocation::None
    );
    assert!(!t.folder().exists());
    Ok(())
}

/// An older build reads `data.json` alone, so it never sees the package and
/// so never reads its missing folder as every file removed.
#[test(tokio::test)]
async fn an_older_build_does_not_see_a_tracked_package() -> Res {
    let t = Tracked::new().await?;

    assert!(t.data_json()?.packages.is_empty());
    Ok(())
}

#[test(tokio::test)]
async fn status_of_a_tracked_package_reads_no_folder() -> Res {
    let t = Tracked::new().await?;

    let status = t.package.status(Some(HostConfig::default())).await?;
    assert_eq!(status.working_copy, WorkingCopy::Absent);
    assert!(status.changes.is_empty());
    assert_eq!(status.upstream_state, UpstreamState::UpToDate);
    assert!(!t.folder().exists());
    Ok(())
}

#[test(tokio::test)]
async fn commit_and_install_paths_refuse_a_tracked_package() -> Res {
    let t = Tracked::new().await?;

    let committed = t
        .package
        .commit("c".to_string(), UserMeta::Keep, None, None)
        .await;
    assert!(is_no_working_copy(&committed));
    let installed = t.package.install_paths(&[PathBuf::from("a.txt")]).await;
    assert!(is_no_working_copy(&installed));
    assert!(!t.folder().exists());
    Ok(())
}

#[test(tokio::test)]
async fn opening_places_every_file_in_a_working_copy() -> Res {
    let t = Tracked::new().await?;

    t.package.open_working_copy(None).await?;

    let read = |name| std::fs::read(t.folder().join(name)).ok();
    assert_eq!(read("a.txt").as_deref(), Some(&b"alpha"[..]));
    assert_eq!(read("b.txt").as_deref(), Some(&b"beta"[..]));
    let status = t.package.status(Some(HostConfig::default())).await?;
    assert_eq!(status.working_copy, WorkingCopy::Available);
    assert!(status.changes.is_empty(), "{:?}", status.changes);
    // Now an ordinary working copy, which an older build reads as one.
    assert!(t.data_json()?.packages.contains_key(&t.package.namespace));
    Ok(())
}

#[test(tokio::test)]
async fn opening_places_only_the_paths_named() -> Res {
    let t = Tracked::new().await?;

    t.package
        .open_working_copy(Some(&[PathBuf::from("a.txt")]))
        .await?;

    assert!(t.folder().join("a.txt").exists());
    assert!(!t.folder().join("b.txt").exists());
    Ok(())
}

/// Files already in the folder would read as the package's own changes, so
/// the open is refused and leaves them alone.
#[test(tokio::test)]
async fn an_open_into_a_folder_holding_files_is_refused() -> Res {
    let t = Tracked::new().await?;
    std::fs::create_dir_all(t.folder())?;
    std::fs::write(t.folder().join("mine.txt"), "mine")?;

    let opened = t.package.open_working_copy(None).await;

    assert!(
        matches!(opened, Err(Error::Lineage(LineageError::FolderNotEmpty(_)))),
        "{opened:?}"
    );
    assert_eq!(
        t.package.lineage().await?.working_copy,
        WorkingCopyLocation::None
    );
    assert!(t.folder().join("mine.txt").exists());
    Ok(())
}

/// A file browser's metadata is not the user's work.
#[test(tokio::test)]
async fn an_open_into_a_folder_holding_only_os_metadata_proceeds() -> Res {
    let t = Tracked::new().await?;
    std::fs::create_dir_all(t.folder())?;
    std::fs::write(t.folder().join(".DS_Store"), "")?;

    let opened = t.package.open_working_copy(None).await;

    assert!(opened.is_ok(), "{opened:?}");
    assert_eq!(
        t.package.lineage().await?.working_copy,
        WorkingCopyLocation::Home
    );
    Ok(())
}

#[test(tokio::test)]
async fn keep_refuses_a_tracked_package_and_saves_no_scope() -> Res {
    let t = Tracked::new().await?;

    let kept = t.package.keep(SyncScope::EntirePackage, false).await;

    assert!(is_no_working_copy(&kept));
    assert_eq!(
        t.package.lineage().await?.sync_scope,
        SyncScope::IndividualFiles
    );
    Ok(())
}

#[test(tokio::test)]
async fn installing_a_tracked_package_again_makes_no_folder() -> Res {
    let t = Tracked::new().await?;
    let remote = t.package.lineage().await?.remote_uri.expect("a remote");

    let installed = t.domain.install_package(&remote).await;

    assert!(matches!(
        installed,
        Err(Error::InstallPackage(
            InstallPackageError::AlreadyInstalled(_)
        ))
    ));
    assert!(!t.folder().exists());
    Ok(())
}

#[test(tokio::test)]
async fn an_open_that_fails_leaves_no_folder_and_no_working_copy() -> Res {
    let t = Tracked::new().await?;

    let opened = t
        .package
        .open_working_copy(Some(&[PathBuf::from("not-in-the-package.txt")]))
        .await;

    assert!(opened.is_err());
    assert_eq!(
        t.package.lineage().await?.working_copy,
        WorkingCopyLocation::None
    );
    assert!(!t.folder().exists());
    Ok(())
}

#[test(tokio::test)]
async fn an_open_package_cannot_be_opened_again() -> Res {
    let t = Tracked::new().await?;
    t.package.open_working_copy(None).await?;

    let again = t.package.open_working_copy(None).await;
    assert!(matches!(
        again,
        Err(Error::InstallPackage(
            InstallPackageError::AlreadyInstalled(_)
        ))
    ));
    Ok(())
}

/// A folder at `<home>/<namespace>` is not the package's when it has no
/// working copy, so uninstalling leaves it.
#[test(tokio::test)]
async fn uninstalling_a_tracked_package_leaves_the_folder_at_its_name() -> Res {
    let t = Tracked::new().await?;
    std::fs::create_dir_all(t.folder())?;
    std::fs::write(t.folder().join("mine.txt"), "mine")?;

    t.domain
        .uninstall_package(t.package.namespace.clone())
        .await?;

    assert!(t.domain.list_installed_packages().await?.is_empty());
    assert!(t.folder().join("mine.txt").exists());
    Ok(())
}

#[test(tokio::test)]
async fn gc_keeps_the_objects_a_tracked_package_uses() -> Res {
    let t = Tracked::new().await?;
    let manifest = t.package.manifest().await?;
    let object = t.paths.object(manifest.rows[0].hash.digest());
    std::fs::write(&object, "alpha")?;

    t.domain.gc().await?;

    assert!(object.exists());
    Ok(())
}
