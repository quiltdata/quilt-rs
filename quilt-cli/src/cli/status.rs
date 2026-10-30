use quilt_rs::io::remote::HostConfig;
use quilt_rs::lineage::Change;
use quilt_rs::lineage::InstalledPackageStatus;
use quilt_rs::lineage::UpstreamState;
use quilt_rs::quiltignore::IgnoredBy;
use quilt_rs::quiltignore::RuleSource;
use quilt_uri::Namespace;

use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Render;
use crate::cli::output::Std;

#[derive(Debug)]
pub struct Input {
    pub namespace: Namespace,
    pub host_config: Option<HostConfig>,
    /// List each ignored path, not only how many there are.
    pub list_ignored: bool,
}

#[derive(Debug)]
pub struct Output {
    pub status: InstalledPackageStatus,
    pub list_ignored: bool,
}

#[derive(tabled::Tabled)]
struct StatusEntry {
    path: String,
    status: String,
}

#[derive(serde::Serialize)]
struct JsonChange {
    path: String,
    status: &'static str,
}

#[derive(tabled::Tabled)]
struct IgnoredEntry {
    path: String,
    rule: String,
    source: String,
}

#[derive(serde::Serialize)]
struct JsonIgnored {
    path: String,
    kind: &'static str,
    rule: String,
    source: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    rules_file: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    line: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    size: Option<u64>,
}

/// One ignored path: a folder ends in `/` and has no size.
struct Ignored<'a> {
    path: String,
    by: &'a IgnoredBy,
    size: Option<u64>,
}

impl Output {
    /// Ignored files and folders in one list, sorted by path.
    fn ignored(&self) -> Vec<Ignored<'_>> {
        let files = self
            .status
            .ignored_files
            .iter()
            .map(|(path, by, size)| Ignored {
                path: path.display().to_string(),
                by,
                size: Some(*size),
            });
        let dirs = self.status.ignored_dirs.iter().map(|(path, by)| Ignored {
            path: format!("{}/", path.display()),
            by,
            size: None,
        });
        let mut ignored: Vec<_> = files.chain(dirs).collect();
        ignored.sort_by(|a, b| a.path.cmp(&b.path));
        ignored
    }
}

/// How many ignored files and folders the package leaves out; `None` when
/// it leaves out nothing.
fn ignored_words(files: usize, folders: usize) -> Option<String> {
    let plural = |n: usize, one: &str| {
        if n == 1 {
            format!("1 ignored {one}")
        } else {
            format!("{n} ignored {one}s")
        }
    };
    let words = match (files, folders) {
        (0, 0) => return None,
        (files, 0) => plural(files, "file"),
        (0, folders) => plural(folders, "folder"),
        (files, folders) => format!(
            "{} and {}",
            plural(files, "file"),
            plural(folders, "folder")
        ),
    };
    Some(format!("{words} not included"))
}

/// Where a rule came from, as `file:line` for a rules file.
fn source_words(source: &RuleSource) -> String {
    match source {
        RuleSource::Default => "built-in".to_string(),
        RuleSource::User { path, line } => format!("{}:{line}", path.display()),
        RuleSource::Package { line } => format!(".quiltignore:{line}"),
    }
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let mut output: Vec<String> = Vec::new();
        let discrete_state = match self.status.upstream_state {
            UpstreamState::UpToDate => "Installed package is up to date",
            UpstreamState::Behind => "Your commits are behind the remote",
            UpstreamState::Ahead => "Your commits are ahead of the remote",
            UpstreamState::Diverged => "Your commits are detached from the remote",
            // Currently only produced by quilt-sync for packages without origin;
            // the CLI's status() call errors out before reaching this state.
            UpstreamState::Local => "Local-only package (no remote origin)",
            UpstreamState::Error => "Unable to check remote status",
        };

        output.push(discrete_state.to_string());

        if self.status.changes.is_empty() {
            output.push("No changes".to_string());
        } else {
            let entries = self
                .status
                .changes
                .iter()
                .map(|(path, change)| StatusEntry {
                    path: path.display().to_string(),
                    status: match change {
                        Change::Modified(_) => "Modified".to_string(),
                        Change::Added(_) => "Added".to_string(),
                        Change::Removed(_) => "Removed".to_string(),
                    },
                });
            let mut entries_table = tabled::Table::new(entries);
            entries_table.with(tabled::settings::Panel::header("Changes:"));
            output.push(entries_table.to_string());
        }

        let words = ignored_words(
            self.status.ignored_files.len(),
            self.status.ignored_dirs.len(),
        );
        if self.list_ignored {
            let ignored = self.ignored();
            if ignored.is_empty() {
                output.push("Nothing ignored".to_string());
            } else {
                let entries = ignored.into_iter().map(|i| IgnoredEntry {
                    path: i.path,
                    rule: i.by.pattern.clone(),
                    source: source_words(&i.by.source),
                });
                let mut ignored_table = tabled::Table::new(entries);
                ignored_table.with(tabled::settings::Panel::header("Ignored:"));
                output.push(ignored_table.to_string());
            }
            output.extend(words);
        } else if let Some(words) = words {
            output.push(format!("{words} (list them with --ignored)"));
        }
        write!(f, "{}", output.join("\n"))
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let changes: Vec<JsonChange> = self
            .status
            .changes
            .iter()
            .map(|(path, change)| JsonChange {
                // Lossless only because these are manifest keys, which the
                // JSONL manifest format cannot express as anything but UTF-8.
                path: path.display().to_string(),
                status: match change {
                    Change::Modified(_) => "modified",
                    Change::Added(_) => "added",
                    Change::Removed(_) => "removed",
                },
            })
            .collect();

        let mut json = serde_json::json!({
            "upstream_state": self.status.upstream_state,
            "changes": changes,
            "ignored": {
                "files": self.status.ignored_files.len(),
                "folders": self.status.ignored_dirs.len(),
            },
        });
        if self.list_ignored {
            let paths: Vec<JsonIgnored> = self
                .ignored()
                .into_iter()
                .map(|i| {
                    let (source, rules_file, line) = match &i.by.source {
                        RuleSource::Default => ("built-in", None, None),
                        RuleSource::User { path, line } => {
                            ("user", Some(path.display().to_string()), Some(*line))
                        }
                        RuleSource::Package { line } => ("package", None, Some(*line)),
                    };
                    JsonIgnored {
                        kind: if i.size.is_some() { "file" } else { "folder" },
                        path: i.path,
                        rule: i.by.pattern.clone(),
                        source,
                        rules_file,
                        line,
                        size: i.size,
                    }
                })
                .collect();
            json["ignored"]["paths"] = serde_json::json!(paths);
        }
        json
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.status(args).await)
}

