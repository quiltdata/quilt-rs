use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Std;

use std::cmp::Ordering;
use std::collections::HashMap;

use quilt_rs::io::storage::Storage;
use quilt_uri::Namespace;

#[derive(Debug)]
pub struct Input {
    pub namespace: Namespace,
}

#[derive(Debug)]
struct Revision {
    hash: String,
    message: String,
    order: Option<usize>,
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
            writeln!(f, "{short_hash}  {}", revision.message)?;
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

    let lineage = package.lineage().await?;
    let current_commit = lineage.commit.as_ref();
    let revision_order: HashMap<String, usize> = current_commit
        .map(|commit| {
            std::iter::once(&commit.hash)
                .chain(commit.prev_hashes.iter())
                .enumerate()
                .map(|(order, hash)| (hash.clone(), order))
                .collect()
        })
        .unwrap_or_default();
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
        let message = manifest
            .header
            .message
            .unwrap_or_else(|| "∅".to_string())
            .replace(['\n', '\r'], " ");

        revisions.push(Revision {
            order: revision_order.get(&hash).copied(),
            hash,
            message,
        });
    }

    revisions.sort_by(|left, right| {
        match (left.order, right.order) {
            (Some(left_order), Some(right_order)) => left_order.cmp(&right_order),
            (Some(_), None) => Ordering::Less,
            (None, Some(_)) => Ordering::Greater,
            (None, None) => Ordering::Equal,
        }
        .then_with(|| left.hash.cmp(&right.hash))
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
                    message: "add east region".to_string(),
                    order: Some(0),
                },
                Revision {
                    hash: "fedcba9876543210".to_string(),
                    message: "initial import".to_string(),
                    order: Some(1),
                },
            ],
        };

        assert_eq!(
            output.to_string(),
            "01234567  add east region\nfedcba98  initial import\n"
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
        assert_eq!(output.revisions[0].message, "add east region");
        assert_eq!(output.revisions[1].message, "initial import");
        Ok(())
    }

    #[tokio::test]
    async fn test_model_orders_revisions_by_lineage_not_manifest_mtime() -> Result<(), Error> {
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

        let package = cli_model
            .get_local_domain()
            .get_installed_package(&namespace)
            .await?
            .unwrap();
        let lineage = package.lineage().await?;
        let current_commit = lineage.commit.unwrap();
        let previous_hash = current_commit
            .prev_hashes
            .first()
            .expect("initial revision should be tracked as previous")
            .clone();

        std::thread::sleep(std::time::Duration::from_millis(1100));
        let previous_manifest_path = package.paths.installed_manifest(&namespace, &previous_hash);
        let previous_manifest = std::fs::read(&previous_manifest_path)?;
        std::fs::write(&previous_manifest_path, previous_manifest)?;

        let output = model(
            cli_model.get_local_domain(),
            Input {
                namespace: namespace.clone(),
            },
        )
        .await?;

        assert_eq!(output.revisions.len(), 2);
        assert_eq!(output.revisions[0].hash, current_commit.hash);
        assert_eq!(output.revisions[0].message, "add east region");
        assert_eq!(output.revisions[1].hash, previous_hash);
        assert_eq!(output.revisions[1].message, "initial import");
        Ok(())
    }
}
