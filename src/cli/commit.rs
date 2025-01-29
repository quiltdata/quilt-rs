use quilt_rs::lineage::CommitState;
use quilt_rs::uri::Namespace;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Debug)]
pub struct Input {
    pub message: String,
    pub namespace: Namespace,
    pub user_meta: Option<quilt_rs::manifest::JsonObject>,
}

#[derive(Debug)]
pub struct Output {
    pub commit: CommitState,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, r##"New commit "{}" created"##, self.commit.hash)
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    match m.commit(args).await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

async fn commit_package(
    local_domain: &quilt_rs::LocalDomain,
    namespace: Namespace,
    message: String,
    user_meta: Option<quilt_rs::manifest::JsonObject>,
) -> Result<CommitState, Error> {
    match local_domain.get_installed_package(&namespace).await? {
        Some(installed_package) => Ok(installed_package.commit(message, user_meta).await?),
        None => Err(Error::NamespaceNotFound(namespace)),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input {
        message,
        namespace,
        user_meta,
    }: Input,
) -> Result<Output, Error> {
    Ok(Output {
        commit: commit_package(local_domain, namespace, message, user_meta).await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;
    use temp_testdir::TempDir;

    use quilt_rs::uri::ManifestUri;
    use quilt_rs::uri::S3PackageUri;
    use quilt_rs::LocalDomain;

    #[tokio::test]
    async fn test_model() -> Result<(), Error> {
        let uri = S3PackageUri::try_from("quilt+s3://udp-spec#package=spec/quiltcore@44c3143c0964d26707651d06b9c3d4c98749b0f0044483fba45388693d227e4c&path=READ%20ME.md")?;

        // TODO: commit is not-modified when we commit the same file (timestamp.txt)
        // TODO: commit is modified when we modify a file (README.md)

        let temp_dir = TempDir::default();
        let local_path = PathBuf::from(temp_dir.as_ref());
        let local_domain = LocalDomain::new(local_path);

        let manifest_uri = ManifestUri::try_from(uri)?;
        local_domain.install_package(&manifest_uri).await?;

        let output = model(
            &local_domain,
            Input {
                message: "Test message".to_string(),
                namespace: ("spec", "quiltcore").into(),
                user_meta: None,
            },
        )
        .await?;

        assert_eq!(format!("{}", output), "New commit \"90b0f7f74a47a4ffe68aecd35dedc5c8cbeea8584c101f5c380531927d462204\" created");

        let second_commit = model(
            &local_domain,
            Input {
                message: "Test message".to_string(),
                namespace: ("spec", "quiltcore").into(),
                user_meta: None,
            },
        )
        .await?;

        assert_eq!(
            second_commit.commit.hash,
            "90b0f7f74a47a4ffe68aecd35dedc5c8cbeea8584c101f5c380531927d462204"
        );
        assert_eq!(
            second_commit.commit.prev_hashes,
            vec!["90b0f7f74a47a4ffe68aecd35dedc5c8cbeea8584c101f5c380531927d462204"]
        );

        let third_commit = model(
            &local_domain,
            Input {
                message: "New commit message".to_string(),
                namespace: ("spec", "quiltcore").into(),
                user_meta: Some(
                    serde_json::json!({"key": "value"})
                        .as_object()
                        .unwrap()
                        .clone(),
                ),
            },
        )
        .await?;

        assert_eq!(
            third_commit.commit.hash,
            "b172328d86eccc2c2ca590988297074066a1a1b52e8d9c45df386599bfe51917"
        );
        assert_eq!(
            third_commit.commit.prev_hashes,
            vec![
                "90b0f7f74a47a4ffe68aecd35dedc5c8cbeea8584c101f5c380531927d462204",
                "90b0f7f74a47a4ffe68aecd35dedc5c8cbeea8584c101f5c380531927d462204"
            ]
        );

        Ok(())
    }
}
