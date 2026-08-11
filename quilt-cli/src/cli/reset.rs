use quilt_rs::lineage::CommitState;
use quilt_uri::Namespace;

use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Std;

#[derive(Debug)]
pub struct Input {
    pub namespace: Namespace,
}

#[derive(Debug)]
pub struct Output {
    pub commit: CommitState,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Reset to local revision \"{}\"", self.commit.hash)
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.reset(args).await)
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { namespace }: Input,
) -> Result<Output, Error> {
    let package = local_domain
        .get_installed_package(&namespace)
        .await?
        .ok_or_else(|| Error::NamespaceNotFound(namespace))?;
    let commit = package.reset_to_local().await?;
    Ok(Output { commit })
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::cli::commit;
    use crate::cli::create;
    use crate::cli::model::create_model_in_temp_dir;
    use quilt_rs::flow::UserMeta;
    use quilt_rs::io::storage::ByteStream;
    use quilt_rs::io::storage::LocalStorage;
    use quilt_rs::io::storage::Storage;

    #[tokio::test]
    async fn resets_a_local_only_package() -> Result<(), Error> {
        let (cli_model, _temp_dir) = create_model_in_temp_dir().await?;
        let namespace: Namespace = ("demo", "local").into();
        let created = cli_model
            .create(create::Input {
                namespace: namespace.clone(),
                source: None,
                message: Some("initial".to_string()),
            })
            .await?;
        let package_home = created.installed_package.package_home().await?;
        let storage = LocalStorage::new();

        storage
            .write_byte_stream(
                package_home.join("value.txt"),
                ByteStream::from_static(b"first"),
            )
            .await?;
        cli_model
            .commit(commit::Input {
                message: "first".to_string(),
                namespace: namespace.clone(),
                user_meta: UserMeta::Keep,
                workflow: quilt_rs::io::remote::WorkflowIntent::BucketDefault,
                host_config: None,
            })
            .await?;

        storage
            .write_byte_stream(
                package_home.join("value.txt"),
                ByteStream::from_static(b"second"),
            )
            .await?;
        cli_model
            .commit(commit::Input {
                message: "second".to_string(),
                namespace: namespace.clone(),
                user_meta: UserMeta::Keep,
                workflow: quilt_rs::io::remote::WorkflowIntent::BucketDefault,
                host_config: None,
            })
            .await?;

        let output = super::model(
            cli_model.get_local_domain(),
            Input {
                namespace: namespace.clone(),
            },
        )
        .await?;
        assert_eq!(output.commit.prev_hashes.len(), 1);
        assert_eq!(
            tokio::fs::read(package_home.join("value.txt")).await?,
            b"first"
        );
        Ok(())
    }
}
