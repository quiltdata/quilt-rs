use std::collections::BTreeSet;

use tracing::log;

use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Render;
use crate::cli::output::Std;

use quilt_rs::InstalledPackage;
use quilt_rs::flow::Kept;
use quilt_rs::flow::Revision;
use quilt_rs::io::remote::Remote;
use quilt_rs::io::storage::Storage;
use quilt_uri::Namespace;

#[derive(Debug)]
pub struct Input {
    pub namespace: Namespace,
}

/// One revision this copy holds, with what `remove-revisions` would make of
/// it.
pub struct Row {
    revision: Revision,
    /// Why it is kept, in [`Kept`]'s order; empty when it is removable.
    kept: Vec<Kept>,
    /// What removing it alone frees. `None` when it is kept, or when the
    /// measure could not be read.
    frees: Option<u64>,
}

pub struct Output {
    rows: Vec<Row>,
    /// Whether `kept` and `frees` were worked out. `false` when the registry
    /// listing failed: without it an unpublished revision can't be told
    /// apart, so the log leaves both out rather than guess.
    judged: bool,
}

impl Row {
    fn kept_words(&self) -> String {
        self.kept
            .iter()
            .map(ToString::to_string)
            .collect::<Vec<_>>()
            .join(" \u{b7} ")
    }

    fn frees_words(&self) -> String {
        self.frees
            .map(quilt_rs::flow::format_bytes)
            .unwrap_or_default()
    }
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.rows.is_empty() {
            return write!(f, "No revisions");
        }

        // Widths in chars, which is what `{:<w$}` pads by: `·` is one.
        let width = |words: &dyn Fn(&Row) -> String, heading: &str| {
            self.rows
                .iter()
                .map(|row| words(row).chars().count())
                .chain([heading.len()])
                .max()
                .unwrap_or_default()
        };
        let kept_width = width(&Row::kept_words, "kept");
        let frees_width = width(&Row::frees_words, "frees");

        write!(f, "revision  obtained (UTC)       ")?;
        if self.judged {
            write!(f, "{:<kept_width$}  {:<frees_width$}  ", "kept", "frees")?;
        }
        writeln!(f, "message")?;
        for row in &self.rows {
            let revision = &row.revision;
            let short_hash: String = revision.hash.chars().take(8).collect();
            let message = revision
                .message
                .as_deref()
                .unwrap_or("\u{2205}")
                .replace(['\n', '\r'], " ");
            write!(
                f,
                "{short_hash}  {}  ",
                revision.obtained.format("%Y-%m-%d %H:%M:%S"),
            )?;
            if self.judged {
                write!(
                    f,
                    "{:<kept_width$}  {:<frees_width$}  ",
                    row.kept_words(),
                    row.frees_words()
                )?;
            }
            writeln!(f, "{message}")?;
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let revisions: Vec<_> = self
            .rows
            .iter()
            .map(|row| {
                let kept = self
                    .judged
                    .then(|| row.kept.iter().map(ToString::to_string).collect::<Vec<_>>());
                serde_json::json!({
                    "hash": &row.revision.hash,
                    "obtained": row.revision.obtained.to_rfc3339(),
                    "message": row.revision.message.as_deref(),
                    "kept": kept,
                    "frees": row.frees,
                })
            })
            .collect();
        serde_json::json!({ "revisions": revisions })
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.log(args).await)
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { namespace }: Input,
) -> Result<Output, Error> {
    let Some(package) = local_domain.get_installed_package(&namespace).await? else {
        return Err(Error::NamespaceNotFound(namespace));
    };
    read(&package).await
}

