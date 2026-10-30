//! The folders of one directory, for the UI's own folder browser where the
//! native picker is not available (a browser talking to a headless engine).

use std::path::{Path, PathBuf};

use serde::Serialize;

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct FolderListing {
    pub path: String,
    pub parent: Option<String>,
    /// Names of the visible subfolders, sorted ignoring case.
    pub folders: Vec<String>,
    pub home: String,
}

fn list_folders_in(path: &Path, home: &Path) -> Result<FolderListing, String> {
    let path = path
        .canonicalize()
        .map_err(|err| format!("{}: {err}", path.display()))?;
    if !path.is_dir() {
        return Err(format!("{} is not a folder", path.display()));
    }
    let entries = std::fs::read_dir(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    let mut folders: Vec<String> = entries
        .filter_map(Result::ok)
        .filter(|entry| entry.file_type().is_ok_and(|t| t.is_dir()))
        .filter_map(|entry| entry.file_name().into_string().ok())
        .filter(|name| !name.starts_with('.'))
        .collect();
    folders.sort_by_key(|name| name.to_lowercase());
    Ok(FolderListing {
        parent: path.parent().map(|p| p.display().to_string()),
        path: path.display().to_string(),
        folders,
        home: home.display().to_string(),
    })
}

#[allow(
    clippy::unused_async,
    reason = "every plain command is async, so the IPC wrappers and the dispatcher treat them alike"
)]
pub async fn list_folders(path: Option<String>) -> Result<FolderListing, String> {
    let home = dirs::home_dir().ok_or("No home directory")?;
    let path = path
        .filter(|p| !p.trim().is_empty())
        .map_or_else(|| home.clone(), |p| PathBuf::from(p.trim()));
    list_folders_in(&path, &home)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lists_visible_folders_sorted() {
        let dir = tempfile::tempdir().unwrap();
        for name in ["beta", "Alpha", ".hidden", "gamma"] {
            std::fs::create_dir(dir.path().join(name)).unwrap();
        }
        std::fs::write(dir.path().join("file.txt"), "x").unwrap();
        let listing = list_folders_in(dir.path(), dir.path()).unwrap();
        assert_eq!(listing.folders, ["Alpha", "beta", "gamma"]);
        assert!(listing.parent.is_some());

        let err = list_folders_in(&dir.path().join("file.txt"), dir.path()).unwrap_err();
        assert!(err.contains("not a folder"), "{err}");
        assert!(list_folders_in(&dir.path().join("missing"), dir.path()).is_err());
    }
}
