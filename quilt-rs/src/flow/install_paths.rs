use std::collections::BTreeMap;
use std::collections::HashSet;
use std::collections::hash_map::RandomState;
use std::path::Path;
use std::path::PathBuf;

use tokio_stream::StreamExt;
use tracing::debug;
use tracing::info;
use url::Url;

use crate::Error;
use crate::InstallPathError;
use crate::Res;
use crate::checksum::refresh_hash;
use crate::error::ManifestError;
use crate::error::PackageOpError;
use crate::io::manifest::RowsStream;
use crate::io::manifest::build_manifest_from_rows_stream;
use crate::io::remote::Remote;
use crate::io::storage::Storage;
use crate::lineage::PackageLineage;
use crate::lineage::PathState;
use crate::manifest::Manifest;
use crate::manifest::ManifestRow;
use crate::paths::DomainPaths;
use quilt_uri::Host;
use quilt_uri::Namespace;
use quilt_uri::S3Uri;

async fn cache_immutable_object(
    storage: &impl Storage,
    remote: &impl Remote,
    host: Option<&Host>,
    object_dest: &PathBuf,
    uri: &S3Uri,
) -> Res {
    let stream = remote.get_object_stream(host, uri).await?;
    storage.write_byte_stream(object_dest, stream.body).await
}

/// Copies an object out of the store into this apply's staging directory,
/// returning where it landed. Nothing in the working tree moves.
///
/// Staged rather than copied straight onto the destination because a copy
/// truncates the target and refills it: a kill mid-write would leave a tracked
/// path holding neither revision — present, matching no manifest row, and
/// readable by a retry only as a conflict.
async fn stage_object(
    storage: &impl Storage,
    immutable_source: &Path,
    run_staging: &Path,
) -> Res<PathBuf> {
    // Unique per file: two files in one apply must never stage over each other.
    let staged = run_staging.join(uuid::Uuid::new_v4().to_string());
    storage.copy(&immutable_source, &staged).await?;
    Ok(staged)
}

/// Moves a staged file onto its working-tree destination, atomically.
///
/// The rename is the only step that touches the working tree, and it is a
/// metadata operation: the path holds the old bytes or the new ones, never a
/// prefix of either. `.quilt/` and the working tree share the domain root, so
/// this stays within one filesystem; a package directory mounted from another
/// would fail here, which is the gap the invariant names rather than papers
/// over with a copy fallback that would reintroduce the hole.
async fn commit_staged(
    storage: &impl Storage,
    staged: &Path,
    mutable_target: &Path,
) -> Res<chrono::DateTime<chrono::Utc>> {
    if let Some(parent) = mutable_target.parent() {
        storage.create_dir_all(parent).await?;
    }
    storage.rename(staged, &mutable_target).await?;
    storage.modified_timestamp(&mutable_target).await
}

/// How [`link_staged_into_place`] ended when it did not refuse.
#[derive(Debug, PartialEq, Eq)]
enum Placement {
    /// Every staged file is linked at its destination.
    Linked,
    /// This filesystem has no hard links; nothing was placed.
    LinksUnsupported,
}

/// Whether a failed first link means the filesystem cannot hard-link at all,
/// rather than that this one link failed.
///
/// `Unsupported` is ENOTSUP/EOPNOTSUPP/ENOSYS (exFAT and SMB on macOS, FUSE),
/// `PermissionDenied` is EPERM from Linux FAT, and `CrossesDevices` is EXDEV.
/// A genuine EACCES lands here too, and the rename it falls back to fails the
/// same way, so nothing is written either.
fn links_unsupported(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::Unsupported
            | std::io::ErrorKind::PermissionDenied
            | std::io::ErrorKind::CrossesDevices
    )
}

/// Links every staged file at its destination, refusing any destination that
/// exists at the moment of its own link, and taking back the links it made
/// before a refusal or error so that nothing is placed.
///
/// A hard link is the no-clobber counterpart of the rename: it fails with
/// `AlreadyExists`, atomically, if anything is at the destination, so a file
/// created after the preflight is refused rather than replaced. Every row is
/// linked before any staged file goes, so a rollback loses nothing: each
/// staged copy is still there, and the staging cleanup drops them after.
async fn link_staged_into_place(
    storage: &(impl Storage + Sync),
    staged: &[(PathBuf, PathBuf, ManifestRow)],
) -> Res<Placement> {
    for (linked, (staged_path, working_dest, row)) in staged.iter().enumerate() {
        if let Some(parent) = working_dest.parent() {
            storage.create_dir_all(parent).await?;
        }
        let Err(err) = storage.hard_link(staged_path, working_dest).await else {
            continue;
        };
        if linked == 0 && links_unsupported(&err) {
            debug!("Hard links are unsupported here ({err}); renaming instead");
            return Ok(Placement::LinksUnsupported);
        }
        // Only this call's links: row `linked` failed, so its destination is
        // not ours to remove.
        for (_, placed_dest, _) in &staged[..linked] {
            let _ = storage.remove_file(placed_dest).await;
        }
        if err.kind() == std::io::ErrorKind::AlreadyExists {
            debug!("❌ A local file appeared at {}", working_dest.display());
            return Err(Error::InstallPath(InstallPathError::LocalFileExists(vec![
                row.logical_key.clone(),
            ])));
        }
        return Err(err.into());
    }
    Ok(Placement::Linked)
}