/// `package`'s revisions, newest first, each with why it is kept
/// ([`quilt_rs::flow::protection`]) and what removing it alone frees.
///
/// A registry listing that fails is not the log's failure: it warns on
/// stderr and leaves `kept` and `frees` out. A measure that fails leaves only
/// `frees` out.
pub async fn read<S: Storage + Clone + Sync, R: Remote>(
    package: &InstalledPackage<S, R>,
) -> Result<Output, Error> {
    let namespace = &package.namespace;
    let lineage = package.lineage().await?;
    let history = match package.revision_history(&lineage).await {
        Ok(history) => history,
        Err(err) => {
            log::warn!(
                "Could not read the registry of {namespace}, so the log leaves out what is kept and what removing frees: {err}"
            );
            let rows = package
                .revisions()
                .await?
                .into_iter()
                .map(|revision| Row {
                    revision,
                    kept: Vec::new(),
                    frees: None,
                })
                .collect();
            return Ok(Output {
                rows,
                judged: false,
            });
        }
    };
    let kept = quilt_rs::flow::protection(&lineage, &history);
    let usage = match package.revision_usage().await {
        Ok(usage) => Some(usage),
        Err(err) => {
            log::warn!("Could not measure what removing revisions of {namespace} frees: {err}");
            None
        }
    };
    let rows = history
        .into_iter()
        .map(|entry| {
            let reasons = kept.get(&entry.revision.hash).cloned().unwrap_or_default();
            let frees = usage
                .as_ref()
                .filter(|_| reasons.is_empty())
                .map(|usage| usage.frees(&BTreeSet::from([entry.revision.hash.clone()])));
            Row {
                revision: entry.revision,
                kept: reasons,
                frees,
            }
        })
        .collect();
    Ok(Output { rows, judged: true })
}

#[cfg(test)]
mod tests {
    use super::*;

    use quilt_rs::flow::UserMeta;
    use quilt_rs::io::remote::WorkflowIntent;

    use test_log::test;

    use crate::cli::commit;
    use crate::cli::create;
    use crate::cli::fixtures::old_revisions;
    use crate::cli::model::Commands;
    use crate::cli::model::create_model_in_temp_dir;

    fn row(hash: &str, obtained: &str, message: &str, kept: &[Kept], frees: Option<u64>) -> Row {
        Row {
            revision: Revision {
                hash: hash.to_string(),
                obtained: obtained.parse().expect("a time"),
                message: Some(message.to_string()),
            },
            kept: kept.to_vec(),
            frees,
        }
    }

    fn rows() -> Vec<Row> {
        vec![
            row(
                "a1b2c3d4e5f6",
                "2026-10-01T09:12:00Z",
                "fix labels",
                &[Kept::Current, Kept::NotPushed],
                None,
            ),
            row(
                "e5f6a7b8c9d0",
                "2026-09-28T14:03:11Z",
                "v3",
                &[Kept::Latest, Kept::Base],
                None,
            ),
            row(
                "0c9d8e7f6a5b",
                "2026-09-20T10:40:52Z",
                "v2",
                &[],
                Some(210_400),
            ),
        ]
    }

    /// A kept row says why and frees nothing; a removable one says what
    /// removing it frees.
    #[test]
    fn test_display() {
        let output = Output {
            rows: rows(),
            judged: true,
        };

        assert_eq!(
            output.to_string(),
            "revision  obtained (UTC)       kept                  frees     message\n\
             a1b2c3d4  2026-10-01 09:12:00  current \u{b7} not pushed            fix labels\n\
             e5f6a7b8  2026-09-28 14:03:11  latest \u{b7} base                   v3\n\
             0c9d8e7f  2026-09-20 10:40:52                        210.4 kB  v2\n"
        );
    }

    /// Without the registry listing the log leaves both columns out.
    #[test]
    fn test_display_unjudged() {
        let output = Output {
            rows: rows()
                .into_iter()
                .map(|row| Row {
                    kept: Vec::new(),
                    frees: None,
                    ..row
                })
                .collect(),
            judged: false,
        };

        assert_eq!(
            output.to_string(),
            "revision  obtained (UTC)       message\n\
             a1b2c3d4  2026-10-01 09:12:00  fix labels\n\
             e5f6a7b8  2026-09-28 14:03:11  v3\n\
             0c9d8e7f  2026-09-20 10:40:52  v2\n"
        );
        assert_eq!(
            output.to_json()["revisions"][0]["kept"],
            serde_json::Value::Null
        );
        assert_eq!(
            output.to_json()["revisions"][0]["frees"],
            serde_json::Value::Null
        );
    }

