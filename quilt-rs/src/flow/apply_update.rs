use std::path::PathBuf;

use tracing::debug;

use crate::Res;
use std::collections::BTreeMap;

use crate::flow;
use crate::flow::Protect;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::paths::DomainPaths;
use crate::paths::copy_cached_to_installed;
use quilt_uri::ManifestUri;
use quilt_uri::Namespace;

/// What an apply did, which is narrower than the touch-set it was given: a
/// touched path this copy does not track is never uninstalled (no file to
/// delete), and one absent from `latest` is never installed. Both are ordinary
/// under [`EntirePackage`](crate::lineage::SyncScope::EntirePackage), whose
/// touch-set covers untracked paths — so a caller reporting the touch-set
/// states things that did not happen.
/// Whether the caller's local work is at stake in this apply.
///
/// Distinct from [`Protect`] one layer down, which carries the rows to check
/// against: those are the apply's to compute, from the manifest it is about to
/// replace. A caller only knows which of the two operations it is running.
pub(crate) enum LocalWork {
    /// A pull: it verified that every path it means to touch still holds its
    /// `base` content, and the apply re-checks that immediately before each
    /// overwrite, because staging puts a whole fetch in between.
    Protect,
    /// A reset: discarding local work is the operation.
    Discard,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Applied {
    /// Paths written from `latest` — whether or not a file was there before.
    pub installed: Vec<PathBuf>,
    /// The subset of `installed` that replaced a file this copy already
    /// tracked, as opposed to writing one where there was nothing. Recorded
    /// because the write itself no longer says which: the apply used to delete
    /// before installing, so "was also uninstalled" meant "was already here".
    pub replaced: Vec<PathBuf>,
    /// Paths deleted from the working tree and dropped from tracking.
    pub uninstalled: Vec<PathBuf>,
}

/// Apply a set of `latest`-manifest path updates to the working tree, object
/// store, lineage, and installed-manifest base — the mechanic shared by
/// gentle [`pull`](super::pull) and [`reset_to_latest`](super::reset_to_latest).
///
/// `touched` is the caller's touch-set over **tracked** paths; each keeps its
/// own touch-set (reset: every differing path; gentle pull: remote-changed
/// minus locally-changed). Paths in `touched` absent from `latest` were
/// remote-removed and stay uninstalled.
///
/// # Errors
/// Propagates uninstall / caching / install failures. The reconcile is not a
/// transaction across I/O; callers treat `pull`/`reset` as retryable.
#[allow(clippy::too_many_arguments)]
pub(crate) async fn apply_latest_update(
    mut lineage: PackageLineage,
    manifest: &mut Manifest,
    paths: &DomainPaths,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    working_dir: PathBuf,
    namespace: Namespace,
    latest: ManifestUri,
    touched: &[PathBuf],
    local_work: &LocalWork,
) -> Res<(PackageLineage, Applied)> {
    // Install first, delete last. The old order deleted every touched path and
    // then re-fetched it, so a failure in between left those paths gone from
    // the working tree AND dropped from the lineage — a retry read the gap as a
    // local delete against a remote modify and refused with `PullConflict`
    // forever, which is the incident this ordering exists to remove. Nothing
    // here is atomic across the set; what it buys is that every tracked path
    // holds `base`'s bytes or `latest`'s at every instant, which is what makes
    // an interrupted apply retryable.
    // The rows the caller's verification was taken against, captured before the
    // manifest becomes `latest`. `Protect::BaseContent` re-checks a destination
    // against these immediately before overwriting it, which is the only moment
    // that means anything once the whole touch set is staged first.
    let base_rows: BTreeMap<PathBuf, ManifestRow> = touched
        .iter()
        .filter(|path| lineage.paths.contains_key(*path))
        .filter_map(|path| {
            manifest
                .get_record(path)
                .map(|row| (path.clone(), row.clone()))
        })
        .collect();

    debug!("⏳ Advancing lineage to latest {}", latest.hash);
    lineage.remote_mut()?.hash.clone_from(&latest.hash);
    lineage.base_hash.clone_from(&latest.hash);
    lineage.latest_hash.clone_from(&latest.hash);

    debug!("⏳ Caching + installing latest manifest as the new base");
    // Cache the remote manifest for its side effect only; the parse result is
    // discarded because `*manifest` is (re)loaded from the installed copy just
    // below from a byte-identical file.
    flow::cache_remote_manifest(paths, storage, remote, &latest).await?;
    copy_cached_to_installed(
        paths,
        storage,
        &ManifestUri {
            namespace: namespace.clone(),
            ..latest.clone()
        },
    )
    .await?;
    *manifest =
        Manifest::from_path(storage, &paths.installed_manifest(&namespace, &latest.hash)).await?;
    lineage.remote_uri = Some(latest);

    // Split the touch set by presence in the new base: what `latest` still has
    // is written over, what it no longer has is deleted afterwards.
    let to_install: Vec<PathBuf> = touched
        .iter()
        .filter(|p| manifest.contains_record(p))
        .cloned()
        .collect();
    let to_uninstall: Vec<PathBuf> = touched
        .iter()
        .filter(|p| !manifest.contains_record(p))
        .filter(|p| lineage.paths.contains_key(*p))
        .cloned()
        .collect();

    // Which installs replace a file already here, read BEFORE the install adds
    // its own lineage rows. The pull report tells an update from an addition by
    // this; it used to read it from a path being both uninstalled and
    // installed, which the reorder makes always false.
    let replaced: Vec<PathBuf> = to_install
        .iter()
        .filter(|p| lineage.paths.contains_key(*p))
        .cloned()
        .collect();

    debug!(
        "⏳ Reinstalling {} touched paths present in latest",
        to_install.len()
    );
    let install_refs: Vec<&PathBuf> = to_install.iter().collect();
    // `install_paths_over`, not `install_paths`: these paths are tracked and
    // stay tracked across the write, which the user-facing verb refuses by
    // design. Safe only because the touch set already excludes every path the
    // user touched — see `pull::touch_set`.
    // `Refuse`, so nothing is ever skipped: the lineage saved on success names
    // the new base, so a path left out would be recorded at a row its working
    // file does not hold. A refusal comes before any rename, and nothing is saved.
    let (mut lineage, _skipped) = flow::install_paths_over(
        lineage,
        manifest,
        paths,
        working_dir.clone(),
        namespace,
        storage,
        remote,
        &install_refs,
        &match local_work {
            LocalWork::Protect => Protect::BaseContent(&base_rows),
            LocalWork::Discard => Protect::Nothing,
        },
        flow::OnMismatch::Refuse,
    )
    .await?;

    debug!("⏳ Uninstalling {} touched paths", to_uninstall.len());
    lineage = flow::uninstall_paths(lineage, working_dir, storage, &to_uninstall).await?;

    // Prune lineage paths that have no row in the new base manifest. This only
    // catches trivially-resolved both-removed paths (locally deleted + absent
    // from `latest`): they are filtered out of the touch-set, so they never go
    // through `uninstall_paths`, yet the persisted lineage must not track a
    // path with no manifest row — `create_status` hard-errors on that for
    // remote-backed packages. Every other case is already handled: a
    // locally-removed path the remote still has keeps its row; a
    // remote-removed + locally-modified path is classified `Blocked` and never
    // reaches apply; a remote-removed untouched path is in the touch-set and
    // uninstalled normally. For reset (touch-set = every path) this is a no-op.
    //
    // AFTER the uninstall, not before: pruning first would drop the very rows
    // `uninstall_paths` looks up, and it errors on a path whose row is gone.
    lineage
        .paths
        .retain(|path, _| manifest.contains_record(path));

    Ok((
        lineage,
        Applied {
            installed: to_install,
            replaced,
            uninstalled: to_uninstall,
        },
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use std::collections::BTreeMap;

    use aws_sdk_s3::primitives::ByteStream;

    use crate::checksum::calculate_hash;
    use crate::io::remote::HostChecksums;
    use crate::io::remote::HostConfig;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::StorageExt;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::PathState;
    use crate::manifest::ManifestRow;
    use quilt_uri::S3Uri;

    // An empty touch-set advances the hashes (`base_hash`, `latest_hash`, and
    // `remote.hash`) to `latest` and reloads the manifest cache without touching
    // any installed paths. Uninstall/reinstall mechanics are covered by the
    // callers' suites (reset_to_latest, pull, quilt-cli).
    #[test(tokio::test)]
    async fn empty_touch_set_advances_hashes() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "OLD".to_string(),
            origin: None,
        };
        let paths = DomainPaths::default();
        let storage = MockStorage::default();
        paths
            .scaffold_for_caching(&storage, &manifest_uri.bucket)
            .await?;

        let lineage = PackageLineage {
            remote_uri: Some(manifest_uri.clone()),
            base_hash: "OLD".to_string(),
            latest_hash: "NEW".to_string(),
            ..PackageLineage::default()
        };

        let new_hash = "deadbeef";
        let remote = MockRemote::default();
        remote
            .put_object(
                None,
                &S3Uri::try_from(format!("s3://b/.quilt/packages/{new_hash}").as_str())?,
                r#"{"version": "v0"}"#.as_bytes().to_vec(),
            )
            .await?;

        let latest = ManifestUri {
            hash: new_hash.to_string(),
            ..manifest_uri
        };
        let mut manifest = Manifest::default();
        let result = apply_latest_update(
            lineage,
            &mut manifest,
            &paths,
            &storage,
            &remote,
            PathBuf::default(),
            Namespace::default(),
            latest,
            &[],
            &LocalWork::Protect,
        )
        .await?;

        let (lineage, applied) = result;
        assert_eq!(lineage.base_hash, new_hash);
        assert_eq!(lineage.latest_hash, new_hash);
        assert_eq!(lineage.remote()?.hash, new_hash);
        // An empty touch-set moved nothing, and the record says so — what a
        // caller reports comes from here, not from the touch-set.
        assert_eq!(applied, Applied::default());
        Ok(())
    }

    async fn publish(remote: &MockRemote, hash: &str, manifest: &Manifest) -> Res {
        remote
            .put_object(
                None,
                &S3Uri::try_from(format!("s3://b/.quilt/packages/{hash}").as_str())?,
                ByteStream::from(manifest),
            )
            .await
    }

    async fn put_object_at(remote: &MockRemote, key: &str, body: &[u8]) -> Res {
        remote
            .put_object(
                None,
                &S3Uri::try_from(format!("s3://b/{key}").as_str())?,
                body.to_vec(),
            )
            .await
    }

    fn wd() -> PathBuf {
        PathBuf::from("/wd")
    }

    /// A row whose hash is `body`'s own: an install verifies what it fetches
    /// against the row, so the object put at `object` must be `body`.
    fn row_at(key: &str, body: &[u8], object: &str) -> ManifestRow {
        use sha2::Digest;
        ManifestRow {
            logical_key: PathBuf::from(key),
            physical_key: format!("s3://b/{object}"),
            hash: multihash::Multihash::<256>::wrap(0x12, &sha2::Sha256::digest(body))
                .unwrap()
                .try_into()
                .unwrap(),
            size: body.len() as u64,
            meta: None,
        }
    }

    /// The invariant the whole change exists for. An apply that dies part-way
    /// must leave **every** tracked path on disk, each holding `base`'s bytes
    /// or `latest`'s — never absent, and never a prefix of either.
    ///
    /// Under the old delete-then-install order this could not hold: the
    /// uninstall removed all three files before the first fetch, so a failure
    /// on the second left two paths gone from the working tree *and* dropped
    /// from the lineage, which a retry could only read as a local delete
    /// against a remote modify and refuse with `PullConflict`. Forever — the
    /// gap re-derived the same verdict on every attempt.
    ///
    /// The interruption here lands in the fetch, so it also pins the stronger
    /// property staging adds: nothing has been swapped in, so the tree is not
    /// merely whole but wholly at `base`.
    #[test(tokio::test)]
    async fn interrupted_apply_leaves_every_tracked_path_whole() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "OLD".to_string(),
            origin: None,
        };
        // Absolute root: `install_paths` builds a `file://` URL from the object
        // path, which requires one.
        let (paths, _domain_tmp) = DomainPaths::from_temp_dir()?;
        let paths = &paths;
        let storage = MockStorage::default();
        paths
            .scaffold_for_caching(&storage, &manifest_uri.bucket)
            .await?;