/// Moves each staged file onto its destination, refusing to overwrite one a
/// caller asked to protect that no longer holds the content it was verified at
/// ([`Protect::BaseContent`]), or one that exists at all ([`Protect::Absent`]).
///
/// The re-check is what keeps the guarantee honest once the whole touch set is
/// staged before anything is written: a caller's verification now happens a
/// whole fetch earlier than the write it licenses, and an edit landing in that
/// gap would otherwise be overwritten in silence. Checked immediately before
/// each rename, the exposure is back to the two syscalls between them.
///
/// [`Protect::Absent`] checks every destination in one pass before the first
/// move instead, so a refusal lands nothing: a row placed before a later one
/// was refused would be left untracked, and a retry would refuse it in turn.
/// The move itself then refuses too: each file is hard-linked rather than
/// renamed, which fails on an existing destination, and a refusal there takes
/// back the rows already linked ([`link_staged_into_place`]). Only where the
/// filesystem has no hard links does it fall back to renaming, leaving the
/// rename loop exposed.
///
/// Fail-safe in the same direction as the conflict rule: a file that cannot be
/// read is treated as changed, so the worst case is a retryable refusal rather
/// than lost work.
async fn swap_staged_into_place(
    storage: &(impl Storage + Sync),
    staged: &[(PathBuf, PathBuf, ManifestRow)],
    protect: &Protect<'_>,
    lineage: &mut PackageLineage,
) -> Res {
    if let Protect::Absent = protect {
        let mut appeared = Vec::new();
        for (_, working_dest, row) in staged {
            if storage.exists(working_dest).await {
                appeared.push(row.logical_key.clone());
            }
        }
        if !appeared.is_empty() {
            debug!("❌ Local files appeared while the paths were being staged");
            return Err(Error::InstallPath(InstallPathError::LocalFileExists(
                appeared,
            )));
        }
        if link_staged_into_place(storage, staged).await? == Placement::Linked {
            for (_, working_dest, row) in staged {
                lineage.paths.insert(
                    row.logical_key.clone(),
                    PathState {
                        timestamp: storage.modified_timestamp(working_dest).await?,
                        hash: row.hash.clone().into(),
                    },
                );
                debug!("✔️ Linked in {}", working_dest.display());
            }
            return Ok(());
        }
        // No hard links on this filesystem: the preflight above is the only
        // guard, and the renames below replace a file created after it.
    }
    for (staged_path, working_dest, row) in staged {
        if let Protect::BaseContent(expected) = protect
            && let Some(base_row) = expected.get(&row.logical_key)
        {
            let unchanged = matches!(
                refresh_hash(storage, working_dest, (*base_row).clone()).await,
                Ok(None)
            );
            if !unchanged {
                debug!(
                    "❌ {} changed while the revision was being staged",
                    row.logical_key.display()
                );
                return Err(Error::PackageOp(PackageOpError::PullConflict(vec![
                    row.logical_key.clone(),
                ])));
            }
        }
        let last_modified = commit_staged(storage, staged_path, working_dest).await?;
        lineage.paths.insert(
            row.logical_key.clone(),
            PathState {
                timestamp: last_modified,
                hash: row.hash.clone().into(),
            },
        );
        debug!("✔️ Swapped in {}", working_dest.display());
    }
    Ok(())
}

async fn stream_remote_with_installed_rows(
    remote_manifest: &Manifest,
    local_entries: BTreeMap<PathBuf, ManifestRow>,
) -> impl RowsStream {
    remote_manifest
        .records_stream()
        .await
        .map(move |rows_result| {
            rows_result.map(|rows| {
                rows.into_iter()
                    .map(|row_result| {
                        row_result.map(|row| match local_entries.get(&row.logical_key) {
                            Some(local_row) => local_row.clone(),
                            None => row,
                        })
                    })
                    .collect()
            })
        })
}

/// What a caller forbids the apply to overwrite.
///
/// The check has to happen immediately before each rename, not once up front:
/// staging the whole touch set puts the whole fetch between a caller's
/// verification and the write it licensed, and a background pull running while
/// someone works is ordinary rather than exotic. But it cannot simply live
/// inside the install, because [`reset_to_latest`](super::reset_to_latest)
/// shares this primitive precisely to *discard* local work. So the caller says
/// which it is, by name.
pub(crate) enum Protect<'a> {
    /// Replace a working file only while it still holds the content its row
    /// names. Anything else is an edit that landed after the caller checked,
    /// and overwriting it would lose work that was never committed and cannot
    /// be recovered — there is no reflog.
    BaseContent(&'a BTreeMap<PathBuf, ManifestRow>),
    /// Overwrite whatever is there. Discarding local work is the operation, so
    /// there is nothing to protect.
    Nothing,
    /// Replace nothing: refuse a destination that exists. Anything there is a
    /// file this copy does not track — the user's own, never committed — and
    /// writing over it would lose it just as surely as an edit to a tracked one.
    /// Checked up front and then by the placement itself, a hard link that
    /// fails on an existing file, so none created in between is overwritten
    /// either (except where the filesystem has no hard links).
    Absent,
}

