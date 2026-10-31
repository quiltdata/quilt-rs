//! A folder typed by hand, where no desktop gives a picker: whether it is
//! one, and the next part of its path.

use std::path::MAIN_SEPARATOR;
use std::path::Path;

use serde::Serialize;

use crate::engine::services::Services;

/// At most this many folders are offered for one prefix.
const MAX_OFFERED: usize = 50;

#[derive(Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PathCheck {
    pub exists: bool,
    pub is_dir: bool,
}

/// Whether `path` exists and is a folder.
pub async fn check_path(_s: &Services, path: String) -> Result<PathCheck, String> {
    Ok(check(Path::new(&path)).await)
}

async fn check(path: &Path) -> PathCheck {
    match tokio::fs::metadata(path).await {
        Ok(meta) => PathCheck {
            exists: true,
            is_dir: meta.is_dir(),
        },
        Err(_) => PathCheck {
            exists: false,
            is_dir: false,
        },
    }
}

/// The folders whose path starts with `prefix`, each ending in a separator,
/// sorted: `/mnt/da` offers `/mnt/data/`. Hidden folders only when the typed
/// part starts with a dot. Nothing for a relative path.
pub async fn complete_path(_s: &Services, prefix: String) -> Result<Vec<String>, String> {
    Ok(complete(&prefix).await)
}

async fn complete(prefix: &str) -> Vec<String> {
    let cut = prefix.rfind(['/', MAIN_SEPARATOR]).map_or(0, |i| i + 1);
    let (parent, part) = prefix.split_at(cut);
    if parent.is_empty() || !Path::new(parent).is_absolute() {
        return Vec::new();
    }
    let Ok(mut entries) = tokio::fs::read_dir(parent).await else {
        return Vec::new();
    };
    let mut offered = Vec::new();
    while let Ok(Some(entry)) = entries.next_entry().await {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !name.starts_with(part) || (name.starts_with('.') && !part.starts_with('.')) {
            continue;
        }
        // Follows links, as the picker does.
        if tokio::fs::metadata(entry.path())
            .await
            .is_ok_and(|meta| meta.is_dir())
        {
            offered.push(format!("{parent}{name}{MAIN_SEPARATOR}"));
        }
    }
    offered.sort();
    offered.truncate(MAX_OFFERED);
    offered
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_of(root: &Path) -> String {
        format!("{}{MAIN_SEPARATOR}", root.display())
    }

    #[tokio::test]
    async fn the_next_part_comes_from_the_subfolders() {
        let root = tempfile::tempdir().unwrap();
        for name in ["data", "dashboards", "docs", ".dotted"] {
            std::fs::create_dir(root.path().join(name)).unwrap();
        }
        std::fs::write(root.path().join("dazzle.txt"), b"").unwrap();
        let base = dir_of(root.path());

        assert_eq!(
            complete(&format!("{base}da")).await,
            vec![
                format!("{base}dashboards{MAIN_SEPARATOR}"),
                format!("{base}data{MAIN_SEPARATOR}"),
            ],
            "folders only, sorted"
        );
        assert_eq!(complete(&base).await.len(), 3, "hidden ones left out");
        assert_eq!(
            complete(&format!("{base}.")).await,
            vec![format!("{base}.dotted{MAIN_SEPARATOR}")]
        );
        assert_eq!(complete("relative/da").await, Vec::<String>::new());
        assert_eq!(complete("").await, Vec::<String>::new());
    }

    #[tokio::test]
    async fn at_most_fifty_are_offered() {
        let root = tempfile::tempdir().unwrap();
        for n in 0..60 {
            std::fs::create_dir(root.path().join(format!("d{n:02}"))).unwrap();
        }
        assert_eq!(complete(&dir_of(root.path())).await.len(), MAX_OFFERED);
    }

    #[tokio::test]
    async fn a_check_tells_a_folder_from_a_file_and_from_nothing() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("f.txt");
        std::fs::write(&file, b"").unwrap();
        let at = |exists, is_dir| PathCheck { exists, is_dir };
        assert_eq!(check(root.path()).await, at(true, true));
        assert_eq!(check(&file).await, at(true, false));
        assert_eq!(check(&root.path().join("nope")).await, at(false, false));
    }
}
