//!
//! `data.json` read and written whole, each write under the short lock
//! `data.json.lock`, as a build from before refs does: the older build that
//! tests run beside this one.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::path::Path;
use std::path::PathBuf;

use serde::Deserialize;

use super::DomainLineage;
use super::parse_record;
use super::read_bytes;
use crate::Error;
use crate::InstallPackageError;
use crate::Res;
use crate::error::LineageError;
use crate::io::storage::Storage;
use crate::io::storage::StorageExt;
use crate::lineage::Home;
use crate::lineage::LineageHook;
use crate::lineage::PackageLineage;
use crate::paths;
use crate::paths::DomainPaths;
use quilt_uri::Namespace;

/// The `data.json` of the domain at `paths`.
pub(crate) struct DataJson<'a> {
    paths: &'a DomainPaths,
    hook: Option<&'a LineageHook>,
}

impl<'a> DataJson<'a> {
    pub(crate) fn new(paths: &'a DomainPaths, hook: Option<&'a LineageHook>) -> Self {
        Self { paths, hook }
    }

    fn path(&self) -> PathBuf {
        self.paths.lineage()
    }

    pub(crate) async fn home(&self, storage: &(impl Storage + Sync)) -> Res<Home> {
        super::home(storage, &self.path()).await
    }

    pub(crate) async fn stored_home(&self, storage: &(impl Storage + Sync)) -> Res<Option<Home>> {
        super::stored_home(storage, &self.path()).await
    }

    /// Shows `look` every entry, then hands `finish` every namespace, all
    /// under the short lock.
    pub(crate) async fn check_each<T>(
        &self,
        storage: &(impl Storage + Sync),
        mut look: impl FnMut(&Namespace, &PackageLineage),
        finish: impl FnOnce(Option<&BTreeSet<Namespace>>) -> T,
    ) -> Res<T> {
        let _locked = storage.lock_exclusive(self.paths.lineage_lock()).await?;
        let lineage = self.read(storage).await?;
        for (namespace, entry) in &lineage.packages {
            look(namespace, entry);
        }
        Ok(finish(Some(&lineage.packages.into_keys().collect())))
    }

    pub(crate) async fn namespaces(
        &self,
        storage: &(impl Storage + Sync),
    ) -> Res<BTreeSet<Namespace>> {
        Ok(self.read(storage).await?.packages.into_keys().collect())
    }

    pub(crate) async fn contains(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
    ) -> Res<bool> {
        Ok(self.read(storage).await?.packages.contains_key(namespace))
    }

    pub(crate) async fn read_packages(
        &self,
        storage: &(impl Storage + Sync),
    ) -> Res<BTreeMap<Namespace, Res<PackageLineage>>> {
        #[derive(Deserialize)]
        struct Entries {
            #[serde(default)]
            packages: BTreeMap<Namespace, serde_json::Value>,
            #[serde(default)]
            home: Home,
        }

        let bytes = read_bytes(storage, &self.path()).await?;
        let entries = parse_record(&bytes, |e: &Entries| &e.home)?;
        Ok(entries
            .packages
            .into_iter()
            .map(|(namespace, entry)| {
                let entry = serde_json::from_value(entry)
                    .map_err(|err| Error::Lineage(LineageError::Parse(err)));
                (namespace, entry)
            })
            .collect())
    }

    pub(crate) async fn read_package_lineage(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
    ) -> Res<(PathBuf, PackageLineage)> {
        let domain_lineage = self.read(storage).await?;

        match domain_lineage.packages.get(namespace) {
            Some(package_lineage) => {
                let package_home = paths::package_home(&domain_lineage.home, namespace);
                Ok((package_home, package_lineage.clone()))
            }
            None => Err(Error::InstallPackage(InstallPackageError::NotInstalled(
                namespace.clone(),
            ))),
        }
    }