        let tracked = ["a.txt", "b.txt", "c.txt"];
        for key in tracked {
            storage
                .write_byte_stream(
                    wd().join(key),
                    ByteStream::from(format!("base-{key}").into_bytes()),
                )
                .await?;
        }

        let lineage = PackageLineage {
            remote_uri: Some(manifest_uri.clone()),
            base_hash: "OLD".to_string(),
            latest_hash: "NEW".to_string(),
            paths: tracked
                .iter()
                .map(|k| (PathBuf::from(k), PathState::default()))
                .collect(),
            ..PackageLineage::default()
        };

        // `latest` rewrites all three paths.
        let latest_manifest = Manifest {
            rows: tracked
                .iter()
                .map(|k| {
                    row_at(
                        k,
                        format!("new-{k}").as_bytes(),
                        &format!("objects/new-{k}"),
                    )
                })
                .collect(),
            ..Manifest::default()
        };
        let new_hash = "deadbeef";
        let remote = MockRemote::default();
        publish(&remote, new_hash, &latest_manifest).await?;
        // Only the FIRST path's object is fetchable. The second install dies,
        // which is the interruption under test.
        put_object_at(&remote, "objects/new-a.txt", b"new-a.txt").await?;

        let mut manifest = Manifest::default();
        let touched: Vec<PathBuf> = tracked.iter().map(PathBuf::from).collect();
        let result = apply_latest_update(
            lineage,
            &mut manifest,
            paths,
            &storage,
            &remote,
            wd(),
            Namespace::default(),
            ManifestUri {
                hash: new_hash.to_string(),
                ..manifest_uri
            },
            &touched,
            &LocalWork::Protect,
        )
        .await;
        assert!(result.is_err(), "the second fetch was meant to fail");

