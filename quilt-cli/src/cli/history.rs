use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Std;

use quilt_rs::io::storage::Storage;
use quilt_uri::Namespace;

#[derive(Debug)]
pub struct Input {
    pub namespace: Namespace,
}

#[derive(Debug, serde::Serialize)]
struct Revision {
    hash: String,
    timestamp: String,
    message: String,
}

pub struct Output {
    revisions: Vec<Revision>,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.revisions.is_empty() {
            return write!(f, "No revisions");
        }

        for revision in &self.revisions {
            let short_hash: String = revision.hash.chars().take(8).collect();
            let date = revision
                .timestamp
                .split('T')
                .next()
                .unwrap_or(&revision.timestamp);
            writeln!(f, "{short_hash}  {date}  {}", revision.message)?;
        }
        Ok(())
    }
}

impl Output {
    fn to_json(&self) -> String {
        serde_json::json!({ "revisions": self.revisions }).to_string()
    }
}

pub async fn command(m: impl Commands, args: Input, json: bool) -> Std {
    match m.log(args).await {
        Ok(output) => Std::Out(if json {
            output.to_json()
        } else {
            output.to_string()
        }),
        Err(error) => Std::Err(error),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { namespace }: Input,
) -> Result<Output, Error> {
    let Some(package) = local_domain.get_installed_package(&namespace).await? else {
        return Err(Error::NamespaceNotFound(namespace));
    };

    let lineage = package.lineage().await?;
    let current_commit = lineage.commit.as_ref();
    let manifest_dir = package.paths.installed_manifests_dir(&namespace);
    let mut entries = package.storage.read_dir(&manifest_dir).await?;
    let mut revisions = Vec::new();

    while let Some(entry) = entries.next_entry().await? {
        if !entry.file_type().await?.is_file() {
            continue;
        }

        let path = entry.path();
        let hash = entry.file_name().to_string_lossy().into_owned();
        let manifest = quilt_rs::manifest::Manifest::from_path(&package.storage, &path).await?;
        let timestamp = match current_commit.filter(|commit| commit.hash == hash) {
            Some(commit) => commit.timestamp.to_rfc3339(),
            None => package
                .storage
                .modified_timestamp(&path)
                .await?
                .to_rfc3339(),
        };
        let message = manifest
            .header
            .message
            .unwrap_or_else(|| "∅".to_string())
            .replace(['\n', '\r'], " ");

        revisions.push(Revision {
            hash,
            timestamp,
            message,
        });
    }

    revisions.sort_by(|left, right| {
        right
            .timestamp
            .cmp(&left.timestamp)
            .then_with(|| right.hash.cmp(&left.hash))
    });

    Ok(Output { revisions })
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
                Revision {
                    hash: "0123456789abcdef".to_string(),
                    timestamp: "2026-08-07T00:00:00Z".to_string(),
                    message: "add east region".to_string(),
                },
                Revision {
                    hash: "fedcba9876543210".to_string(),
                    timestamp: "2026-08-06T00:00:00Z".to_string(),
                    message: "initial import".to_string(),
                },
            ],
        };

        assert_eq!(
            output.to_string(),
            "01234567  2026-08-07  add east region\nfedcba98  2026-08-06  initial import\n"
        );
    }

    #[test]
    fn test_json() {
        let output = Output {
            revisions: vec![Revision {
                hash: "0123456789abcdef".to_string(),
                timestamp: "2026-08-07T00:00:00Z".to_string(),
                message: "initial import".to_string(),
            }],
        };

        assert_eq!(
            output.to_json(),
            r#"{"revisions":[{"hash":"0123456789abcdef","timestamp":"2026-08-07T00:00:00Z","message":"initial import"}]}"#
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

        let output = model(cli_model.get_local_domain(), Input { namespace }).await?;

        assert_eq!(output.revisions.len(), 2);
        assert_eq!(output.revisions[0].message, "add east region");
        assert_eq!(output.revisions[1].message, "initial import");
        Ok(())
    }
}
