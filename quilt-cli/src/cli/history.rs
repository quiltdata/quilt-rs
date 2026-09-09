use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Std;

use quilt_uri::Namespace;

#[derive(Debug)]
pub struct Input {
    pub namespace: Namespace,
}

pub struct Output {
    revisions: Vec<quilt_rs::flow::Revision>,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.revisions.is_empty() {
            return write!(f, "No revisions");
        }

        writeln!(f, "revision  obtained (UTC)       message")?;
        for revision in &self.revisions {
            let short_hash: String = revision.hash.chars().take(8).collect();
            let message = revision
                .message
                .as_deref()
                .unwrap_or("\u{2205}")
                .replace(['\n', '\r'], " ");
            writeln!(
                f,
                "{short_hash}  {}  {message}",
                revision.obtained.format("%Y-%m-%d %H:%M:%S"),
            )?;
        }
        Ok(())
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

    Ok(Output {
        revisions: package.revisions().await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use quilt_rs::flow::UserMeta;
    use quilt_rs::io::remote::WorkflowIntent;

    use crate::cli::commit;
    use crate::cli::create;
    use crate::cli::model::Commands;
    use crate::cli::model::create_model_in_temp_dir;

    #[test]
    fn test_display() {
        let output = Output {
            revisions: vec![
                quilt_rs::flow::Revision {
                    hash: "0123456789abcdef".to_string(),
                    obtained: "2026-08-07T11:24:31Z".parse().unwrap(),
                    message: Some("add east region".to_string()),
                },
                quilt_rs::flow::Revision {
                    hash: "fedcba9876543210".to_string(),
                    obtained: "2026-08-07T09:02:07Z".parse().unwrap(),
                    message: Some("initial import".to_string()),
                },
            ],
        };

        assert_eq!(
            output.to_string(),
            "revision  obtained (UTC)       message\n\
             01234567  2026-08-07 11:24:31  add east region\n\
             fedcba98  2026-08-07 09:02:07  initial import\n"
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

        assert_eq!(output.revisions.len(), 2);
        assert_eq!(
            output.revisions[0].message.as_deref(),
            Some("add east region")
        );
        assert_eq!(
            output.revisions[1].message.as_deref(),
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
        .revisions
        .pop()
        .expect("two revisions");
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
                .revisions
                .iter()
                .map(|revision| revision.message.as_deref().unwrap_or_default())
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
                .revisions
                .iter()
                .map(|revision| revision.message.as_deref().unwrap_or_default())
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
}