        // The invariant: present, and holding one revision's bytes whole.
        for key in tracked {
            let on_disk = storage
                .read_bytes(&wd().join(key))
                .await
                .unwrap_or_else(|err| {
                    panic!("tracked path {key} is absent after an interrupted apply: {err:?}")
                });
            let base = format!("base-{key}").into_bytes();
            let latest = format!("new-{key}").into_bytes();
            assert!(
                on_disk == base || on_disk == latest,
                "{key} holds neither revision whole: {:?}",
                String::from_utf8_lossy(&on_disk)
            );
        }
        // And specifically, because the interruption landed in the fetch — which
        // is where essentially all of an apply's time goes — *every* file is
        // still at `base`. Nothing was swapped in, so there is no mixture at
        // all, and a retry is an ordinary update rather than a reconcile. This
        // is what staging the whole set before swapping any of it buys; the
        // per-file replace could only ever leave the first file at `latest`.
        for key in tracked {
            assert_eq!(
                storage.read_bytes(&wd().join(key)).await?,
                format!("base-{key}").into_bytes(),
                "{key} was written before the whole set was staged"
            );
        }
        Ok(())
    }

    /// An edit that lands while the revision is being staged must be refused,
    /// not overwritten.
    ///
    /// Staging the whole touch set before writing any of it puts an entire
    /// fetch between a caller's verification and the write that verification
    /// licensed. A background pull running while someone works is ordinary, so
    /// that gap is where uncommitted work would be lost — silently, and with no
    /// reflog to recover it from. `LocalWork::Protect` re-checks each
    /// destination against the row it was verified at, immediately before
    /// replacing it.
    ///
    /// The working file here simply does not match its base row. The apply
    /// captures rows, not file contents, so it cannot tell that from an edit
    /// that raced in during staging — which is the case this stands in for,
    /// since an in-process test cannot inject a write mid-apply.
    #[test(tokio::test)]
    async fn an_edit_during_staging_is_refused_not_overwritten() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "OLD".to_string(),
            origin: None,
        };
        let (paths, _domain_tmp) = DomainPaths::from_temp_dir()?;
        let paths = &paths;
        let storage = MockStorage::default();
        paths
            .scaffold_for_caching(&storage, &manifest_uri.bucket)
            .await?;

        let path = PathBuf::from("edited.txt");
        let users_edit = b"what the user typed while the pull was running";
        storage
            .write_byte_stream(wd().join(&path), ByteStream::from_static(users_edit))
            .await?;

        // The base row names content this file no longer holds.
        let mut base_row = calculate_hash(
            &storage,
            &wd().join(&path),
            &path,
            &HostConfig {
                checksums: HostChecksums::Sha256Chunked,
                host: None,
            },
        )
        .await?;
        base_row.hash = row_at(
            "edited.txt",
            b"the content the pull verified",
            "objects/base",
        )
        .hash;
        let mut base = Manifest {
            rows: vec![base_row],
            ..Manifest::default()
        };

        let lineage = PackageLineage {
            remote_uri: Some(manifest_uri.clone()),
            base_hash: "OLD".to_string(),
            latest_hash: "NEW".to_string(),
            paths: BTreeMap::from([(path.clone(), PathState::default())]),
            ..PackageLineage::default()
        };
        let latest_manifest = Manifest {
            rows: vec![row_at(
                "edited.txt",
                b"the remote's new content",
                "objects/new-edited",
            )],
            ..Manifest::default()
        };
        let new_hash = "deadbeef";
        let remote = MockRemote::default();
        publish(&remote, new_hash, &latest_manifest).await?;
        put_object_at(&remote, "objects/new-edited", b"the remote's new content").await?;

        let result = apply_latest_update(
            lineage,
            &mut base,
            paths,
            &storage,
            &remote,
            wd(),
            Namespace::default(),
            ManifestUri {
                hash: new_hash.to_string(),
                ..manifest_uri
            },
            std::slice::from_ref(&path),
            &LocalWork::Protect,
        )
        .await;

        assert!(
            matches!(
                result.as_ref().unwrap_err(),
                crate::Error::PackageOp(crate::error::PackageOpError::PullConflict(paths))
                    if paths == &vec![path.clone()]
            ),
            "expected a PullConflict naming the edited path, got: {result:?}"
        );
        assert_eq!(
            storage.read_bytes(&wd().join(&path)).await?,
            users_edit.to_vec(),
            "the user's edit was overwritten"
        );
        Ok(())
    }

    /// The Example's *rare* interruption: one that lands inside the closing run
    /// of renames rather than during the fetch. Everything staged, so the swap
    /// begins — and then stops part-way, leaving earlier paths at `latest` and
    /// later ones at `base`.
    ///
    /// This is the mixture the invariant is written to survive, and staging
    /// makes it rare without making it impossible: the renames are atomic one
    /// at a time and not as a set. Its counterpart in
    /// `flow::pull` shows such a tree retrying cleanly; this one shows the
    /// apply actually producing it, rather than a test hand-building the state
    /// and asserting about it.
    ///
    /// The injection is a destination that cannot be renamed onto — a directory
    /// where a file belongs — standing in for a kill between two renames, which
    /// an in-process test cannot produce.
    #[test(tokio::test)]
    async fn interrupted_swap_leaves_earlier_paths_at_latest() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "OLD".to_string(),
            origin: None,
        };
        let (paths, _domain_tmp) = DomainPaths::from_temp_dir()?;
        let paths = &paths;
        let storage = MockStorage::default();
        paths
            .scaffold_for_caching(&storage, &manifest_uri.bucket)
            .await?;

        let tracked = ["a.txt", "b.txt", "c.txt"];
        for key in tracked {
            storage
                .write_byte_stream(
                    wd().join(key),
                    ByteStream::from(format!("base-{key}").into_bytes()),
                )
                .await?;
        }
        // `b.txt` becomes a directory, so its rename fails and the swap stops
        // there — after `a.txt` has landed and before `c.txt` is reached.
        storage.remove_file(wd().join("b.txt")).await.ok();
        storage.create_dir_all(wd().join("b.txt")).await?;

        let lineage = PackageLineage {
            remote_uri: Some(manifest_uri.clone()),
            base_hash: "OLD".to_string(),
            latest_hash: "NEW".to_string(),
            paths: tracked
                .iter()
                .map(|k| (PathBuf::from(k), PathState::default()))
                .collect(),
            ..PackageLineage::default()
        };
        let latest_manifest = Manifest {
            rows: tracked
                .iter()
                .map(|k| {
                    row_at(
                        k,
                        format!("new-{k}").as_bytes(),
                        &format!("objects/new-{k}"),
                    )
                })
                .collect(),
            ..Manifest::default()
        };
        let new_hash = "deadbeef";
        let remote = MockRemote::default();
        publish(&remote, new_hash, &latest_manifest).await?;
        // Every object is fetchable, so staging completes and the swap starts.
        for key in tracked {
            put_object_at(
                &remote,
                &format!("objects/new-{key}"),
                format!("new-{key}").as_bytes(),
            )
            .await?;
        }

        let mut manifest = Manifest::default();
        let touched: Vec<PathBuf> = tracked.iter().map(PathBuf::from).collect();
        let result = apply_latest_update(
            lineage,
            &mut manifest,
            paths,
            &storage,
            &remote,
            wd(),
            Namespace::default(),
            ManifestUri {
                hash: new_hash.to_string(),
                ..manifest_uri
            },
            &touched,
            &LocalWork::Protect,
        )
        .await;
        assert!(result.is_err(), "the swap was meant to stop at b.txt");

        // The mixture: swapped before the failure, untouched after it. Both
        // whole, which is what makes the retry ordinary.
        assert_eq!(
            storage.read_bytes(&wd().join("a.txt")).await?,
            b"new-a.txt".to_vec(),
            "a.txt was swapped in before the failure"
        );
        assert_eq!(
            storage.read_bytes(&wd().join("c.txt")).await?,
            b"base-c.txt".to_vec(),
            "c.txt was never reached, so it still holds base"
        );
        Ok(())
    }

    /// The apply records which writes *replaced* a file this copy already held,
    /// because the write itself no longer says so. The pull report tells an
    /// update from an addition by exactly this, and it used to read it from the
    /// path having been uninstalled first — which the reorder makes never true.
    #[test(tokio::test)]
    async fn replaced_names_the_paths_that_were_already_here() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "OLD".to_string(),
            origin: None,
        };
        // Absolute root: `install_paths` builds a `file://` URL from the object
        // path, which requires one.
        let (paths, _domain_tmp) = DomainPaths::from_temp_dir()?;
        let paths = &paths;
        let storage = MockStorage::default();
        paths
            .scaffold_for_caching(&storage, &manifest_uri.bucket)
            .await?;

        let held = "held.txt";
        let fresh = "fresh.txt";
        storage
            .write_byte_stream(wd().join(held), ByteStream::from_static(b"old"))
            .await?;

        // Only `held` is tracked; `fresh` is a path this copy never had.
        let lineage = PackageLineage {
            remote_uri: Some(manifest_uri.clone()),
            base_hash: "OLD".to_string(),
            latest_hash: "NEW".to_string(),
            paths: BTreeMap::from([(PathBuf::from(held), PathState::default())]),
            ..PackageLineage::default()
        };

        let latest_manifest = Manifest {
            rows: vec![
                row_at(held, b"new-held", "objects/new-held"),
                row_at(fresh, b"new-fresh", "objects/new-fresh"),
            ],
            ..Manifest::default()
        };
        let new_hash = "deadbeef";
        let remote = MockRemote::default();
        remote
            .put_object(
                None,
                &S3Uri::try_from(format!("s3://b/.quilt/packages/{new_hash}").as_str())?,
                ByteStream::from(&latest_manifest),
            )
            .await?;
        for (object, body) in [
            ("new-held", &b"new-held"[..]),
            ("new-fresh", &b"new-fresh"[..]),
        ] {
            remote
                .put_object(
                    None,
                    &S3Uri::try_from(format!("s3://b/objects/{object}").as_str())?,
                    body.to_vec(),
                )
                .await?;
        }

        let mut manifest = Manifest::default();
        let (_, applied) = apply_latest_update(
            lineage,
            &mut manifest,
            paths,
            &storage,
            &remote,
            wd(),
            Namespace::default(),
            ManifestUri {
                hash: new_hash.to_string(),
                ..manifest_uri
            },
            &[PathBuf::from(held), PathBuf::from(fresh)],
            &LocalWork::Protect,
        )
        .await?;

        assert_eq!(
            applied.replaced,
            vec![PathBuf::from(held)],
            "only the path this copy already held was replaced"
        );
        assert!(
            applied.installed.contains(&PathBuf::from(fresh)),
            "the new path was still written"
        );
        Ok(())
    }

    // A path tracked in lineage but absent from the new `latest` manifest — a
    // trivially-resolved both-removed path (locally deleted + gone from remote,
    // so filtered out of the touch-set and never uninstalled) — must be pruned
    // from `lineage.paths`. Otherwise the persisted lineage tracks a path with
    // no manifest row and `create_status` hard-errors for remote-backed
    // packages.
    #[test(tokio::test)]
    async fn prunes_lineage_path_absent_from_latest_manifest() -> Res {
        let manifest_uri = ManifestUri {
            bucket: "b".to_string(),
            namespace: ("f", "a").into(),
            hash: "OLD".to_string(),
            origin: None,
        };
        let paths = DomainPaths::default();
        let storage = MockStorage::default();
        paths
            .scaffold_for_caching(&storage, &manifest_uri.bucket)
            .await?;

        let stale = PathBuf::from("both-removed.txt");
        let lineage = PackageLineage {
            remote_uri: Some(manifest_uri.clone()),
            base_hash: "OLD".to_string(),
            latest_hash: "NEW".to_string(),
            paths: BTreeMap::from([(stale.clone(), PathState::default())]),
            ..PackageLineage::default()
        };

        let new_hash = "deadbeef";
        let remote = MockRemote::default();
        remote
            .put_object(
                None,
                &S3Uri::try_from(format!("s3://b/.quilt/packages/{new_hash}").as_str())?,
                // `latest` manifest has no rows → no record for `stale`.
                r#"{"version": "v0"}"#.as_bytes().to_vec(),
            )
            .await?;

        let latest = ManifestUri {
            hash: new_hash.to_string(),
            ..manifest_uri
        };
        let mut manifest = Manifest::default();
        let result = apply_latest_update(
            lineage,
            &mut manifest,
            &paths,
            &storage,
            &remote,
            PathBuf::default(),
            Namespace::default(),
            latest,
            // Empty touch-set: `stale` is NOT uninstalled the normal way.
            &[],
            &LocalWork::Protect,
        )
        .await?;

        assert!(
            !result.0.paths.contains_key(&stale),
            "stale both-removed path must be pruned from lineage"
        );
        assert!(
            result.1.uninstalled.is_empty(),
            "pruning a stale row is not an uninstall, and must not be reported as one"
        );
        Ok(())
    }
}
