//! A short top hash, resolved to the whole one among the revisions a package
//! holds locally: quilt3's `Package.resolve_hash` on the local registry, with
//! [`resolve_hash`](super::resolve_hash)'s rules and errors.
//!
//! A prefix is matched against the names of the package's installed
//! manifests, with no manifest read. Only a regular file named by a whole top
//! hash is a revision, so a stranded `.tmp-<uuid>`, a folder or a symlink is
//! not. A manifests folder that is not there holds no revisions.
//!
//! Unlike quilt3's V1 local registry, which matched across every package, only
//! this package's revisions count, so a prefix another package shares is not
//! ambiguous.
//!
//! The flow takes the manifests folder, not the package's name, so it does not
//! depend on how a domain keys its packages.

use std::path::Path;

use quilt_uri::is_top_hash;

use super::resolve_hash::check_prefix;
use super::resolve_hash::pick_one;
use crate::Res;
use crate::io::storage::Storage;

/// The one revision among the manifests in `manifests_dir` whose top hash
/// starts with `prefix`.
///
/// `manifests_dir` is the folder holding one package's installed manifests,
/// each named by its top hash. See the module docs for the rules. Reads only:
/// it writes and creates nothing, not even the folder.
///
/// # Errors
///
/// The same [`HashPrefixError`](crate::error::HashPrefixError)s as
/// [`resolve_hash`](super::resolve_hash), and whatever listing the folder
/// fails with.
pub async fn resolve_local_hash(
    storage: &(impl Storage + Sync),
    manifests_dir: impl AsRef<Path> + Send + Sync,
    prefix: &str,
) -> Res<String> {
    if let Some(whole) = check_prefix(prefix)? {
        return Ok(whole);
    }
    let found = revision_names_with_prefix(storage, manifests_dir.as_ref(), prefix).await?;
    Ok(pick_one(prefix, found)?)
}