async fn get_status(
    local_domain: &quilt_rs::LocalDomain,
    namespace: Namespace,
    host_config: Option<HostConfig>,
) -> Result<InstalledPackageStatus, Error> {
    match local_domain.get_installed_package(&namespace).await? {
        Some(installed_package) => Ok(installed_package.status(host_config).await?),
        None => Err(Error::NamespaceNotFound(namespace)),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input {
        namespace,
        host_config,
        list_ignored,
    }: Input,
) -> Result<Output, Error> {
    let status = get_status(local_domain, namespace, host_config).await?;
    Ok(Output {
        status,
        list_ignored,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use test_log::test;

    use quilt_rs::flow::UserMeta;

    use crate::cli::model::install_package_into_temp_dir;

    #[test]
    fn test_json_empty_status() {
        let output = Output {
            status: InstalledPackageStatus::new(
                UpstreamState::UpToDate,
                std::collections::BTreeMap::new(),
            ),
            list_ignored: false,
        };

        assert_eq!(
            output.to_json().to_string(),
            r#"{"upstream_state":"up_to_date","changes":[],"ignored":{"files":0,"folders":0}}"#
        );
    }

    /// One ignored path from each rule source: a folder by a built-in rule, a
    /// file by the user's rules, a file by the package's `.quiltignore`.
    fn ignored_output(list_ignored: bool) -> Output {
        let mut status =
            InstalledPackageStatus::new(UpstreamState::UpToDate, std::collections::BTreeMap::new());
        status.ignored_files = vec![
            (
                PathBuf::from("scratch.tmp"),
                IgnoredBy {
                    pattern: "*.tmp".to_string(),
                    source: RuleSource::Package { line: 3 },
                },
                12,
            ),
            (
                PathBuf::from("data/raw.bak"),
                IgnoredBy {
                    pattern: "*.bak".to_string(),
                    source: RuleSource::User {
                        path: PathBuf::from("/domain/.quiltignore"),
                        line: 2,
                    },
                },
                5,
            ),
        ];
        status.ignored_dirs = vec![(
            PathBuf::from(".git"),
            IgnoredBy {
                pattern: ".*/".to_string(),
                source: RuleSource::Default,
            },
        )];
        Output {
            status,
            list_ignored,
        }
    }

    #[test]
    fn text_counts_ignored_files_and_folders() {
        assert_eq!(
            format!("{}", ignored_output(false)),
            "Installed package is up to date\nNo changes\n\
             2 ignored files and 1 ignored folder not included (list them with --ignored)"
        );
    }

    #[test]
    fn text_lists_each_ignored_path_with_rule_and_source() {
        let text = format!("{}", ignored_output(true));
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines[0], "Installed package is up to date");
        assert_eq!(lines[1], "No changes");
        let row = |path: &str| {
            let line = lines
                .iter()
                .find(|l| l.starts_with(&format!("| {path} ")))
                .unwrap_or_else(|| panic!("no row for {path} in:\n{text}"));
            line.split('|')
                .map(str::trim)
                .filter(|cell| !cell.is_empty())
                .collect::<Vec<_>>()
        };
        assert_eq!(row(".git/"), [".git/", ".*/", "built-in"]);
        assert_eq!(
            row("data/raw.bak"),
            ["data/raw.bak", "*.bak", "/domain/.quiltignore:2"]
        );
        assert_eq!(
            row("scratch.tmp"),
            ["scratch.tmp", "*.tmp", ".quiltignore:3"]
        );
        let order: Vec<usize> = [".git/", "data/raw.bak", "scratch.tmp"]
            .iter()
            .map(|p| text.find(&format!("| {p} ")).unwrap())
            .collect();
        assert!(order.is_sorted(), "rows are sorted by path:\n{text}");
        assert_eq!(
            lines.last(),
            Some(&"2 ignored files and 1 ignored folder not included")
        );
    }

    #[test]
    fn text_says_nothing_ignored_when_listing_an_empty_set() {
        let output = Output {
            status: InstalledPackageStatus::new(
                UpstreamState::UpToDate,
                std::collections::BTreeMap::new(),
            ),
            list_ignored: true,
        };
        assert_eq!(
            format!("{output}"),
            "Installed package is up to date\nNo changes\nNothing ignored"
        );
    }

    #[test]
    fn json_lists_ignored_paths_only_with_the_flag() {
        assert_eq!(
            ignored_output(false).to_json()["ignored"],
            serde_json::json!({"files": 2, "folders": 1})
        );
        assert_eq!(
            ignored_output(true).to_json()["ignored"],
            serde_json::json!({
                "files": 2,
                "folders": 1,
                "paths": [
                    {"path": ".git/", "kind": "folder", "rule": ".*/", "source": "built-in"},
                    {
                        "path": "data/raw.bak",
                        "kind": "file",
                        "rule": "*.bak",
                        "source": "user",
                        "rules_file": "/domain/.quiltignore",
                        "line": 2,
                        "size": 5,
                    },
                    {
                        "path": "scratch.tmp",
                        "kind": "file",
                        "rule": "*.tmp",
                        "source": "package",
                        "line": 3,
                        "size": 12,
                    },
                ],
            })
        );
    }

    /// The three `Change` arms are the only hand-written mapping in the JSON,
    /// and the only place a swapped arm would go unnoticed: nothing else pins
    /// these strings.
    #[test]
    fn test_json_maps_each_change_kind() {
        let row = |key: &str| quilt_rs::manifest::ManifestRow {
            logical_key: PathBuf::from(key),
            physical_key: String::new(),
            hash: quilt_rs::object_hash::ObjectHash::default(),
            size: 0,
            meta: None,
        };
        let changes = std::collections::BTreeMap::from([
            (
                PathBuf::from("edited.csv"),
                Change::Modified(row("edited.csv")),
            ),
            (PathBuf::from("fresh.csv"), Change::Added(row("fresh.csv"))),
            (PathBuf::from("gone.csv"), Change::Removed(row("gone.csv"))),
        ]);
        let output = Output {
            status: InstalledPackageStatus::new(UpstreamState::Local, changes),
            list_ignored: false,
        };

        assert_eq!(
            output.to_json()["changes"].to_string(),
            r#"[{"path":"edited.csv","status":"modified"},{"path":"fresh.csv","status":"added"},{"path":"gone.csv","status":"removed"}]"#
        );
    }

    use quilt_rs::io::storage::ByteStream;

    use quilt_rs::io::storage::LocalStorage;
    use quilt_rs::io::storage::Storage;

    #[allow(
        clippy::too_many_lines,
        reason = "large integration test; allowed per-test so new large tests stay flagged"
    )]
    #[test(tokio::test)]
    async fn live_model() -> Result<(), Error> {
        use crate::cli::fixtures::packages::default as pkg;

        let uri = pkg::URI;

        let (m, installed_package, _temp_dir) = install_package_into_temp_dir(uri).await?;

        let readme_logical_key = PathBuf::from(pkg::README_LK);
        let timestamp_logical_key = PathBuf::from(pkg::TIMESTAMP_LK);
        installed_package
            .install_paths(&[readme_logical_key.clone(), timestamp_logical_key.clone()])
            .await?;

        {
            let local_domain = m.get_local_domain();
            let output = model(
                local_domain,
                Input {
                    namespace: pkg::NAMESPACE.into(),
                    host_config: None,
                    list_ignored: false,
                },
            )
            .await?;

            assert_eq!(
                format!("{output}"),
                "Installed package is up to date\nNo changes"
            );
        }

        let new_key = PathBuf::from("foo/bar.md");

        let working_dir = installed_package.package_home().await?;
        let storage = LocalStorage::new();

        storage
            .write_byte_stream(working_dir.join(new_key), ByteStream::default())
            .await?;

        storage
            .write_byte_stream(working_dir.join(&readme_logical_key), ByteStream::default())
            .await?;

        {
            let local_domain = m.get_local_domain();
            let status_new_files = model(
                local_domain,
                Input {
                    namespace: pkg::NAMESPACE.into(),
                    host_config: None,
                    list_ignored: false,
                },
            )
            .await?;

            let status_new_files_str = format!("{status_new_files}");
            assert!(status_new_files_str.contains("Installed package is up to date"));
            assert!(
                status_new_files_str.contains("foo/bar.md                               | Added")
            );
            assert!(
                status_new_files_str
                    .contains("one/two two/three three three/READ ME.md | Modified")
            );
        }

        storage
            .remove_file(working_dir.join(readme_logical_key))
            .await
            .map_err(|e| Error::Test(format!("Failed to remove file: {e}")))?;

        {
            let local_domain = m.get_local_domain();
            let status_file_removed = model(
                local_domain,
                Input {
                    namespace: pkg::NAMESPACE.into(),
                    host_config: None,
                    list_ignored: false,
                },
            )
            .await?;

            let file_removed_status_str = format!("{status_file_removed}");
            assert!(file_removed_status_str.contains("Installed package is up to date"));
            assert!(
                file_removed_status_str
                    .contains("foo/bar.md                               | Added")
            );
            assert!(
                file_removed_status_str
                    .contains("one/two two/three three three/READ ME.md | Removed")
            );
        }

        {
            let local_domain = m.get_local_domain();
            let not_found = model(
                local_domain,
                Input {
                    namespace: ("a", "b").into(),
                    host_config: None,
                    list_ignored: false,
                },
            )
            .await;

            assert_eq!(not_found.unwrap_err().to_string(), "Package a/b not found");
        }

        installed_package
            .commit("Anything".to_string(), UserMeta::Keep, None, None)
            .await?;
        {
            let local_domain = m.get_local_domain();
            let status_ahead = model(
                local_domain,
                Input {
                    namespace: pkg::NAMESPACE.into(),
                    host_config: None,
                    list_ignored: false,
                },
            )
            .await?;
            assert_eq!(
                format!("{status_ahead}"),
                "Your commits are ahead of the remote\nNo changes"
            );
        }

        Ok(())
    }

    /// Default rules leave clutter out of a real package: a `.DS_Store` file
    /// and a `.git/` folder, listed as one row.
    #[test(tokio::test)]
    async fn live_model_lists_ignored_paths() -> Result<(), Error> {
        use crate::cli::fixtures::packages::default as pkg;

        let (m, installed_package, _temp_dir) = install_package_into_temp_dir(pkg::URI).await?;
        let working_dir = installed_package.package_home().await?;
        let storage = LocalStorage::new();
        storage
            .write_byte_stream(working_dir.join(".DS_Store"), ByteStream::default())
            .await?;
        storage
            .write_byte_stream(working_dir.join(".git/HEAD"), ByteStream::default())
            .await?;

        let output = model(
            m.get_local_domain(),
            Input {
                namespace: pkg::NAMESPACE.into(),
                host_config: None,
                list_ignored: true,
            },
        )
        .await?;
        let text = format!("{output}");
        assert!(
            text.contains("| .DS_Store | .DS_Store | built-in |"),
            "{text}"
        );
        assert!(
            text.contains("| .git/     | .*/       | built-in |"),
            "{text}"
        );
        assert!(!text.contains(".git/HEAD"), "{text}");
        assert!(
            text.ends_with("1 ignored file and 1 ignored folder not included"),
            "{text}"
        );

        Ok(())
    }

    #[test(tokio::test)]
    async fn live_model_when_latest_is_outdated() -> Result<(), Error> {
        use crate::cli::fixtures::packages::outdated as pkg;

        let uri = pkg::URI;

        let (m, installed_package, _temp_dir) = install_package_into_temp_dir(uri).await?;

        {
            let local_domain = m.get_local_domain();
            let output = model(
                local_domain,
                Input {
                    namespace: pkg::NAMESPACE.into(),
                    host_config: None,
                    list_ignored: false,
                },
            )
            .await?;

            assert_eq!(
                format!("{output}"),
                "Your commits are behind the remote\nNo changes"
            );
        }

        installed_package
            .commit("Anything".to_string(), UserMeta::Keep, None, None)
            .await?;

        {
            let local_domain = m.get_local_domain();
            let output = model(
                local_domain,
                Input {
                    namespace: pkg::NAMESPACE.into(),
                    host_config: None,
                    list_ignored: false,
                },
            )
            .await?;

            assert_eq!(
                format!("{output}"),
                "Your commits are detached from the remote\nNo changes"
            );
        }

        Ok(())
    }
}