    /// Changes `namespace`'s entry: `step` sees `None` when the package is
    /// not installed, and setting it to `None` removes the entry.
    pub(crate) async fn update_entry<T>(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
        step: impl FnOnce(&mut Option<PackageLineage>) -> Res<T>,
    ) -> Res<T> {
        self.update(storage, |lineage| {
            let mut entry = lineage.packages.remove(namespace);
            let out = step(&mut entry);
            if let Some(entry) = entry {
                lineage.packages.insert(namespace.clone(), entry);
            }
            out
        })
        .await
    }

    /// **The short lock.** Under `data.json.lock`: re-read the whole record,
    /// let `step` change it, and write it back if it changed. An error from
    /// `step` writes nothing.
    ///
    /// The file holds every package's entry, so this is where writers of
    /// different packages meet. A writer of one package takes it inside that
    /// package's lock.
    async fn update<T>(
        &self,
        storage: &(impl Storage + Sync),
        step: impl FnOnce(&mut DomainLineage) -> Res<T>,
    ) -> Res<T> {
        let _locked = storage.lock_exclusive(self.paths.lineage_lock()).await?;
        let mut lineage = self.read(storage).await?;
        let before = lineage.clone();
        let out = step(&mut lineage)?;
        if lineage != before {
            self.write_unlocked(storage, &lineage).await?;
            self.tell(&before, &lineage);
        }
        Ok(out)
    }

    /// Read, check and write `data.json` under the short lock, so a package
    /// writer's splice can't land between the check and the write. A record
    /// whose only fault is a missing home is repaired.
    pub(crate) async fn change_home(
        &self,
        storage: &(impl Storage + Sync),
        home: impl AsRef<Path>,
        overwrite: bool,
    ) -> Res<Home> {
        let path = self.path();
        let _locked = storage.lock_exclusive(self.paths.lineage_lock()).await?;
        match storage.read_bytes(&path).await {
            Ok(bytes) => {
                let (mut lineage, home_missing) = match DomainLineage::from_slice(&bytes) {
                    Ok(lineage) => (lineage, false),
                    Err(Error::Lineage(LineageError::MissingHome)) => {
                        (serde_json::from_slice(&bytes)?, true)
                    }
                    Err(err) => return Err(err),
                };
                let home = Home::from(home);
                if lineage.home == home {
                    storage.create_dir_all(home.as_ref()).await?;
                    return Ok(home);
                }
                if !overwrite && !home_missing && !lineage.packages.is_empty() {
                    return Err(Error::Lineage(LineageError::HomeInUse {
                        home: lineage.home.as_ref().clone(),
                        installed: lineage.packages.len(),
                    }));
                }
                storage.create_dir_all(home.as_ref()).await?;
                lineage.home = home;
                self.write_unlocked(storage, &lineage).await?;
                Ok(lineage.home)
            }
            Err(_) if !storage.exists(&path).await => {
                storage.create_dir_all(home.as_ref()).await?;
                let lineage = DomainLineage::new(home);
                self.write_unlocked(storage, &lineage).await?;
                Ok(lineage.home)
            }
            Err(e) => Err(e),
        }
    }

    pub(crate) async fn read(&self, storage: &(impl Storage + Sync)) -> Res<DomainLineage> {
        DomainLineage::from_slice(&read_bytes(storage, &self.path()).await?)
    }

    /// Replace the whole record, under the short lock.
    pub(crate) async fn write(
        &self,
        storage: &(impl Storage + Sync),
        lineage: DomainLineage,
    ) -> Res<DomainLineage> {
        let _locked = storage.lock_exclusive(self.paths.lineage_lock()).await?;
        let before = if self.hook.is_some() {
            self.read(storage).await.unwrap_or_default()
        } else {
            DomainLineage::default()
        };
        self.write_unlocked(storage, &lineage).await?;
        self.tell(&before, &lineage);
        Ok(lineage)
    }

    async fn write_unlocked(
        &self,
        storage: &(impl Storage + Sync),
        lineage: &DomainLineage,
    ) -> Res {
        let contents = serde_json::to_string_pretty(lineage)?;
        storage
            .write_byte_stream_via(
                self.path(),
                crate::io::storage::temp_name(),
                contents.into_bytes().into(),
                Some("data.json"),
            )
            .await
    }