/// The names of the regular files in `manifests_dir` that are a whole top hash
/// starting with `prefix`; none when the folder is not there, and an error when
/// it cannot be read.
async fn revision_names_with_prefix(
    storage: &(impl Storage + Sync),
    manifests_dir: &Path,
    prefix: &str,
) -> Res<Vec<String>> {
    // Not `exists`, which is also false for a folder that cannot be read.
    let mut entries = match storage.read_dir(manifests_dir).await {
        Ok(entries) => entries,
        Err(err) if err.is_not_found() => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut found = Vec::new();
    while let Some(entry) = entries.next_entry().await? {
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !name.starts_with(prefix) || !is_top_hash(&name) {
            continue;
        }
        if entry.file_type().await?.is_file() {
            found.push(name);
        }
    }
    Ok(found)
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use tempfile::TempDir;
    use test_log::test;

    use quilt_uri::Namespace;

    use crate::Error;
    use crate::error::HashPrefixError;
    use crate::flow::MAX_LISTED_MATCHES;
    use crate::flow::MIN_HASH_PREFIX_LEN;
    use crate::flow::TOP_HASH_LEN;
    use crate::flow::UserMeta;
    use crate::flow::create;
    use crate::flow::list_revisions;
    use crate::io::storage::LocalStorage;
    use crate::lineage::Home;
    use crate::paths::DomainPaths;

    /// Two revisions that share their first eight characters.
    const ALPHA: &str = "abcdef01aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const BETA: &str = "abcdef01bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";
    const GAMMA: &str = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";

    fn namespace() -> Namespace {
        ("team", "data").into()
    }

    /// A domain whose `namespace` holds a manifest file for each of `hashes`.
    async fn domain_with(
        namespace: &Namespace,
        hashes: &[&str],
    ) -> Res<(DomainPaths, PathBuf, TempDir)> {
        let (paths, temp_dir) = DomainPaths::from_temp_dir()?;
        let dir = paths.installed_manifests_dir(namespace);
        tokio::fs::create_dir_all(&dir).await?;
        for hash in hashes {
            tokio::fs::write(dir.join(hash), b"{}").await?;
        }
        Ok((paths, dir, temp_dir))
    }

    fn prefix_error(err: Error) -> HashPrefixError {
        match err {
            Error::HashPrefix(inner) => inner,
            other => panic!("expected a hash prefix error, got {other:?}"),
        }
    }

    #[test(tokio::test)]
    async fn a_unique_prefix_resolves_to_the_whole_hash() -> Res {
        let (_paths, dir, _temp_dir) = domain_with(&namespace(), &[ALPHA, BETA, GAMMA]).await?;
        let storage = LocalStorage::new();

        assert_eq!(resolve_local_hash(&storage, &dir, "012345").await?, GAMMA);
        assert_eq!(
            resolve_local_hash(&storage, &dir, &GAMMA[..MIN_HASH_PREFIX_LEN]).await?,
            GAMMA
        );
        // Long enough to tell the two that share eight characters apart.
        assert_eq!(resolve_local_hash(&storage, &dir, "abcdef01b").await?, BETA);
        assert_eq!(
            resolve_local_hash(&storage, &dir, &ALPHA[..TOP_HASH_LEN - 1]).await?,
            ALPHA
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_prefix_two_revisions_share_is_ambiguous_and_names_them() -> Res {
        let (_paths, dir, _temp_dir) = domain_with(&namespace(), &[ALPHA, BETA, GAMMA]).await?;

        let err = resolve_local_hash(&LocalStorage::new(), &dir, "abcdef")
            .await
            .unwrap_err();
        assert_eq!(
            prefix_error(err),
            HashPrefixError::Ambiguous {
                prefix: "abcdef".to_string(),
                matches: vec![ALPHA.to_string(), BETA.to_string()],
                more: 0,
            }
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_prefix_many_revisions_share_lists_only_the_first_few() -> Res {
        let total = MAX_LISTED_MATCHES + 3;
        let hashes: Vec<String> = (0..total).map(|n| format!("abcdef{n:058x}")).collect();
        let (_paths, dir, _temp_dir) = domain_with(
            &namespace(),
            &hashes.iter().map(String::as_str).collect::<Vec<_>>(),
        )
        .await?;

        let err = resolve_local_hash(&LocalStorage::new(), &dir, "abcdef")
            .await
            .unwrap_err();
        let HashPrefixError::Ambiguous { matches, more, .. } = prefix_error(err) else {
            panic!("expected Ambiguous");
        };
        assert_eq!(matches, hashes[..MAX_LISTED_MATCHES]);
        assert_eq!(more, 3);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_prefix_nothing_starts_with_matches_zero() -> Res {
        let (_paths, dir, _temp_dir) = domain_with(&namespace(), &[ALPHA, GAMMA]).await?;

        let err = resolve_local_hash(&LocalStorage::new(), &dir, "123456")
            .await
            .unwrap_err();
        assert_eq!(
            prefix_error(err),
            HashPrefixError::NoMatch("123456".to_string())
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn an_empty_manifests_folder_matches_zero() -> Res {
        let (_paths, dir, _temp_dir) = domain_with(&namespace(), &[]).await?;

        let err = resolve_local_hash(&LocalStorage::new(), &dir, "abcdef")
            .await
            .unwrap_err();
        assert_eq!(
            prefix_error(err),
            HashPrefixError::NoMatch("abcdef".to_string())
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_package_with_no_manifests_folder_matches_zero() -> Res {
        let (paths, _dir, _temp_dir) = domain_with(&namespace(), &[ALPHA]).await?;
        let absent = paths.installed_manifests_dir(&("team", "other").into());

        let err = resolve_local_hash(&LocalStorage::new(), &absent, "abcdef")
            .await
            .unwrap_err();
        assert!(matches!(prefix_error(err), HashPrefixError::NoMatch(_)));
        assert!(!absent.exists());
        Ok(())
    }

    /// A file in the folder's place: listing it fails as not a folder on every
    /// platform, where a file in a parent's place reads as not found on
    /// Windows.
    #[test(tokio::test)]
    async fn a_manifests_folder_that_cannot_be_read_is_an_error() -> Res {
        let (paths, _dir, _temp_dir) = domain_with(&namespace(), &[ALPHA]).await?;
        let blocked = paths.installed_dir().join("blocked");
        tokio::fs::write(&blocked, b"not a folder").await?;

        let err = resolve_local_hash(&LocalStorage::new(), &blocked, "abcdef")
            .await
            .unwrap_err();
        assert!(!matches!(err, Error::HashPrefix(_)), "{err:?}");
        Ok(())
    }

    /// quilt3 returns a 64-character hash without looking; so does this, even
    /// for a package that does not hold it.
    #[test(tokio::test)]
    async fn a_whole_hash_comes_back_as_given_even_if_no_revision_has_it() -> Res {
        let (_paths, dir, _temp_dir) = domain_with(&namespace(), &[ALPHA]).await?;
        let storage = LocalStorage::new();

        assert_eq!(resolve_local_hash(&storage, &dir, ALPHA).await?, ALPHA);
        assert_eq!(resolve_local_hash(&storage, &dir, GAMMA).await?, GAMMA);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_prefix_of_the_wrong_length_or_not_hex_is_refused() -> Res {
        let (_paths, dir, _temp_dir) = domain_with(&namespace(), &[ALPHA]).await?;
        let storage = LocalStorage::new();

        for bad in ["", "abcde", &format!("{ALPHA}0")] {
            let err = resolve_local_hash(&storage, &dir, bad).await.unwrap_err();
            assert_eq!(
                prefix_error(err),
                HashPrefixError::InvalidLength(bad.to_string()),
                "{bad:?}"
            );
        }
        for bad in ["ABCDEF", "abcde/", "zzzzzz", &ALPHA.to_uppercase()] {
            let err = resolve_local_hash(&storage, &dir, bad).await.unwrap_err();
            assert_eq!(
                prefix_error(err),
                HashPrefixError::NotHex(bad.to_string()),
                "{bad:?}"
            );
        }
        Ok(())
    }

    #[test(tokio::test)]
    async fn only_whole_hash_files_are_revisions() -> Res {
        let (_paths, dir, _temp_dir) = domain_with(&namespace(), &[ALPHA]).await?;
        for stray in ["abcdef01-notes.txt", ".tmp-abcdef01", "abcdef.tmp"] {
            tokio::fs::write(dir.join(stray), b"x").await?;
        }
        // A folder named like a revision is not one.
        tokio::fs::create_dir(dir.join(BETA)).await?;
        // Nor is a symlink, even to a manifest.
        #[cfg(unix)]
        tokio::fs::symlink(dir.join(ALPHA), dir.join(format!("abcdef02{}", &BETA[8..]))).await?;

        assert_eq!(
            resolve_local_hash(&LocalStorage::new(), &dir, "abcdef").await?,
            ALPHA
        );
        Ok(())
    }

    /// quilt3's V1 local registry matched across packages; this does not.
    #[test(tokio::test)]
    async fn another_packages_revisions_do_not_count() -> Res {
        let (paths, dir, _temp_dir) = domain_with(&namespace(), &[ALPHA]).await?;
        let other = paths.installed_manifests_dir(&("team", "other").into());
        tokio::fs::create_dir_all(&other).await?;
        tokio::fs::write(other.join(BETA), b"{}").await?;
        tokio::fs::write(other.join(GAMMA), b"{}").await?;
        let storage = LocalStorage::new();

        assert_eq!(resolve_local_hash(&storage, &dir, "abcdef").await?, ALPHA);
        let err = resolve_local_hash(&storage, &dir, "012345")
            .await
            .unwrap_err();
        assert!(matches!(prefix_error(err), HashPrefixError::NoMatch(_)));
        assert_eq!(resolve_local_hash(&storage, &other, "abcdef").await?, BETA);
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_local_only_package_resolves_its_revision() -> Res {
        let (home, _home_dir) = Home::from_temp_dir()?;
        let (paths, _temp_dir) = DomainPaths::from_temp_dir()?;
        let storage = LocalStorage::new();
        let namespace = namespace();
        create(
            &home,
            false,
            &paths,
            &storage,
            namespace.clone(),
            None,
            Some("first".to_string()),
            UserMeta::Clear,
        )
        .await?;
        let revisions = list_revisions(&paths, &storage, &namespace).await?;
        let [revision] = revisions.as_slice() else {
            panic!("expected one revision, got {revisions:?}");
        };
        let dir = paths.installed_manifests_dir(&namespace);

        let short = &revision.hash[..7];
        assert_eq!(
            resolve_local_hash(&storage, &dir, short).await?,
            revision.hash
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn the_domain_resolves_a_local_only_package_by_namespace() -> Res {
        let temp_dir = TempDir::new()?;
        let domain = crate::LocalDomain::new(temp_dir.path());
        domain.set_home(temp_dir.path()).await?;
        let namespace = namespace();
        domain
            .create_package(
                namespace.clone(),
                None,
                Some("first".to_string()),
                UserMeta::Clear,
            )
            .await?;
        let paths = DomainPaths::new(temp_dir.path().to_path_buf());
        let revisions = list_revisions(&paths, &LocalStorage::new(), &namespace).await?;
        let [revision] = revisions.as_slice() else {
            panic!("expected one revision, got {revisions:?}");
        };

        assert_eq!(
            domain
                .resolve_local_hash(&namespace, &revision.hash[..6])
                .await?,
            revision.hash
        );
        let err = domain
            .resolve_local_hash(&("team", "other").into(), &revision.hash[..6])
            .await
            .unwrap_err();
        assert!(matches!(prefix_error(err), HashPrefixError::NoMatch(_)));
        Ok(())
    }
}
