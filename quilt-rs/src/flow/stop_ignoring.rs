use std::path::Path;
use std::path::PathBuf;

use crate::Res;
use crate::file_stamp::EntryKind;
use crate::lineage::PackageLineage;
use crate::manifest::Manifest;
use crate::paths::DomainPaths;
use crate::quiltignore::Rules;
use crate::quiltignore::Scope;

use super::status::Tracked;
use super::status::walk_package_home;

/// What *Stop ignoring* a path writes in one scope, and what that brings in.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StopIgnoring {
    pub scope: Scope,
    /// The rules file the lines go to.
    pub file: PathBuf,
    /// The `!` lines, outermost first.
    pub lines: Vec<String>,
    /// Files the next sync adds to the package, and their total size.
    pub files: u64,
    pub bytes: u64,
}

fn tracked(manifest: &Manifest, lineage: &PackageLineage) -> Tracked {
    Tracked::new(
        manifest
            .rows
            .iter()
            .map(|row| &row.logical_key)
            .chain(lineage.paths.keys()),
    )
}

fn lines_for(
    rules: &Rules,
    tracked: &Tracked,
    relative_path: &Path,
    is_dir: bool,
    scope: Scope,
) -> Res<Option<Vec<String>>> {
    let tracked = if is_dir {
        tracked.dirs.contains(relative_path)
    } else {
        tracked.files.contains(relative_path)
    };
    rules.stop_ignoring_lines(relative_path, is_dir, tracked, scope)
}

/// What stopping to ignore `relative_path` in `scope` would write and bring
/// in, without writing it. `None` when `scope` cannot bring it back.
///
/// Walks the whole package home, since a line in the user-wide file names
/// the path anywhere in it.
pub async fn preview_stop_ignoring(
    paths: &DomainPaths,
    manifest: &Manifest,
    lineage: &PackageLineage,
    package_home: &Path,
    relative_path: &Path,
    is_dir: bool,
    scope: Scope,
) -> Res<Option<StopIgnoring>> {
    let rules = Rules::load(package_home, &paths.user_quiltignore())?;
    let tracked = tracked(manifest, lineage);
    let Some(lines) = lines_for(&rules, &tracked, relative_path, is_dir, scope)? else {
        return Ok(None);
    };
    let file = rules.file(scope);
    let after = rules.with_lines(scope, &lines)?;
    let home = package_home.to_path_buf();
    let (files, bytes) = tokio::task::spawn_blocking(move || -> Res<(u64, u64)> {
        let walked = walk_package_home(&home, &after, &tracked)?;
        let mut moved = (0, 0);
        for (key, _, kind, stamp) in walked.entries {
            if kind == EntryKind::File && was_ignored(&rules, &tracked, &key) {
                moved.0 += 1;
                moved.1 += stamp.size;
            }
        }
        Ok(moved)
    })
    .await
    .map_err(std::io::Error::other)??;
    Ok(Some(StopIgnoring {
        scope,
        file,
        lines,
        files,
        bytes,
    }))
}

/// Whether the walk under `rules` left out the file at `key`. A file's rule
/// counts the folders above it, so this is the walk's answer too.
fn was_ignored(rules: &Rules, tracked: &Tracked, key: &Path) -> bool {
    rules
        .file_ignored_by(key, tracked.files.contains(key))
        .is_some()
}

