//! The edits the package page makes on the dataspace engine: the package's
//! registry and catalog, edits to `data.toml` (25.3, 25.4), and its
//! `.quiltignore`; and the local registry's rename (25.10).

use std::path::Path;
use std::path::PathBuf;

use quilt_rs::dataspace::contract::CONTRACT_FILE;
use quilt_rs::dataspace::contract::Registry;
use quilt_rs::dataspace::keep::Keep;
use quilt_rs::io::remote::Remote;
use quilt_rs::quiltignore::Rules;
use quilt_rs::quiltignore::Scope;

use crate::engine::commands::ScopePreview;
use crate::engine::commands::StopIgnoringPreview;
use crate::engine::commands::package_ops::remote_origin;
use crate::engine::dataspace::Dataspace;
use crate::engine::dataspace::fail;
use crate::engine::error::Error;

use super::namespace;
use super::pages::PackageRead;

/// Moves the package to `bucket`, with the catalog `origin` names: an edit
/// of its entry in `data.toml`, its folder staying where it is (25.4). The
/// next sync publishes it there; a package from the local registry is
/// pushed as its last commit (25.10). A package with a published revision
/// keeps its registry, as the ref path keeps a pushed remote.
pub(in crate::engine::commands) async fn set_remote<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
    origin: &str,
    bucket: &str,
) -> Result<(), Error> {
    let namespace = namespace(text)?;
    let bucket = bucket.trim();
    if bucket.is_empty() {
        return Err(Error::Dataspace("Name a bucket".to_string()));
    }
    let status = ds.entry_status(&namespace, false).await?;
    if status.hashes.base.is_some() {
        return Err(Error::Dataspace(format!(
            "{namespace} is published to {}; edit {CONTRACT_FILE} to move it",
            status.entry.registry.spelled()
        )));
    }
    let mut request = Keep::new(status.entry.key());
    request.from = Some(status.entry.access);
    request.registry = Some(Registry::S3(bucket.to_string()));
    request.catalog = Some(remote_origin(origin)?);
    ds.keep(&request).await?;
    Ok(())
}

/// Sets the package's catalog, or takes its own off where `origin` is
/// blank, so the contract's default applies: an edit of its entry.
pub(in crate::engine::commands) async fn change_catalog<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
    origin: &str,
) -> Result<(), Error> {
    let entry = ds.entry(&namespace(text)?).await?;
    let mut request = Keep::new(entry.key());
    request.from = Some(entry.access);
    request.catalog = Some(remote_origin(origin)?);
    ds.keep(&request).await?;
    Ok(())
}

/// The package's folder, refused where it is missing: writing a rules file
/// there would make the folder, and a folder holding only it reads as every
/// file deleted.
pub(in crate::engine::commands) async fn folder<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
) -> Result<PathBuf, Error> {
    let folder = ds.entry(&namespace(text)?).await?.folder;
    if folder.is_dir() {
        Ok(folder)
    } else {
        Err(Error::Dataspace(format!(
            "{} is missing; the next sync reads it back",
            folder.display()
        )))
    }
}

/// Where the package's file at `key` is, refused where there is none, as
/// the ref path's open and reveal refuse it.
pub(in crate::engine::commands) async fn file_path<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
    key: &quilt_rs::LogicalKey,
) -> Result<PathBuf, Error> {
    let path = key.join_to(&ds.entry(&namespace(text)?).await?.folder);
    if path.exists() {
        Ok(path)
    } else {
        Err(Error::FsOpen(
            crate::engine::error::FsOpenError::PathNotFound(path),
        ))
    }
}

/// Adds the first line of `pattern` to the folder's own `.quiltignore`.
pub(in crate::engine::commands) async fn add_to_quiltignore<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
    pattern: &str,
) -> Result<(), Error> {
    let folder = folder(ds, text).await?;
    let line = pattern.lines().next().unwrap_or(pattern).to_string();
    let rules = ds.ignore_rules(&folder).await?;
    quilt_rs::quiltignore::append_lines(&rules.file(Scope::Package), &[line]).map_err(fail)
}

/// The rules, the files of the current revision, and `path` checked not to
/// be a nested dataspace, which no `!` line brings back (25o item 7).
async fn stop_ignoring_inputs<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
    path: &Path,
) -> Result<(PathBuf, Rules, Vec<PathBuf>), Error> {
    let folder = folder(ds, text).await?;
    if folder.join(path).join(CONTRACT_FILE).is_file() {
        return Err(Error::General(format!(
            "{} holds a {CONTRACT_FILE} of its own: it is another dataspace, never packaged",
            path.display()
        )));
    }
    let rules = ds.ignore_rules(&folder).await?;
    let read = PackageRead::of(ds, &namespace(text)?, false).await?;
    let tracked = read
        .current
        .rows
        .iter()
        .map(|row| row.logical_key.as_path().to_path_buf())
        .collect();
    Ok((folder, rules, tracked))
}

