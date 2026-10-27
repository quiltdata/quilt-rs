use std::path::PathBuf;

use quilt_rs::lineage::SyncScope;
use quilt_uri::Namespace;

use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Render;
use crate::cli::output::Std;
use crate::cli::text::printable_line;

#[derive(Debug)]
pub struct Input {
    pub namespace: Namespace,
    /// `None` prints the current setting.
    pub scope: Option<SyncScope>,
    /// Whether to download the files the new scope covers.
    pub download: bool,
}

#[derive(Debug)]
pub struct Output {
    namespace: Namespace,
    scope: SyncScope,
    /// Files this command downloaded.
    downloaded: usize,
    /// Files it tried to download whose bytes the remote no longer holds.
    skipped: Vec<PathBuf>,
    /// Files the scope covers that are still not here.
    outstanding: usize,
}

/// The scope a set of `keep` or `install` flags asks for, or `None` when they
/// ask for none.
pub fn scope_from(files: bool, all: bool, folders: Vec<String>) -> Option<SyncScope> {
    if all {
        Some(SyncScope::EntirePackage)
    } else if files {
        Some(SyncScope::IndividualFiles)
    } else if folders.is_empty() {
        None
    } else {
        Some(SyncScope::folders(folders))
    }
}

/// `a`, `a and b`, `a, b and c`.
fn listed(names: &[String]) -> String {
    match names {
        [] => String::new(),
        [only] => only.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    }
}

fn files(count: usize) -> String {
    if count == 1 {
        "1 file".to_string()
    } else {
        format!("{count} files")
    }
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let kept = match &self.scope {
            SyncScope::IndividualFiles => "only the files you picked".to_string(),
            SyncScope::EntirePackage => "every file in the package".to_string(),
            SyncScope::Folders(folders) => {
                format!("every file in {}", printable_line(&listed(folders)))
            }
        };
        write!(f, "{} keeps {kept}", self.namespace)?;
        if self.downloaded > 0 {
            write!(f, "\nDownloaded {}", files(self.downloaded))?;
        }
        for path in &self.skipped {
            write!(
                f,
                "\nSkipped: \"{}\" (no longer on the remote)",
                printable_line(&path.display().to_string())
            )?;
        }
        if self.outstanding > 0 {
            write!(f, "\n{} not downloaded yet", files(self.outstanding))?;
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let mut json = serde_json::json!({
            "namespace": self.namespace.to_string(),
            "scope": match self.scope {
                SyncScope::IndividualFiles => "individual_files",
                SyncScope::EntirePackage => "entire_package",
                SyncScope::Folders(_) => "folders",
            },
        });
        if let SyncScope::Folders(folders) = &self.scope {
            json["folders"] = serde_json::json!(folders);
        }
        json["outstanding"] = serde_json::json!(self.outstanding);
        json
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.keep(args).await)
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input {
        namespace,
        scope,
        download,
    }: Input,
) -> Result<Output, Error> {
    let Some(package) = local_domain.get_installed_package(&namespace).await? else {
        return Err(Error::NamespaceNotFound(namespace));
    };
    keep(&package, scope, download).await
}