    /// Leave `namespace`'s entry one no reader can parse.
    pub(crate) async fn damage(
        &self,
        storage: &(impl Storage + Sync),
        namespace: &Namespace,
    ) -> Res {
        let path = self.path();
        let _locked = storage.lock_exclusive(self.paths.lineage_lock()).await?;
        let mut record: serde_json::Value =
            serde_json::from_slice(&read_bytes(storage, &path).await?)?;
        record["packages"][namespace.to_string()] = serde_json::json!({ "paths": "damaged" });
        storage
            .write_byte_stream(path, serde_json::to_vec(&record)?.into())
            .await
    }

    /// Tell the hook of each package entry that differs from `before` to
    /// `after`.
    fn tell(&self, before: &DomainLineage, after: &DomainLineage) {
        let Some(LineageHook(hook)) = self.hook else {
            return;
        };
        let namespaces: BTreeSet<&Namespace> = before
            .packages
            .keys()
            .chain(after.packages.keys())
            .collect();
        for namespace in namespaces {
            let was = before.packages.get(namespace);
            let is = after.packages.get(namespace);
            if was != is {
                hook(namespace, was, is);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    use test_log::test;

    use aws_sdk_s3::primitives::ByteStream;
    use aws_smithy_types::base64;

    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::CommitState;
    use crate::lineage::DomainLineageIo;
    use crate::lineage::PathState;
    use crate::lineage::SyncScope;
    use crate::object_hash::Sha256ChunkedHash;
    use quilt_uri::ManifestUri;

    fn paths() -> DomainPaths {
        DomainPaths::new(PathBuf::from("domain"))
    }

    /// A handle on the `data.json` of the domain at [`paths`].
    fn data_json() -> DomainLineageIo {
        DomainLineageIo::new(paths()).into_data_json()
    }

    #[test(tokio::test)]
    async fn test_domain_lineage_from_file() -> Res {
        let storage = MockStorage::default();
        let file_path = paths().lineage();
        storage
            .write_byte_stream(
                &file_path,
                ByteStream::from_static(br#"{"packages":{},"home":"/home/directory"}"#),
            )
            .await?;
        let lineage = data_json().legacy().read(&storage).await?;
        assert_eq!(lineage, DomainLineage::new("/home/directory"));
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_set_home_repairs_existing_lineage_with_missing_home() -> Res {
        let storage = MockStorage::default();
        let file_path = paths().lineage();
        storage
            .write_byte_stream(
                &file_path,
                ByteStream::from_static(br#"{"packages":{},"home":""}"#),
            )
            .await?;

        let io = data_json();
        let home = io.set_home(&storage, "/home/directory").await?;

        assert_eq!(home, Home::from("/home/directory"));
        assert_eq!(
            io.legacy().read(&storage).await.ok(),
            Some(DomainLineage::new("/home/directory"))
        );
        Ok(())
    }

    /// The stored bytes are compact JSON and `write` pretty-prints, so any
    /// rewrite would change them.
    #[test(tokio::test)]
    async fn set_home_to_the_current_home_writes_nothing() -> Res {
        let storage = MockStorage::default();
        let file_path = paths().lineage();
        let stored = br#"{"packages":{},"home":"/home/directory"}"#;
        storage
            .write_byte_stream(&file_path, ByteStream::from_static(stored))
            .await?;

        let home = data_json().set_home(&storage, "/home/directory").await?;

        assert_eq!(home, Home::from("/home/directory"));
        assert_eq!(storage.read_bytes(&file_path).await?, stored.to_vec());
        assert!(
            storage.exists("/home/directory").await,
            "a missing folder is still created"
        );
        Ok(())
    }

    const ONE_INSTALLED: &[u8] =
        br#"{"packages":{"foo/bar":{"commit":null,"base_hash":"","latest_hash":"","paths":{}}},"home":"/old/home"}"#;

    #[test(tokio::test)]
    async fn set_home_refuses_a_new_home_while_packages_are_installed() -> Res {
        let storage = MockStorage::default();
        let file_path = paths().lineage();
        storage
            .write_byte_stream(&file_path, ByteStream::from_static(ONE_INSTALLED))
            .await?;

        let err = data_json()
            .set_home(&storage, "/new/home")
            .await
            .unwrap_err();

        assert!(
            matches!(
                &err,
                Error::Lineage(LineageError::HomeInUse { home, installed: 1 })
                    if home == &PathBuf::from("/old/home")
            ),
            "{err:?}"
        );
        assert_eq!(
            err.to_string(),
            "Cannot change the home: 1 package is installed in /old/home"
        );
        assert_eq!(
            storage.read_bytes(&file_path).await?,
            ONE_INSTALLED.to_vec()
        );
        assert!(!storage.exists("/new/home").await, "no folder is created");
        Ok(())
    }

    #[test(tokio::test)]
    async fn set_home_to_the_current_home_is_allowed_while_packages_are_installed() -> Res {
        let storage = MockStorage::default();
        let file_path = paths().lineage();
        storage
            .write_byte_stream(&file_path, ByteStream::from_static(ONE_INSTALLED))
            .await?;

        let home = data_json().set_home(&storage, "/old/home").await?;

        assert_eq!(home, Home::from("/old/home"));
        Ok(())
    }

    #[test(tokio::test)]
    async fn overwrite_home_changes_the_home_while_packages_are_installed() -> Res {
        let storage = MockStorage::default();
        let file_path = paths().lineage();
        storage
            .write_byte_stream(&file_path, ByteStream::from_static(ONE_INSTALLED))
            .await?;
        let io = data_json();

        io.overwrite_home(&storage, "/new/home").await?;

        let lineage = io.legacy().read(&storage).await?;
        assert_eq!(lineage.home, Home::from("/new/home"));
        assert_eq!(lineage.namespaces(), vec![Namespace::from(("foo", "bar"))]);
        Ok(())
    }

    #[test(tokio::test)]
    async fn set_home_creates_the_home_folder() -> Res {
        let storage = MockStorage::default();

        data_json().set_home(&storage, "/new/home").await?;

        assert!(storage.exists("/new/home").await);
        Ok(())
    }

    #[test(tokio::test)]
    async fn changing_the_home_creates_the_new_folder() -> Res {
        let storage = MockStorage::default();
        let file_path = paths().lineage();
        storage
            .write_byte_stream(&file_path, ByteStream::from_static(ONE_INSTALLED))
            .await?;

        data_json().overwrite_home(&storage, "/new/home").await?;

        assert!(storage.exists("/new/home").await);
        Ok(())
    }

    /// A file where the home's parent should be makes the folder impossible
    /// to create.
    #[test(tokio::test)]
    async fn a_home_that_cannot_be_created_is_not_stored() -> Res {
        let storage = MockStorage::default();
        storage
            .write_byte_stream("/blocked", ByteStream::from_static(b"a file"))
            .await?;
        let file_path = paths().lineage();
        storage
            .write_byte_stream(&file_path, ByteStream::from_static(ONE_INSTALLED))
            .await?;

        data_json()
            .overwrite_home(&storage, "/blocked/home")
            .await
            .unwrap_err();

        assert_eq!(
            storage.read_bytes(&file_path).await?,
            ONE_INSTALLED.to_vec()
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_first_home_that_cannot_be_created_writes_no_record() -> Res {
        let storage = MockStorage::default();
        storage
            .write_byte_stream("/blocked", ByteStream::from_static(b"a file"))
            .await?;
        let file_path = paths().lineage();

        data_json()
            .set_home(&storage, "/blocked/home")
            .await
            .unwrap_err();

        assert!(!storage.exists(&file_path).await);
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_domain_lineage_from_nothing() -> Res {
        let storage = MockStorage::default();
        let lineage = data_json().legacy().read(&storage).await.unwrap_err();
        assert!(matches!(lineage, Error::Lineage(LineageError::Missing)));
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_domain_lineage_write() -> Res {
        let storage = MockStorage::default();
        let file_path = paths().lineage();
        assert!(!storage.exists(&file_path).await);
        let bytes = "0123456789abcdef".as_bytes();
        let working_dir = PathBuf::from("/tmp/working_dir");
        data_json()
            .legacy()
            .write(
                &storage,
                DomainLineage {
                    home: Home::new(working_dir),
                    packages: BTreeMap::from([(
                        ("foo", "bar").into(),
                        PackageLineage {
                            commit: None,
                            remote_uri: Some(ManifestUri {
                                bucket: "bucket".to_string(),
                                namespace: ("foo", "bar").into(),
                                hash: "abcdef0000000000000000000000000000000000000000000000000000000000".to_string(),
                                origin: None,
                            }),
                            base_hash: "abcdef0000000000000000000000000000000000000000000000000000000000".to_string(),
                            latest_hash: "abcdef0000000000000000000000000000000000000000000000000000000000".to_string(),
                            paths: BTreeMap::from([(
                                PathBuf::from("foo"),
                                PathState {
                                    timestamp: jiff::Timestamp::from_millisecond(
                                        1_737_031_820_534,
                                    )
                                    .unwrap(),
                                    hash: Some(
                                        Sha256ChunkedHash::from_async_read(
                                            bytes,
                                            bytes.len() as u64,
                                        )
                                        .await?
                                        .into(),
                                    ),
                                },
                            )]),
                            ..PackageLineage::default()
                        },
                    )]),
                },
            )
            .await?;
        assert!(storage.exists(&file_path).await);
        let file_contents = storage.read_bytes(&file_path).await?;
        let lineage = DomainLineage::from_slice(&file_contents)?;

        assert_eq!(lineage.as_ref(), &PathBuf::from("/tmp/working_dir"));

        let hash_from_lineage = lineage
            .packages
            .get(&(("foo".to_string(), "bar".to_string()).into()))
            .unwrap()
            .paths
            .get(&PathBuf::from("foo"))
            .unwrap()
            .hash
            .map(|hash| base64::encode(hash.digest()));
        assert_eq!(
            hash_from_lineage.as_deref(),
            Some("Xb1PbjJeWof4zD7zuHc9PI7sLiz/Ykj4gphlaZEt3xA=")
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_read_package_lineage() -> Res {
        let storage = MockStorage::default();
        let namespace = Namespace::from(("foo", "bar"));
        let package_lineage = PackageLineage {
            commit: None,
            remote_uri: Some(ManifestUri {
                bucket: "bucket".to_string(),
                namespace: namespace.clone(),
                hash: "abcdef0000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
                origin: None,
            }),
            base_hash: "abcdef0000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            latest_hash: "abcdef0000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            paths: BTreeMap::new(),
            ..PackageLineage::default()
        };

        // Create a domain lineage with a package
        let lineage = DomainLineage {
            home: Home::from("/home/user/quilt"),
            packages: BTreeMap::from([(namespace.clone(), package_lineage.clone())]),
        };

        // Write it to storage
        let lineage_io = data_json();
        lineage_io.legacy().write(&storage, lineage).await?;

        // Read the package lineage
        let (package_home, read_package_lineage) = lineage_io
            .read_package_lineage(&storage, &namespace)
            .await?;

        // Verify the results
        assert_eq!(package_home, PathBuf::from("/home/user/quilt/foo/bar"));
        assert_eq!(read_package_lineage, package_lineage);

        // Try reading a non-existent package
        let non_existent = Namespace::from(("does", "notexist"));
        let result = lineage_io
            .read_package_lineage(&storage, &non_existent)
            .await;
        assert!(matches!(
            result.unwrap_err(),
            Error::InstallPackage(InstallPackageError::NotInstalled(ns)) if ns == non_existent
        ));

        Ok(())
    }

    /// An entry that does not parse costs that package alone.
    #[test(tokio::test)]
    async fn read_packages_reads_past_an_entry_that_does_not_parse() -> Res {
        let storage = MockStorage::default();
        let file_path = paths().lineage();
        storage
            .write_byte_stream(
                &file_path,
                ByteStream::from_static(
                    br#"{"packages":{
                        "foo/good":{"commit":null,"base_hash":"b","latest_hash":"","paths":{}},
                        "foo/bad":{"commit":null,"base_hash":"","latest_hash":"","paths":"damaged"}
                    },"home":"/home/directory"}"#,
                ),
            )
            .await?;

        let packages = data_json().read_packages(&storage).await;

        let Ok(packages) = packages else {
            panic!("one entry failed the whole list: {packages:?}");
        };
        let good = Namespace::from(("foo", "good"));
        let bad = Namespace::from(("foo", "bad"));
        assert_eq!(packages.keys().collect::<Vec<_>>(), [&bad, &good]);
        assert_eq!(
            packages[&good].as_ref().ok().map(|l| l.base_hash.as_str()),
            Some("b")
        );
        assert!(
            matches!(packages[&bad], Err(Error::Lineage(LineageError::Parse(_)))),
            "{:?}",
            packages[&bad]
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_update_package_lineage() -> Res {
        let storage = MockStorage::default();
        let lineage_io = data_json();

        // Create an initial domain lineage with home directory
        let initial_lineage = DomainLineage {
            home: Home::from("/home/user/quilt"),
            packages: BTreeMap::new(),
        };

        // Write the initial lineage
        lineage_io.legacy().write(&storage, initial_lineage).await?;

        // Create a package lineage to write
        let namespace = Namespace::from(("foo", "bar"));
        let package_lineage = PackageLineage {
            commit: None,
            remote_uri: Some(ManifestUri {
                bucket: "bucket".to_string(),
                namespace: namespace.clone(),
                hash: "abcdef0000000000000000000000000000000000000000000000000000000000"
                    .to_string(),
                origin: None,
            }),
            base_hash: "abcdef0000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            latest_hash: "abcdef0000000000000000000000000000000000000000000000000000000000"
                .to_string(),
            paths: BTreeMap::new(),
            ..PackageLineage::default()
        };

        // Write the package lineage
        let written_lineage = lineage_io
            .update_package_lineage(&storage, &namespace, |entry| {
                *entry = Some(package_lineage.clone());
                Ok(package_lineage.clone())
            })
            .await?;

        // Verify the written lineage matches what we provided
        assert_eq!(written_lineage, package_lineage);

        // Read the domain lineage to verify the package was added
        let domain_lineage = lineage_io.legacy().read(&storage).await?;
        assert_eq!(domain_lineage.packages.len(), 1);
        assert!(domain_lineage.packages.contains_key(&namespace));
        assert_eq!(
            domain_lineage.packages.get(&namespace).unwrap(),
            &package_lineage
        );

        // Update the package lineage
        let updated_package_lineage = PackageLineage {
            commit: Some(CommitState {
                timestamp: jiff::Timestamp::now(),
                hash: String::new(),
                prev_hashes: Vec::new(),
            }),
            ..package_lineage.clone()
        };

        // Write the updated package lineage
        lineage_io
            .update_package_lineage(&storage, &namespace, |entry| {
                *entry = Some(updated_package_lineage.clone());
                Ok(())
            })
            .await?;

        // Read the domain lineage again to verify the update
        let updated_domain_lineage = lineage_io.legacy().read(&storage).await?;
        assert_eq!(updated_domain_lineage.packages.len(), 1);
        assert!(updated_domain_lineage.packages.contains_key(&namespace));
        assert_eq!(
            updated_domain_lineage.packages.get(&namespace).unwrap(),
            &updated_package_lineage
        );

        Ok(())
    }

    /// The narrow edit reads the domain, changes one field, and writes — so a
    /// value another writer put in the same entry *before* that read survives,
    /// which is the point of not holding a copy across other work.
    #[test(tokio::test)]
    async fn edit_package_lineage_preserves_the_rest_of_the_entry() -> Res {
        let storage = MockStorage::default();
        let namespace = Namespace::from(("foo", "bar"));
        let lineage_io = data_json();
        lineage_io
            .legacy()
            .write(
                &storage,
                DomainLineage {
                    home: Home::from("/home/user/quilt"),
                    packages: BTreeMap::from([(namespace.clone(), PackageLineage::default())]),
                },
            )
            .await?;

        // Stand in for a concurrent operation that advanced the entry.
        lineage_io
            .edit_package_lineage(&storage, &namespace, |l| {
                l.base_hash = "advanced".to_string();
            })
            .await?;

        let updated = lineage_io
            .edit_package_lineage(&storage, &namespace, |l| {
                l.sync_scope = SyncScope::EntirePackage;
            })
            .await?;

        assert_eq!(updated.sync_scope, SyncScope::EntirePackage);
        assert_eq!(
            updated.base_hash, "advanced",
            "the scope write must not roll back the other writer"
        );
        Ok(())
    }

    /// A package that is not installed is an error, not a silent insert.
    #[test(tokio::test)]
    async fn edit_package_lineage_refuses_an_unknown_namespace() -> Res {
        let storage = MockStorage::default();
        let lineage_io = data_json();
        lineage_io
            .legacy()
            .write(&storage, DomainLineage::new("/home/user/quilt"))
            .await?;

        let err = lineage_io
            .edit_package_lineage(&storage, &Namespace::from(("no", "such")), |l| {
                l.sync_scope = SyncScope::EntirePackage;
            })
            .await
            .unwrap_err();
        assert!(matches!(
            err,
            Error::InstallPackage(InstallPackageError::NotInstalled(_))
        ));
        Ok(())
    }

    type Told = std::sync::Mutex<Vec<(Namespace, Option<String>, Option<String>)>>;

    /// A handle whose hook notes each change as the base hashes before and
    /// after.
    fn hooked() -> (DomainLineageIo, Arc<Told>) {
        let told = Arc::new(Told::default());
        let noted = Arc::clone(&told);
        let io = data_json().with_hook(LineageHook::new(move |namespace, was, is| {
            noted.lock().unwrap().push((
                namespace.clone(),
                was.map(|l| l.base_hash.clone()),
                is.map(|l| l.base_hash.clone()),
            ));
        }));
        (io, told)
    }

    #[test(tokio::test)]
    async fn the_hook_hears_each_changed_entry_once() -> Res {
        let storage = MockStorage::default();
        let (io, told) = hooked();
        let foo = Namespace::from(("foo", "bar"));
        let other = Namespace::from(("other", "pkg"));
        let entry = |hash: &str| PackageLineage {
            base_hash: hash.to_string(),
            ..PackageLineage::default()
        };
        io.legacy()
            .write(
                &storage,
                DomainLineage {
                    home: Home::from("/home/user/quilt"),
                    packages: BTreeMap::from([(other.clone(), entry("o"))]),
                },
            )
            .await?;
        io.update_package_lineage(&storage, &foo, |e| {
            *e = Some(entry("a"));
            Ok(())
        })
        .await?;
        io.edit_package_lineage(&storage, &foo, |l| l.base_hash = "b".to_string())
            .await?;
        // A write that changes nothing is not told.
        io.edit_package_lineage(&storage, &foo, |l| l.base_hash = "b".to_string())
            .await?;
        io.update_package_lineage(&storage, &foo, |e| {
            *e = None;
            Ok(())
        })
        .await?;

        let some = |s: &str| Some(s.to_string());
        assert_eq!(
            *told.lock().unwrap(),
            vec![
                (other, None, some("o")),
                (foo.clone(), None, some("a")),
                (foo.clone(), some("a"), some("b")),
                (foo, some("b"), None),
            ]
        );
        Ok(())
    }

    /// A handle without the hook, as another process writes, tells nothing.
    #[test(tokio::test)]
    async fn a_write_without_the_hook_is_not_told() -> Res {
        let storage = MockStorage::default();
        let (io, told) = hooked();
        io.legacy()
            .write(&storage, DomainLineage::new("/home/user/quilt"))
            .await?;
        let foo = Namespace::from(("foo", "bar"));
        data_json()
            .update_package_lineage(&storage, &foo, |e| {
                *e = Some(PackageLineage::default());
                Ok(())
            })
            .await?;
        assert!(io.contains(&storage, &foo).await?);
        assert!(told.lock().unwrap().is_empty());
        Ok(())
    }
}