/// Refuses the whole call if any requested path is already installed.
///
/// The check reads `lineage.paths`, not the working tree: "already installed"
/// means this copy tracks the path, and so may hold edits in it that writing
/// over would destroy with nothing to recover them from.
fn refuse_already_installed(lineage: &PackageLineage, entries_paths: &[&PathBuf]) -> Res {
    debug!("🔍 Checking for already installed paths");
    if !lineage
        .paths
        .keys()
        .collect::<HashSet<&PathBuf, RandomState>>()
        .is_disjoint(&entries_paths.iter().copied().collect::<HashSet<_>>())
    {
        debug!("❌ Found paths that are already installed");
        return Err(Error::InstallPath(InstallPathError::AlreadyInstalled));
    }
    Ok(())
}

/// Refuses the whole call if any requested path already has a file in the
/// working folder.
///
/// Complements [`refuse_already_installed`]: a path this copy does not track can
/// still hold the user's own new file, and that is uncommitted work too.
async fn refuse_existing_working_files(
    storage: &impl Storage,
    working_dir: &Path,
    entries_paths: &[&PathBuf],
) -> Res {
    debug!("🔍 Checking for local files at the requested paths");
    let mut existing = Vec::new();
    for path in entries_paths {
        if storage.exists(working_dir.join(path)).await {
            existing.push((*path).clone());
        }
    }
    if !existing.is_empty() {
        debug!("❌ Found local files at requested paths");
        return Err(Error::InstallPath(InstallPathError::LocalFileExists(
            existing,
        )));
    }
    Ok(())
}

/// Installs paths this copy does not already hold, refusing the whole call if
/// any of them is already installed or already has a file in the working
/// folder.
///
/// This is the verb a user reaches, directly or through
/// [`InstalledPackage::install_paths`](crate::InstalledPackage::install_paths).
/// Writing over a path someone is editing loses work that was never committed
/// and cannot be recovered, so asking for one is refused rather than guessed
/// at. The reconcile, whose paths are known to carry no local edit, uses
/// [`install_paths_over`] instead — a separate entry point rather than a flag
/// on this one, so that overwriting is something a caller *chooses by name*
/// and cannot reach from outside this crate at all.
#[allow(clippy::too_many_arguments)]
pub async fn install_paths(
    lineage: PackageLineage,
    manifest: &mut Manifest,
    paths: &DomainPaths,
    working_dir: PathBuf,
    namespace: Namespace,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    entries_paths: &[&PathBuf],
) -> Res<PackageLineage> {
    refuse_already_installed(&lineage, entries_paths)?;
    refuse_existing_working_files(storage, &working_dir, entries_paths).await?;
    // Checked up front so a refusal fetches nothing, and again at placement
    // (`Protect::Absent`) because a file can appear during the fetch.
    install_paths_over(
        lineage,
        manifest,
        paths,
        working_dir,
        namespace,
        storage,
        remote,
        entries_paths,
        &Protect::Absent,
    )
    .await
}