/// Store `scope`, when given, then download the files it covers that are not
/// here yet, when `download` asks to.
pub async fn keep(
    package: &quilt_rs::InstalledPackage,
    scope: Option<SyncScope>,
    download: bool,
) -> Result<Output, Error> {
    let changed = scope.is_some();
    if let Some(scope) = scope {
        package.set_sync_scope(scope).await?;
    }
    let scope = package.lineage().await?.sync_scope;
    let backlog = package.backlog().await?;
    let (downloaded, skipped, outstanding) = if changed && download && !backlog.is_empty() {
        let skipped = package.install_paths(&backlog).await?.skipped;
        (
            backlog.len() - skipped.len(),
            skipped.clone(),
            skipped.len(),
        )
    } else {
        (0, Vec::new(), backlog.len())
    };
    Ok(Output {
        namespace: package.namespace.clone(),
        scope,
        downloaded,
        skipped,
        outstanding,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::cli::create;
    use crate::cli::fixtures::packages::default as pkg;
    use crate::cli::model::create_model_in_temp_dir;
    use crate::cli::model::install_package_into_temp_dir;

    fn output(scope: SyncScope, outstanding: usize) -> Output {
        Output {
            namespace: ("team", "plates").into(),
            scope,
            downloaded: 0,
            skipped: Vec::new(),
            outstanding,
        }
    }

    #[test]
    fn the_flags_pick_one_scope() {
        assert_eq!(scope_from(false, false, Vec::new()), None);
        assert_eq!(
            scope_from(true, false, Vec::new()),
            Some(SyncScope::IndividualFiles)
        );
        assert_eq!(
            scope_from(false, true, Vec::new()),
            Some(SyncScope::EntirePackage)
        );
        assert_eq!(
            scope_from(false, false, vec!["data/".to_string()]),
            Some(SyncScope::Folders(vec!["data".to_string()]))
        );
        assert_eq!(
            scope_from(false, false, vec!["./".to_string()]),
            Some(SyncScope::EntirePackage),
            "--folder ./ is --all"
        );
    }

    #[test]
    fn the_text_says_what_is_kept_and_what_waits() {
        assert_eq!(
            output(SyncScope::IndividualFiles, 0).to_string(),
            "team/plates keeps only the files you picked"
        );
        assert_eq!(
            output(SyncScope::EntirePackage, 1).to_string(),
            "team/plates keeps every file in the package\n1 file not downloaded yet"
        );
        assert_eq!(
            Output {
                downloaded: 3,
                skipped: vec![PathBuf::from("data/gone.csv")],
                ..output(SyncScope::folders(["data", "results"]), 1)
            }
            .to_string(),
            "team/plates keeps every file in data and results\nDownloaded 3 files\n\
             Skipped: \"data/gone.csv\" (no longer on the remote)\n1 file not downloaded yet"
        );
    }

    #[test]
    fn the_json_names_the_folders_only_for_a_folders_scope() {
        assert_eq!(
            output(SyncScope::EntirePackage, 2).to_json().to_string(),
            r#"{"namespace":"team/plates","scope":"entire_package","outstanding":2}"#
        );
        assert_eq!(
            output(SyncScope::folders(["data"]), 0)
                .to_json()
                .to_string(),
            r#"{"namespace":"team/plates","scope":"folders","folders":["data"],"outstanding":0}"#
        );
    }

    /// A local package holds every file it lists, so nothing waits.
    #[test(tokio::test)]
    async fn keep_stores_the_scope_and_prints_it_back() -> Result<(), Error> {
        let (m, temp_dir) = create_model_in_temp_dir().await?;
        let source = temp_dir.path().join("source");
        std::fs::create_dir_all(source.join("data"))?;
        std::fs::write(source.join("data/a.csv"), "a")?;
        m.create(create::Input {
            namespace: ("team", "plates").into(),
            source: Some(source),
            message: None,
        })
        .await?;
        let input = |scope| Input {
            namespace: ("team", "plates").into(),
            scope,
            download: true,
        };

        let set = m
            .keep(input(Some(SyncScope::folders(["data/", "data/raw"]))))
            .await?;
        assert_eq!(set.scope, SyncScope::Folders(vec!["data".to_string()]));
        assert_eq!((set.downloaded, set.outstanding), (0, 0));

        let shown = m.keep(input(None)).await?;
        assert_eq!(shown.scope, SyncScope::Folders(vec!["data".to_string()]));
        Ok(())
    }

    #[test(tokio::test)]
    async fn keep_refuses_a_package_that_is_not_installed() -> Result<(), Error> {
        let (m, _temp_dir) = create_model_in_temp_dir().await?;
        let err = m
            .keep(Input {
                namespace: ("team", "missing").into(),
                scope: Some(SyncScope::EntirePackage),
                download: true,
            })
            .await
            .unwrap_err();
        assert!(matches!(err, Error::NamespaceNotFound(_)), "{err}");
        Ok(())
    }

    /// The package has two files, one under `one`. Keeping that folder fetches
    /// it and leaves the other.
    #[test(tokio::test)]
    async fn live_keep_downloads_the_files_under_a_kept_folder() -> Result<(), Error> {
        let (m, installed, _temp_dir) = install_package_into_temp_dir(pkg::URI).await?;

        let saved = m
            .keep(Input {
                namespace: pkg::NAMESPACE.into(),
                scope: Some(SyncScope::folders(["one"])),
                download: false,
            })
            .await?;
        assert_eq!((saved.downloaded, saved.outstanding), (0, 1));
        assert!(installed.lineage().await?.paths.is_empty());

        // A file already at the path is left for the user, not overwritten.
        let readme = installed.package_home().await?.join(pkg::README_LK);
        std::fs::create_dir_all(readme.parent().unwrap())?;
        std::fs::write(&readme, "mine")?;
        assert_eq!(installed.backlog().await?, [] as [PathBuf; 0]);
        std::fs::remove_file(&readme)?;

        let fetched = m
            .keep(Input {
                namespace: pkg::NAMESPACE.into(),
                scope: Some(SyncScope::folders(["one"])),
                download: true,
            })
            .await?;
        assert_eq!((fetched.downloaded, fetched.outstanding), (1, 0));
        let tracked: Vec<_> = installed.lineage().await?.paths.into_keys().collect();
        assert_eq!(tracked, vec![PathBuf::from(pkg::README_LK)]);
        Ok(())
    }
}