fn lines_for(
    rules: &Rules,
    tracked: &[PathBuf],
    path: &Path,
    is_dir: bool,
    scope: Scope,
) -> Result<Option<Vec<String>>, Error> {
    let tracked = if is_dir {
        tracked.iter().any(|file| file.starts_with(path))
    } else {
        tracked.iter().any(|file| file == path)
    };
    rules
        .stop_ignoring_lines(path, is_dir, tracked, scope)
        .map_err(fail)
}

/// The files `before` leaves out of `folder` and `after` does not, and
/// their bytes: what the next sync adds.
fn brought_in(folder: &Path, before: &Rules, after: &Rules, tracked: &[PathBuf]) -> (u64, u64) {
    let mut moved = (0, 0);
    let mut queue = vec![PathBuf::new()];
    while let Some(prefix) = queue.pop() {
        let Ok(listed) = std::fs::read_dir(folder.join(&prefix)) else {
            continue;
        };
        for item in listed.flatten() {
            let key = prefix.join(item.file_name());
            match item.file_type() {
                Ok(kind) if kind.is_dir() => {
                    if !item.path().join(CONTRACT_FILE).is_file() {
                        queue.push(key);
                    }
                }
                Ok(kind) if kind.is_file() => {
                    let held = tracked.contains(&key);
                    if before.file_ignored_by(&key, held).is_some()
                        && after.file_ignored_by(&key, held).is_none()
                    {
                        moved.0 += 1;
                        moved.1 += item.metadata().map_or(0, |meta| meta.len());
                    }
                }
                _ => {}
            }
        }
    }
    moved
}

fn preview(
    folder: &Path,
    rules: &Rules,
    tracked: &[PathBuf],
    path: &Path,
    is_dir: bool,
    scope: Scope,
) -> Result<Option<ScopePreview>, Error> {
    let Some(lines) = lines_for(rules, tracked, path, is_dir, scope)? else {
        return Ok(None);
    };
    let after = rules.with_lines(scope, &lines).map_err(fail)?;
    let (files, bytes) = brought_in(folder, rules, &after, tracked);
    Ok(Some(ScopePreview {
        lines,
        files,
        bytes,
    }))
}

/// What *Stop ignoring* `path` writes in each scope, and what it brings in.
pub(in crate::engine::commands) async fn stop_ignoring_preview<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
    path: &str,
    is_dir: bool,
) -> Result<StopIgnoringPreview, Error> {
    let path = PathBuf::from(path);
    let (folder, rules, tracked) = stop_ignoring_inputs(ds, text, &path).await?;
    let this_package = preview(&folder, &rules, &tracked, &path, is_dir, Scope::Package)?
        .ok_or_else(|| {
            Error::General(format!(
                "This package cannot stop ignoring {}",
                path.display()
            ))
        })?;
    Ok(StopIgnoringPreview {
        this_package,
        all_packages: preview(&folder, &rules, &tracked, &path, is_dir, Scope::AllPackages)?,
    })
}

/// Writes the `!` lines that bring `path` back in `scope`, and answers
/// them: empty when nothing ignores it, `None` when `scope` cannot.
pub(in crate::engine::commands) async fn stop_ignoring<R: Remote + Sync>(
    ds: &Dataspace<R>,
    text: &str,
    path: &str,
    is_dir: bool,
    scope: Scope,
) -> Result<Option<Vec<String>>, Error> {
    let path = PathBuf::from(path);
    let (_, rules, tracked) = stop_ignoring_inputs(ds, text, &path).await?;
    let lines = lines_for(&rules, &tracked, &path, is_dir, scope)?;
    if let Some(lines) = &lines
        && !lines.is_empty()
    {
        quilt_rs::quiltignore::append_lines(&rules.file(scope), lines).map_err(fail)?;
    }
    Ok(lines)
}

/// Renames this machine's local registry; its old name stays an alias.
pub(in crate::engine::commands) async fn rename<R: Remote + Sync>(
    ds: &Dataspace<R>,
    name: &str,
) -> Result<String, Error> {
    Ok(ds.rename(name.trim()).await?.names.name)
}

#[cfg(test)]
mod tests;