/// Installs paths **over** whatever the working tree already holds for them.
///
/// `pub(crate)` on purpose: the only caller entitled to this is the reconcile
/// in [`apply_latest_update`](super::apply_update), whose touch set excludes
/// every path the user has touched, so no local edit is ever at stake. Nothing
/// outside this crate can reach it, which is what makes "only the reconcile may
/// install over a tracked path" a fact about the code rather than a convention
/// callers have to keep.
///
/// Each write lands whole, and the whole touch set is staged before any of it
/// is swapped in — see [`stage_object`] and [`commit_staged`].
///
/// Rows go into the installed manifest **verbatim** — `physical_key` is never
/// rewritten to the `file://` object-store location, despite the `place` value
/// computed below (logged, never consumed) and the plan sketch's "replace
/// entry's physical key" step (never implemented). Nothing needs the rewrite:
/// [`matches_content`](crate::manifest::ManifestRow::matches_content) ignores
/// `physical_key`, so a later push dedups by content whatever the scheme.
///
/// A row's scheme is therefore just its source manifest's, not an origin
/// marker. `file://` means the bytes were committed locally
/// ([`commit`](fn@super::commit) / [`create`](fn@super::create)) and outlives a
/// push, which never rewrites the installed manifest. Watch out — the caching
/// call below parses `physical_key` as an `S3Uri`, so a `file://` row whose
/// object is missing from the cache errors instead of being fetched.
// TODO: `working_dir` is in `paths` already, and we pass namespace anyway
//       so we can remove working_dir from the arguments
#[allow(clippy::too_many_arguments)]
pub(crate) async fn install_paths_over(
    mut lineage: PackageLineage,
    manifest: &mut Manifest,
    paths: &DomainPaths,
    working_dir: PathBuf, // This working dir is working dir of the package
    namespace: Namespace,
    storage: &(impl Storage + Sync),
    remote: &impl Remote,
    entries_paths: &[&PathBuf],
    protect: &Protect<'_>,
) -> Res<PackageLineage> {
    if entries_paths.is_empty() {
        info!("No paths to install");
        return Ok(lineage);
    }

    let remote_uri = lineage.remote()?.clone();

    info!(
        "⏳ Installing {} paths for package {}",
        entries_paths.len(),
        namespace
    );

    // for each path in entries_paths:
    //   get entry from installed manifest
    //   cache the entry into identity cache (if not there)
    //   (the sketch rewrote the physical key here; the code does not — see above)
    //
    // write the adjusted manifest into the installed manifest path
    // copy the selected paths into the working folder
    //
    // record installation into the lineage:
    //   add installed package entry:
    //     remote: RemoteManifest
    let mut entries = BTreeMap::new();
    // Outside the tree the status walk reads: a staging file beside the working
    // file would be reported as a new file, committed if a commit landed in the
    // window, and left as a permanent stray by a kill. One subdirectory per
    // apply, so a failure can drop exactly its own staged files without
    // touching a concurrent apply's.
    let run_staging = paths.staging_dir().join(uuid::Uuid::new_v4().to_string());
    storage.create_dir_all(&run_staging).await?;
    // Fetching and staging, as one fallible phase. Every `?` in here — a
    // missing manifest row, a failed fetch, an unparseable physical key, a
    // relative object path — lands on the single cleanup below, so none of them
    // can strand the staged set. Scoping it this way rather than sweeping after
    // each call is what makes "no error leaks a staging directory" a property of
    // the shape instead of a list of call sites to remember.
    let staging: Res<Vec<(PathBuf, PathBuf, ManifestRow)>> = async {
        let mut staged: Vec<(PathBuf, PathBuf, ManifestRow)> = Vec::new();

        for path in entries_paths {
            // TODO: Consider using a hashmap or treemap for manifest.rows
            let row = manifest
                .get_record(path)
                .ok_or(ManifestError::Table(format!(
                    "path \"{}\" not found",
                    path.display()
                )))?;

            let object_dest = paths.object(row.hash.digest());

            if storage.exists(&object_dest).await {
                debug!("✔️ Object already in cache: {}", object_dest.display());
            } else {
                cache_immutable_object(
                    storage,
                    remote,
                    remote_uri.origin.as_ref(),
                    &object_dest,
                    &row.physical_key.parse()?,
                )
                .await?;
                debug!("✔️ Cached object: {}", object_dest.display());
            }

            // Diagnostic only: the `file://` URL the row *would* carry if rows were
            // rewritten to the object store (they are not — see above). Logged and
            // discarded; the error arm still asserts `object_dest` is absolute.
            let place = Url::from_file_path(&object_dest)
                .map_err(|()| Error::InstallPath(InstallPathError::Install(object_dest.clone())))?
                .to_string();
            debug!(
                "✔️ Path {} converted to a `place` {}",
                object_dest.display(),
                place
            );
            // The row goes in unchanged — `physical_key` and all.
            entries.insert(row.logical_key.clone(), row.clone());

            // Staged, not written: the working tree stays wholly at its current
            // revision until every file is ready.
            let staged_path = stage_object(storage, &object_dest, &run_staging).await?;
            staged.push((staged_path, working_dir.join(&row.logical_key), row.clone()));
            debug!("✔️ Staged {}", row.logical_key.display());
        }
        Ok(staged)
    }
    .await;
    let staged = match staging {
        Ok(staged) => staged,
        Err(err) => {
            let _ = storage.remove_dir_all(&run_staging).await;
            return Err(err);
        }
    };

    // The swap. Everything above this line is fetching and copying, and none of
    // it touched the working tree: interrupted anywhere earlier — which is where
    // essentially all of the time goes — the tree is still wholly at `base` and
    // a retry is an ordinary update with nothing to reconcile. From here it is
    // renames only, milliseconds, and an interruption inside them leaves the
    // mixed tree the invariant is written to survive.
    //
    // Not a transaction: the renames are atomic one at a time and not as a set.
    debug!("⏳ Swapping {} staged files into place", staged.len());
    let swapped = swap_staged_into_place(storage, &staged, protect, &mut lineage).await;
    // On every path out, not only the successful one: the renames empty this
    // directory when they all land, but any error above leaves the whole staged
    // set behind, and at one object apiece that accumulates across retries. A
    // kill still strands one, which no sweep collects yet.
    let _ = storage.remove_dir_all(&run_staging).await;
    swapped?;

    debug!("⏳ Building manifest with installed rows");
    let stream = stream_remote_with_installed_rows(manifest, entries).await;
    let dest_dir = paths.installed_manifests_dir(&namespace);
    build_manifest_from_rows_stream(storage, dest_dir, manifest.header.clone(), stream).await?;

    info!("✔️ Successfully installed {} paths", entries_paths.len());
    Ok(lineage)
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use aws_sdk_s3::primitives::ByteStream;
    use std::path::PathBuf;
    use std::str::FromStr;

    use crate::fixtures;
    use crate::io::remote::mocks::MockRemote;
    use crate::io::storage::StorageExt;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::Home;
    use crate::paths;
    use quilt_uri::ManifestUri;

    // Verify installing the path that is already fetched to the `.quilt/objects`
    // Practically it is useful when we try to install identical files. Then we can re-use cache (because files are located by hash).
    // In other cases, it tests implementation details.
    #[test(tokio::test)]
    async fn test_installing_one_cached_path() -> Res {
        let (home, _temp_dir1) = Home::from_temp_dir()?;
        let (domain_paths, _temp_dir2) = &DomainPaths::from_temp_dir()?;

        let namespace = Namespace::from(("foo", "bar"));
        let package_home = paths::package_home(&home, &namespace);

        // Simulate the file already exists in `.quilt/objects/HASH`
        // We trust that the hash is correct, so we can skip the actual file content
        let storage = MockStorage::default();

        // Simulate the manifest with rows containing objects
        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri::default()),
            ..PackageLineage::default()
        };
        let single_object_path = PathBuf::from("less-then-8mb.txt");
        let entries_paths = vec![&single_object_path];
        let mut manifest = fixtures::manifest_with_objects_all_sizes::manifest().await?;

        let hash: multihash::Multihash<256> = manifest
            .get_record(&single_object_path)
            .unwrap()
            .hash
            .clone()
            .into();
        // let hash = fixtures::create_multihash(fixtures::objects::LESS_THAN_8MB_HASH_B64)?;
        let object_path = domain_paths.object(hash.digest());
        let absolute_path = home.join(object_path);
        // Path is `.quilt/objects/HASH`
        storage
            .write_byte_stream(absolute_path, ByteStream::default())
            .await?;

        // Lineage does not track anything before the installation
        assert!(lineage.paths.is_empty());

        // We deal with cached file, so remote is "empty" and doesn't make any HTTP calls,
        // since it doesn't throw "key not found"
        let remote = MockRemote::default();
        let lineage = install_paths(
            lineage,
            &mut manifest,
            domain_paths,
            package_home.clone(),
            namespace,
            &storage,
            &remote,
            &entries_paths,
        )
        .await?;

        // Now lineage tracks the file in the working directory
        assert!(lineage.paths.contains_key(&single_object_path));
        // And working directory of the package contains the file
        assert!(storage.exists(&package_home.join(single_object_path)).await);

        Ok(())
    }

    /// Verify installing a path that is not cached locally in `.quilt/objects`.
    /// The path should be downloaded from the remote storage, cached locally, and then installed into the working directory.
    #[test(tokio::test)]
    async fn test_installing_one_uncached_path() -> Res {
        let (home, _temp_dir1) = Home::from_temp_dir()?;
        let (domain_paths, _temp_dir2) = &DomainPaths::from_temp_dir()?;

        let namespace = Namespace::from(("foo", "bar"));
        let package_home = paths::package_home(&home, &namespace);

        // Simulate the manifest with rows containing an object path
        let remote = MockRemote::default();
        let storage = MockStorage::default();
        let single_object_path = PathBuf::from("a/a");
        let entries_paths = vec![&single_object_path];

        domain_paths
            .scaffold_for_installing(&storage, &home, &namespace)
            .await?;

        let remote_file_url = "s3://any/valid-url.md".to_string();

        // Before installation, lineage does not track any paths
        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri::default()),
            ..PackageLineage::default()
        };

        // Simulate the remote object
        let remote_object_uri = S3Uri::from_str(&remote_file_url)?;
        remote
            .put_object(None, &remote_object_uri, Vec::new())
            .await?;

        // Create the manifest with a single remote row with a random hash
        let hash: multihash::Multihash<256> = multihash::Multihash::wrap(0x12, b"anything")?;
        let mut manifest = Manifest::default();
        manifest
            .insert_record(ManifestRow {
                logical_key: single_object_path.clone(),
                hash: hash.try_into()?,
                physical_key: remote_file_url,
                ..ManifestRow::default()
            })
            .await?;

        assert!(lineage.paths.is_empty());

        // Perform the installation
        let lineage = install_paths(
            lineage,
            &mut manifest,
            domain_paths,
            package_home.clone(),
            namespace,
            &storage,
            &remote,
            &entries_paths,
        )
        .await?;

        // Verify the path is now tracked in lineage
        assert!(lineage.paths.contains_key(&single_object_path));
        // Verify the working directory contains the installed file
        assert!(storage.exists(&package_home.join(single_object_path)).await);
        // Verify the object is cached locally in `.quilt/objects`
        // Note, that we don't verify the hash and trust the manifest
        let object_path = domain_paths.object(hash.digest());
        assert!(storage.exists(object_path).await);

        Ok(())
    }

    // Nothing special, just a combination of two previous tests,
    // so we're sure that single file is not a special case.
    #[test(tokio::test)]
    async fn test_installing_multiple_paths() -> Res {
        let (home, _temp_dir1) = Home::from_temp_dir()?;
        let (domain_paths, _temp_dir2) = &DomainPaths::from_temp_dir()?;

        let namespace = Namespace::from(("foo", "bar"));
        let package_home = paths::package_home(&home, &namespace);

        // Simulate the manifest with rows containing objects
        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri::default()),
            ..PackageLineage::default()
        };
        let row_1 = ManifestRow {
            logical_key: PathBuf::from("a"),
            physical_key: "file:///ignored".to_string(),
            hash: multihash::Multihash::wrap(0x12, b"one")?.try_into()?,
            ..ManifestRow::default()
        };
        let row_2 = ManifestRow {
            logical_key: PathBuf::from("b/b"),
            physical_key: "s3://bucket/foo/bar".to_string(),
            hash: multihash::Multihash::wrap(0x12, b"two")?.try_into()?,
            ..ManifestRow::default()
        };
        let row_3 = ManifestRow {
            logical_key: PathBuf::from("c/c/c"),
            physical_key: "file:///ignored".to_string(),
            hash: multihash::Multihash::wrap(0x12, b"three")?.try_into()?,
            ..ManifestRow::default()
        };
        let row_4 = ManifestRow {
            logical_key: PathBuf::from("d/d/d/d"),
            physical_key: "s3://bucket/foo/baz".to_string(),
            hash: multihash::Multihash::wrap(0x12, b"four")?.try_into()?,
            ..ManifestRow::default()
        };
        let mut manifest = Manifest::default();
        manifest.insert_record(row_1.clone()).await?;
        manifest.insert_record(row_2.clone()).await?;
        manifest.insert_record(row_3.clone()).await?;
        manifest.insert_record(row_4.clone()).await?;

        // Simulate two of three files (1 and 3) are already exist in `.quilt/objects/HASH`
        // We trust that the hash is correct, so we can skip the actual file content
        let storage = MockStorage::default();
        let object_path_1 = home.join(domain_paths.object(row_1.hash.digest()));
        storage
            .write_byte_stream(object_path_1, ByteStream::default())
            .await?;
        let object_path_3 = home.join(domain_paths.object(row_3.hash.digest()));
        storage
            .write_byte_stream(object_path_3, ByteStream::default())
            .await?;

        // Simulate the remote object
        let remote = MockRemote::default();
        let remote_object_uri_2 = S3Uri::from_str(&row_2.physical_key)?;
        remote
            .put_object(None, &remote_object_uri_2, Vec::new())
            .await?;
        let remote_object_uri_4 = S3Uri::from_str(&row_4.physical_key)?;
        remote
            .put_object(None, &remote_object_uri_4, Vec::new())
            .await?;

        let entries_paths = vec![
            &row_1.logical_key,
            &row_2.logical_key,
            &row_3.logical_key,
            &row_4.logical_key,
        ];

        // Lineage does not track anything before the installation
        assert!(lineage.paths.is_empty());

        let lineage = install_paths(
            lineage,
            &mut manifest,
            domain_paths,
            package_home.clone(),
            namespace,
            &storage,
            &remote,
            &entries_paths,
        )
        .await?;

        // Now lineage tracks the files in the working directory
        assert!(lineage.paths.contains_key(&row_1.logical_key));
        assert!(lineage.paths.contains_key(&row_2.logical_key));
        assert!(lineage.paths.contains_key(&row_3.logical_key));
        assert!(lineage.paths.contains_key(&row_4.logical_key));
        // And working directory of the package contains the files
        assert!(storage.exists(&package_home.join(&row_1.logical_key)).await);
        assert!(storage.exists(&package_home.join(&row_2.logical_key)).await);
        assert!(storage.exists(&package_home.join(&row_3.logical_key)).await);
        assert!(storage.exists(&package_home.join(&row_4.logical_key)).await);

        Ok(())
    }

    // Verify that the installation fails when we try to install a path that doesn't exist in the
    // manifest.
    #[test(tokio::test)]
    async fn test_installing_path_that_doesnt_exists_in_manifest() -> Res {
        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri::default()),
            ..PackageLineage::default()
        };
        let remote = MockRemote::default();
        let storage = MockStorage::default();

        let not_existed = PathBuf::from("z/z");
        // We want to install z/z
        let entries_paths = vec![&not_existed];
        // But manifest clearly doens't contain it
        let mut manifest = fixtures::manifest_with_objects_all_sizes::manifest().await?;

        // Assert we don't track anything
        assert!(lineage.paths.is_empty());

        let lineage = install_paths(
            lineage,
            &mut manifest,
            &DomainPaths::default(),
            PathBuf::new(),
            Namespace::default(),
            &storage,
            &remote,
            &entries_paths,
        )
        .await;
        assert_eq!(
            lineage.unwrap_err().to_string(),
            r#"Table error: path "z/z" not found"#
        );
        Ok(())
    }

    #[test(tokio::test)]
    async fn test_installing_more_than_1024_paths() -> Res {
        let (home, _temp_dir1) = Home::from_temp_dir()?;
        let (domain_paths, _temp_dir2) = &DomainPaths::from_temp_dir()?;

        let namespace = Namespace::from(("foo", "bar"));
        let package_home = paths::package_home(&home, &namespace);

        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri::default()),
            ..PackageLineage::default()
        };
        let storage = MockStorage::default();
        let remote = MockRemote::default();

        let mut manifest = Manifest::default();
        let mut entries_paths = Vec::new();
        let mut path_refs = Vec::new();

        // Create 1024 * 2 test paths and rows
        for i in 0..2048 {
            let path = PathBuf::from(format!("path_{i}.txt"));
            let place = format!("s3://bucket/path_{i}.txt");
            let hash = multihash::Multihash::wrap(0x12, format!("hash_{i}").as_bytes())?;

            let row = ManifestRow {
                logical_key: path.clone(),
                physical_key: place.clone(),
                hash: hash.try_into()?,
                ..ManifestRow::default()
            };

            manifest.insert_record(row).await?;
            entries_paths.push(path);

            // Simulate remote objects
            let remote_uri = S3Uri::from_str(&place)?;
            remote.put_object(None, &remote_uri, Vec::new()).await?;
        }

        // Create references for the function call
        for path in &entries_paths {
            path_refs.push(path);
        }

        domain_paths
            .scaffold_for_installing(&storage, &home, &namespace)
            .await?;

        assert!(lineage.paths.is_empty());

        let lineage = install_paths(
            lineage,
            &mut manifest,
            domain_paths,
            package_home.clone(),
            namespace,
            &storage,
            &remote,
            &path_refs,
        )
        .await?;

        // Verify all 2048 paths are tracked in lineage
        assert_eq!(lineage.paths.len(), 2048);
        for path in &entries_paths {
            assert!(lineage.paths.contains_key(path));
        }

        // Verify all files exist in working directory
        for path in &entries_paths {
            assert!(storage.exists(&package_home.join(path)).await);
        }

        Ok(())
    }

    /// A file the user created at a requested path is uncommitted work: writing
    /// the remote file over it would lose it with nothing to recover it from.
    #[test(tokio::test)]
    async fn an_untracked_local_file_at_a_requested_path_is_refused() -> Res {
        let (home, _temp_dir1) = Home::from_temp_dir()?;
        let (domain_paths, _temp_dir2) = &DomainPaths::from_temp_dir()?;

        let namespace = Namespace::from(("foo", "bar"));
        let package_home = paths::package_home(&home, &namespace);

        let remote = MockRemote::default();
        let storage = MockStorage::default();
        let requested = PathBuf::from("a/a");
        let entries_paths = vec![&requested];

        domain_paths
            .scaffold_for_installing(&storage, &home, &namespace)
            .await?;

        let remote_file_url = "s3://any/valid-url.md".to_string();
        remote
            .put_object(
                None,
                &S3Uri::from_str(&remote_file_url)?,
                b"remote bytes".to_vec(),
            )
            .await?;
        let hash: multihash::Multihash<256> = multihash::Multihash::wrap(0x12, b"anything")?;
        let mut manifest = Manifest::default();
        manifest
            .insert_record(ManifestRow {
                logical_key: requested.clone(),
                hash: hash.try_into()?,
                physical_key: remote_file_url,
                ..ManifestRow::default()
            })
            .await?;

        // The user's own new file, not in `lineage.paths`
        let users_file = package_home.join(&requested);
        storage
            .write_byte_stream(&users_file, ByteStream::from_static(b"users new file"))
            .await?;
        let lineage = PackageLineage {
            remote_uri: Some(ManifestUri::default()),
            ..PackageLineage::default()
        };

        let result = install_paths(
            lineage,
            &mut manifest,
            domain_paths,
            package_home.clone(),
            namespace,
            &storage,
            &remote,
            &entries_paths,
        )
        .await;

        assert!(matches!(
            result,
            Err(Error::InstallPath(InstallPathError::LocalFileExists(ref p))) if p == &vec![requested.clone()]
        ));
        assert_eq!(storage.read_bytes(&users_file).await?, b"users new file");
        // Refused before fetching anything
        assert!(!storage.exists(domain_paths.object(hash.digest())).await);

        Ok(())
    }

    /// The up-front refusal is a whole fetch before the write; a file created
    /// in that gap must not be overwritten either.
    #[test(tokio::test)]
    async fn a_local_file_created_during_the_fetch_is_not_overwritten() -> Res {
        let storage = MockStorage::default();
        let logical_key = PathBuf::from("a/a");
        let staged_path = PathBuf::from("staging/run/staged");
        let working_dest = PathBuf::from("work").join(&logical_key);
        storage
            .write_byte_stream(&staged_path, ByteStream::from_static(b"remote bytes"))
            .await?;
        storage
            .write_byte_stream(&working_dest, ByteStream::from_static(b"users new file"))
            .await?;
        let row = ManifestRow {
            logical_key: logical_key.clone(),
            hash: multihash::Multihash::wrap(0x12, b"anything")?.try_into()?,
            ..ManifestRow::default()
        };
        let mut lineage = PackageLineage::default();

        let result = swap_staged_into_place(
            &storage,
            &[(staged_path, working_dest.clone(), row)],
            &Protect::Absent,
            &mut lineage,
        )
        .await;

        assert!(matches!(
            result,
            Err(Error::InstallPath(InstallPathError::LocalFileExists(ref p))) if p == &vec![logical_key.clone()]
        ));
        assert_eq!(storage.read_bytes(&working_dest).await?, b"users new file");
        assert!(!lineage.paths.contains_key(&logical_key));

        Ok(())
    }

    /// A refusal lands nothing: a row renamed before a later one is refused
    /// would sit untracked, and a retry would then refuse it as a local file.
    #[test(tokio::test)]
    async fn a_refusal_under_protect_absent_renames_none_of_the_rows() -> Res {
        let storage = MockStorage::default();
        let row = |key: &str| -> Res<ManifestRow> {
            Ok(ManifestRow {
                logical_key: PathBuf::from(key),
                hash: multihash::Multihash::wrap(0x12, b"anything")?.try_into()?,
                ..ManifestRow::default()
            })
        };
        let (first_staged, first_dest) = (PathBuf::from("staging/run/1"), PathBuf::from("work/a"));
        let (second_staged, second_dest) =
            (PathBuf::from("staging/run/2"), PathBuf::from("work/b"));
        for staged in [&first_staged, &second_staged] {
            storage
                .write_byte_stream(staged, ByteStream::from_static(b"remote bytes"))
                .await?;
        }
        storage
            .write_byte_stream(&second_dest, ByteStream::from_static(b"users new file"))
            .await?;
        let mut lineage = PackageLineage::default();

        let result = swap_staged_into_place(
            &storage,
            &[
                (first_staged, first_dest.clone(), row("a")?),
                (second_staged, second_dest.clone(), row("b")?),
            ],
            &Protect::Absent,
            &mut lineage,
        )
        .await;

        assert!(matches!(
            result,
            Err(Error::InstallPath(InstallPathError::LocalFileExists(ref p))) if p == &vec![PathBuf::from("b")]
        ));
        assert!(
            !storage.exists(&first_dest).await,
            "the first row stays staged"
        );
        assert_eq!(storage.read_bytes(&second_dest).await?, b"users new file");
        assert!(lineage.paths.is_empty());

        Ok(())
    }

    #[test(tokio::test)]
    async fn an_absent_destination_is_swapped_in_under_protect_absent() -> Res {
        let storage = MockStorage::default();
        let logical_key = PathBuf::from("a/a");
        let staged_path = PathBuf::from("staging/run/staged");
        let working_dest = PathBuf::from("work").join(&logical_key);
        storage
            .write_byte_stream(&staged_path, ByteStream::from_static(b"remote bytes"))
            .await?;
        let row = ManifestRow {
            logical_key: logical_key.clone(),
            hash: multihash::Multihash::wrap(0x12, b"anything")?.try_into()?,
            ..ManifestRow::default()
        };
        let mut lineage = PackageLineage::default();

        swap_staged_into_place(
            &storage,
            &[(staged_path, working_dest.clone(), row)],
            &Protect::Absent,
            &mut lineage,
        )
        .await?;

        assert_eq!(storage.read_bytes(&working_dest).await?, b"remote bytes");
        assert!(lineage.paths.contains_key(&logical_key));

        Ok(())
    }

    /// The preflight is a whole placement loop before the last move; a file
    /// created at a later destination in that gap must not be overwritten, and
    /// the rows already placed are taken back so the refusal lands nothing.
    #[test(tokio::test)]
    async fn a_file_created_after_the_preflight_is_kept_and_earlier_rows_are_rolled_back() -> Res {
        let storage = MockStorage::default();
        let row = |key: &str| -> Res<ManifestRow> {
            Ok(ManifestRow {
                logical_key: PathBuf::from(key),
                hash: multihash::Multihash::wrap(0x12, b"anything")?.try_into()?,
                ..ManifestRow::default()
            })
        };
        let (first_staged, first_dest) = (PathBuf::from("staging/run/1"), PathBuf::from("work/a"));
        let (second_staged, second_dest) =
            (PathBuf::from("staging/run/2"), PathBuf::from("work/sub/b"));
        for staged in [&first_staged, &second_staged] {
            storage
                .write_byte_stream(staged, ByteStream::from_static(b"remote bytes"))
                .await?;
        }
        // Past the preflight: the user's file appears before the second move.
        storage
            .write_byte_stream(&second_dest, ByteStream::from_static(b"users new file"))
            .await?;

        let result = link_staged_into_place(
            &storage,
            &[
                (first_staged.clone(), first_dest.clone(), row("a")?),
                (second_staged.clone(), second_dest.clone(), row("sub/b")?),
            ],
        )
        .await;

        assert!(
            matches!(
                result,
                Err(Error::InstallPath(InstallPathError::LocalFileExists(ref p))) if p == &vec![PathBuf::from("sub/b")]
            ),
            "got {result:?}"
        );
        assert_eq!(storage.read_bytes(&second_dest).await?, b"users new file");
        assert!(
            !storage.exists(&first_dest).await,
            "the first row is rolled back"
        );
        assert_eq!(storage.read_bytes(&first_staged).await?, b"remote bytes");
        assert_eq!(storage.read_bytes(&second_staged).await?, b"remote bytes");

        Ok(())
    }

    #[test(tokio::test)]
    async fn every_staged_file_is_linked_into_place_when_nothing_is_there() -> Res {
        let storage = MockStorage::default();
        let row = ManifestRow {
            logical_key: PathBuf::from("sub/a"),
            hash: multihash::Multihash::wrap(0x12, b"anything")?.try_into()?,
            ..ManifestRow::default()
        };
        let (staged, dest) = (PathBuf::from("staging/run/1"), PathBuf::from("work/sub/a"));
        storage
            .write_byte_stream(&staged, ByteStream::from_static(b"remote bytes"))
            .await?;

        let placement = link_staged_into_place(&storage, &[(staged, dest.clone(), row)]).await?;

        assert_eq!(placement, Placement::Linked);
        assert_eq!(storage.read_bytes(&dest).await?, b"remote bytes");

        Ok(())
    }

    // TODO: fail if path is already installed
    // TODO: fail if manifest entry has invalid URL
}