/// Write the `!` lines that bring `relative_path` back in `scope`, and return
/// them: empty when nothing ignores it, `None` when `scope` cannot.
pub fn stop_ignoring(
    paths: &DomainPaths,
    manifest: &Manifest,
    lineage: &PackageLineage,
    package_home: &Path,
    relative_path: &Path,
    is_dir: bool,
    scope: Scope,
) -> Res<Option<Vec<String>>> {
    let rules = Rules::load(package_home, &paths.user_quiltignore())?;
    let tracked = tracked(manifest, lineage);
    let lines = lines_for(&rules, &tracked, relative_path, is_dir, scope)?;
    if let Some(lines) = &lines
        && !lines.is_empty()
    {
        crate::quiltignore::append_lines(&rules.file(scope), lines)?;
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;
    use test_log::test;

    use crate::io::remote::HostConfig;
    use crate::io::storage::mocks::MockStorage;
    use crate::lineage::Change;
    use crate::lineage::PathState;

    struct Package {
        storage: MockStorage,
        paths: DomainPaths,
        home: PathBuf,
    }

    impl Package {
        /// A package home holding each `(path, size)` as a new file.
        fn new(files: &[(&str, usize)]) -> Self {
            let storage = MockStorage::default();
            let paths = DomainPaths::new(storage.temp_dir.path().join("domain"));
            let home = storage.temp_dir.path().join("pkg");
            for (path, size) in files {
                let path = home.join(path);
                std::fs::create_dir_all(path.parent().unwrap()).unwrap();
                std::fs::write(path, vec![b'x'; *size]).unwrap();
            }
            Package {
                storage,
                paths,
                home,
            }
        }

        async fn preview(&self, path: &str, is_dir: bool, scope: Scope) -> Option<StopIgnoring> {
            preview_stop_ignoring(
                &self.paths,
                &Manifest::default(),
                &PackageLineage::default(),
                &self.home,
                Path::new(path),
                is_dir,
                scope,
            )
            .await
            .unwrap()
        }

        fn stop(&self, path: &str, is_dir: bool, scope: Scope) -> Option<Vec<String>> {
            stop_ignoring(
                &self.paths,
                &Manifest::default(),
                &PackageLineage::default(),
                &self.home,
                Path::new(path),
                is_dir,
                scope,
            )
            .unwrap()
        }
    }

    fn snakemake() -> Package {
        Package::new(&[
            (".snakemake/a", 3),
            (".snakemake/log/b", 5),
            ("x/.snakemake/c", 7),
            (".DS_Store", 11),
            ("data.csv", 13),
        ])
    }

    #[test(tokio::test)]
    async fn this_package_counts_the_folder_it_brings_back() {
        let package = snakemake();
        let preview = package
            .preview(".snakemake", true, Scope::Package)
            .await
            .unwrap();
        assert_eq!(
            preview,
            StopIgnoring {
                scope: Scope::Package,
                file: package.home.join(".quiltignore"),
                lines: vec!["!/.snakemake/".to_string()],
                files: 2,
                bytes: 8,
            }
        );
    }

    /// The user-wide line names the folder by name, so it brings back every
    /// folder of that name.
    #[test(tokio::test)]
    async fn all_packages_counts_every_folder_of_that_name() {
        let package = snakemake();
        let preview = package
            .preview(".snakemake", true, Scope::AllPackages)
            .await
            .unwrap();
        assert_eq!(preview.file, package.paths.user_quiltignore());
        assert_eq!(preview.lines, vec!["!.snakemake/".to_string()]);
        assert_eq!((preview.files, preview.bytes), (3, 15));
    }

    #[test(tokio::test)]
    async fn all_packages_is_not_offered_over_a_package_rule() {
        let package = snakemake();
        std::fs::write(package.home.join(".quiltignore"), "*.csv\n").unwrap();
        assert!(
            package
                .preview("data.csv", false, Scope::AllPackages)
                .await
                .is_none()
        );
        assert!(
            package
                .stop("data.csv", false, Scope::AllPackages)
                .is_none()
        );
        let preview = package
            .preview("data.csv", false, Scope::Package)
            .await
            .unwrap();
        assert_eq!((preview.files, preview.bytes), (1, 13));
    }

    #[test(tokio::test)]
    async fn the_written_line_brings_the_files_in() -> Res {
        let package = snakemake();
        std::fs::write(package.home.join(".quiltignore"), "*.csv").unwrap();
        assert_eq!(
            package.stop(".snakemake", true, Scope::Package),
            Some(vec!["!/.snakemake/".to_string()])
        );
        assert_eq!(
            std::fs::read_to_string(package.home.join(".quiltignore"))?,
            "*.csv\n!/.snakemake/\n"
        );
        assert_eq!(
            package.stop(".snakemake", true, Scope::Package),
            Some(vec![])
        );

        let (_, status) = crate::flow::status(
            PackageLineage::default(),
            &package.storage,
            &package.paths,
            &Manifest::default(),
            &package.home,
            HostConfig::default(),
        )
        .await?;
        for path in [".snakemake/a", ".snakemake/log/b"] {
            assert!(
                matches!(status.changes.get(Path::new(path)), Some(Change::Added(_))),
                "{path}"
            );
        }
        assert!(!status.changes.contains_key(Path::new("x/.snakemake/c")));
        Ok(())
    }

    #[test(tokio::test)]
    async fn all_packages_writes_the_user_wide_file() -> Res {
        let package = snakemake();
        assert_eq!(
            package.stop(".DS_Store", false, Scope::AllPackages),
            Some(vec!["!.DS_Store".to_string()])
        );
        assert_eq!(
            std::fs::read_to_string(package.paths.user_quiltignore())?,
            "!.DS_Store\n"
        );
        assert!(!package.home.join(".quiltignore").exists());
        Ok(())
    }

    /// A tracked file replaced by a folder of the same name: the walk checks
    /// the folder against tracked folders only.
    #[test]
    fn a_folder_where_a_tracked_file_was_is_not_tracked() {
        let package = Package::new(&[("node_modules/a.js", 3)]);
        let mut lineage = PackageLineage::default();
        lineage
            .paths
            .insert(PathBuf::from("node_modules"), PathState::default());
        let lines = stop_ignoring(
            &package.paths,
            &Manifest::default(),
            &lineage,
            &package.home,
            Path::new("node_modules"),
            true,
            Scope::Package,
        )
        .unwrap();
        assert_eq!(lines, Some(vec!["!/node_modules/".to_string()]));
    }
}