    #[tokio::test]
    async fn test_model_lists_local_revisions() -> Result<(), Error> {
        let (cli_model, _domain_temp_dir) = create_model_in_temp_dir().await?;
        let namespace = Namespace::from(("demo", "sales"));

        cli_model
            .create(create::Input {
                namespace: namespace.clone(),
                source: None,
                message: Some("initial import".to_string()),
            })
            .await?;

        let package = cli_model
            .get_local_domain()
            .get_installed_package(&namespace)
            .await?
            .unwrap();
        std::fs::write(package.package_home().await?.join("data.txt"), "one")?;

        cli_model
            .commit(commit::Input {
                message: "add east region".to_string(),
                namespace: namespace.clone(),
                user_meta: UserMeta::Keep,
                workflow: WorkflowIntent::NoWorkflow,
                host_config: None,
            })
            .await?;

        let output = model(
            cli_model.get_local_domain(),
            Input {
                namespace: namespace.clone(),
            },
        )
        .await?;

        assert_eq!(output.rows.len(), 2);
        // No remote means no registry: every revision is unpublished, so kept.
        assert!(
            output
                .rows
                .iter()
                .all(|row| !row.kept.is_empty() && row.frees.is_none()),
            "{output}"
        );
        assert_eq!(
            output.rows[0].revision.message.as_deref(),
            Some("add east region")
        );
        assert_eq!(
            output.rows[1].revision.message.as_deref(),
            Some("initial import")
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_model_orders_by_acquisition_time_not_lineage() -> Result<(), Error> {
        let (cli_model, _domain_temp_dir) = create_model_in_temp_dir().await?;
        let namespace = Namespace::from(("demo", "sales"));

        cli_model
            .create(create::Input {
                namespace: namespace.clone(),
                source: None,
                message: Some("r1 initial import".to_string()),
            })
            .await?;

        let package = cli_model
            .get_local_domain()
            .get_installed_package(&namespace)
            .await?
            .unwrap();
        std::fs::write(package.package_home().await?.join("data.txt"), "one")?;

        cli_model
            .commit(commit::Input {
                message: "r2 add east".to_string(),
                namespace: namespace.clone(),
                user_meta: UserMeta::Keep,
                workflow: WorkflowIntent::NoWorkflow,
                host_config: None,
            })
            .await?;

        let oldest = model(
            cli_model.get_local_domain(),
            Input {
                namespace: namespace.clone(),
            },
        )
        .await?
        .rows
        .pop()
        .expect("two revisions")
        .revision;
        assert_eq!(oldest.message.as_deref(), Some("r1 initial import"));

        // `obtained` is this copy's acquisition time, so re-acquiring the oldest
        // revision moves it to the top. This is the documented meaning of the
        // column, not an ordering bug: nothing on disk records when a revision
        // was *made*.
        // Rewriting the same bytes bumps the mtime; the manifest is
        // content-addressed, so its name and contents still agree.
        let manifest = package.paths.installed_manifest(&namespace, &oldest.hash);
        let bytes = std::fs::read(&manifest)?;
        std::fs::write(&manifest, &bytes)?;

        let output = model(cli_model.get_local_domain(), Input { namespace }).await?;
        assert_eq!(
            output
                .rows
                .iter()
                .map(|row| row.revision.message.as_deref().unwrap_or_default())
                .collect::<Vec<_>>(),
            ["r1 initial import", "r2 add east"]
        );
        Ok(())
    }

    #[tokio::test]
    async fn test_model_orders_four_successive_commits_newest_first() -> Result<(), Error> {
        let (cli_model, _domain_temp_dir) = create_model_in_temp_dir().await?;
        let namespace = Namespace::from(("demo", "sales"));

        cli_model
            .create(create::Input {
                namespace: namespace.clone(),
                source: None,
                message: Some("r1 initial import".to_string()),
            })
            .await?;

        let package = cli_model
            .get_local_domain()
            .get_installed_package(&namespace)
            .await?
            .unwrap();
        let home = package.package_home().await?;

        for message in ["r2 add east", "r3 add west", "r4 fix header"] {
            std::fs::write(home.join("data.txt"), message)?;
            cli_model
                .commit(commit::Input {
                    message: message.to_string(),
                    namespace: namespace.clone(),
                    user_meta: UserMeta::Keep,
                    workflow: WorkflowIntent::NoWorkflow,
                    host_config: None,
                })
                .await?;
        }

        let output = model(cli_model.get_local_domain(), Input { namespace }).await?;

        let expected = [
            "r4 fix header",
            "r3 add west",
            "r2 add east",
            "r1 initial import",
        ];
        assert_eq!(
            output
                .rows
                .iter()
                .map(|row| row.revision.message.as_deref().unwrap_or_default())
                .collect::<Vec<_>>(),
            expected
        );

        // And the same order in the rendered output a user actually sees,
        // past the heading row.
        assert_eq!(
            output
                .to_string()
                .lines()
                .skip(1)
                .map(|line| line
                    .rsplit_once("  ")
                    .expect("columns are two-space separated")
                    .1
                    .to_string())
                .collect::<Vec<_>>(),
            expected
        );
        Ok(())
    }

    fn revision(hash: &str, message: Option<&str>) -> Row {
        Row {
            revision: Revision {
                hash: hash.to_string(),
                obtained: std::time::SystemTime::UNIX_EPOCH.into(),
                message: message.map(str::to_string),
            },
            kept: Vec::new(),
            frees: None,
        }
    }

    #[test]
    fn json_empty_history() {
        let output = Output {
            rows: Vec::new(),
            judged: true,
        };

        assert_eq!(output.to_json().to_string(), r#"{"revisions":[]}"#);
    }

    /// The table shows eight characters and a zoneless local-looking time.
    /// Neither is usable as input, so JSON carries the full hash and RFC 3339.
    #[test]
    fn json_carries_full_hashes_and_rfc3339_times() {
        let output = Output {
            rows: vec![
                revision("0123456789abcdef", Some("first")),
                Row {
                    kept: vec![Kept::Current, Kept::NotPushed],
                    ..revision("fedcba9876543210", None)
                },
            ],
            judged: true,
        };

        assert_eq!(
            output.to_json().to_string(),
            r#"{"revisions":[{"hash":"0123456789abcdef","obtained":"1970-01-01T00:00:00+00:00","message":"first","kept":[],"frees":null},{"hash":"fedcba9876543210","obtained":"1970-01-01T00:00:00+00:00","message":null,"kept":["current","not pushed"],"frees":null}]}"#
        );
    }

    /// Over a registry: the current and the unpublished revisions say why
    /// they are kept, and each old one what removing it alone frees.
    #[test(tokio::test)]
    async fn kept_and_frees_over_a_registry() -> Result<(), Error> {
        let (package, _dirs) = old_revisions::package().await?;

        let output = read(&package).await?;

        let said: Vec<(&str, String, Option<u64>)> = output
            .rows
            .iter()
            .map(|row| (row.revision.hash.as_str(), row.kept_words(), row.frees))
            .collect();
        assert_eq!(
            said,
            [
                ("local-rev", "unpublished".to_string(), None),
                (
                    "current-rev",
                    "current \u{b7} latest \u{b7} base".to_string(),
                    None
                ),
                ("old-3", String::new(), Some(0)),
                ("old-2", String::new(), Some(1_500)),
                ("old-1", String::new(), Some(210_400)),
            ]
        );
        let json = output.to_json();
        assert_eq!(
            json["revisions"][1]["kept"],
            serde_json::json!(["current", "latest", "base"])
        );
        assert_eq!(json["revisions"][1]["frees"], serde_json::Value::Null);
        assert_eq!(json["revisions"][4]["kept"], serde_json::json!([]));
        assert_eq!(json["revisions"][4]["frees"], 210_400);
        assert!(output.to_string().contains("210.4 kB"), "{output}");
        Ok(())
    }

    /// A registry that can't be listed doesn't fail the log: it lists the
    /// revisions without the kept and frees columns.
    #[test(tokio::test)]
    async fn an_unreadable_registry_leaves_kept_and_frees_out() -> Result<(), Error> {
        let (package, _dirs) = old_revisions::package_with_a_broken_registry().await?;

        let output = read(&package).await?;

        assert!(!output.judged);
        assert_eq!(output.rows.len(), 5);
        assert!(
            output
                .to_string()
                .starts_with("revision  obtained (UTC)       message\n"),
            "{output}"
        );
        assert_eq!(
            output.to_json()["revisions"][0]["kept"],
            serde_json::Value::Null
        );
        assert_eq!(
            output.to_json()["revisions"][0]["frees"],
            serde_json::Value::Null
        );
        Ok(())
    }
}
