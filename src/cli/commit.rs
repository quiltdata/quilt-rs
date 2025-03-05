use quilt_rs::lineage::CommitState;
use quilt_rs::uri::Namespace;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Clone, Debug)]
pub struct Input {
    pub message: String,
    pub namespace: Namespace,
    pub user_meta: Option<quilt_rs::manifest::JsonObject>,
    pub workflow: Option<String>,
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
    workflow_id: Option<String>,
) -> Result<CommitState, Error> {
    match local_domain.get_installed_package(&namespace).await? {
        Some(installed_package) => {
            let workflow = installed_package.resolve_workflow(workflow_id).await?;
            Ok(installed_package
                .commit(message, user_meta, workflow)
                .await?)
        }
        None => Err(Error::NamespaceNotFound(namespace)),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input {
        message,
        namespace,
        user_meta,
        workflow,
    }: Input,
) -> Result<Output, Error> {
    let commit = commit_package(local_domain, namespace, message, user_meta, workflow).await?;
    Ok(Output { commit })
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::path::PathBuf;

    use test_log::test;

    use crate::cli::model::install_package_into_temp_dir;

    use quilt_rs::io::storage::LocalStorage;
    use quilt_rs::io::storage::Storage;

    /// Verify the commit of that package:
    ///  * workflow/config.yml exists
    ///  * workflow id is not set
    ///  * no files to commit,
    #[test(tokio::test)]
    async fn test_commit_package_with_message_and_null_workflow() -> Result<(), Error> {
        use crate::cli::fixtures::packages::workflow_null as pkg;

        let uri = pkg::URI;
        let (m, _installed_package, _tempdir) = install_package_into_temp_dir(uri).await?;
        {
            let local_domain = m.get_local_domain();

            let output = model(
                local_domain,
                Input {
                    message: pkg::MESSAGE.to_string(),
                    namespace: pkg::NAMESPACE.into(),
                    user_meta: None,
                    workflow: None,
                },
            )
            .await?;

            assert_eq!(output.commit.hash, pkg::TOP_HASH);
        }

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_commit_package_with_workflow_and_meta() -> Result<(), Error> {
        use crate::cli::fixtures::packages::my_workflow as pkg;

        let uri = pkg::URI;
        let (m, _installed_package, _tempdir) = install_package_into_temp_dir(uri).await?;
        {
            let local_domain = m.get_local_domain();

            let output = model(
                local_domain,
                Input {
                    message: pkg::MESSAGE.to_string(),
                    namespace: pkg::NAMESPACE.into(),
                    user_meta: Some(
                        serde_json::json!({
                            "Date": "2025-12-31",
                            "Name": "Foo",
                            "Owner": "Kevin",
                            "Type": "NGS"
                        })
                        .as_object()
                        .unwrap()
                        .clone(),
                    ),
                    workflow: Some("my-workflow".to_string()),
                },
            )
            .await?;

            assert_eq!(output.commit.hash, pkg::TOP_HASH);
        }

        Ok(())
    }

    /// Verify the commit of that package:
    ///  * workflow/config.yml DOESN'T exists
    ///  * workflow id is not set
    ///  * no files to commit,
    #[test(tokio::test)]
    async fn test_commit_package_with_message_only() -> Result<(), Error> {
        use crate::cli::fixtures::packages::no_workflows_message_only as pkg;

        let uri = pkg::URI;
        let (m, _installed_package, _tempdir) = install_package_into_temp_dir(uri).await?;
        {
            let local_domain = m.get_local_domain();

            let output = model(
                local_domain,
                Input {
                    message: pkg::MESSAGE.to_string(),
                    namespace: pkg::NAMESPACE.into(),
                    user_meta: None,
                    workflow: None,
                },
            )
            .await?;

            assert_eq!(output.commit.hash, pkg::TOP_HASH);
        }

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_throwing_error_when_workflow_set_but_no_workflows_config() -> Result<(), Error> {
        use crate::cli::fixtures::packages::no_workflows_message_only as pkg;

        let uri = pkg::URI;
        let (m, _installed_package, _tempdir) = install_package_into_temp_dir(uri).await?;
        {
            let local_domain = m.get_local_domain();

            let output = model(
                local_domain,
                Input {
                    message: pkg::MESSAGE.to_string(),
                    namespace: pkg::NAMESPACE.into(),
                    user_meta: None,
                    workflow: Some("Anything".to_string()),
                },
            )
            .await;

            assert_eq!(
                output.unwrap_err().to_string(),
                r#"quilt_rs error: Workflow error: There is no workflows config, but the workflow "Anything" is set"#
            );
        }

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_commit_package_with_meta_only() -> Result<(), Error> {
        use crate::cli::fixtures::packages::no_workflows_with_meta as pkg;

        let uri = pkg::URI;
        let (m, _installed_package, _tempdir) = install_package_into_temp_dir(uri).await?;
        {
            let local_domain = m.get_local_domain();

            let output = model(
                local_domain,
                Input {
                    message: "Initial".to_string(),
                    namespace: pkg::NAMESPACE.into(),
                    user_meta: Some(
                        serde_json::json!({
                            // NOTE: will be sorted
                            "C": "D",
                            "c": "d",
                            "a": "b",
                            "A": "B",
                            "e": 123,
                            "f": null
                        })
                        .as_object()
                        .unwrap()
                        .clone(),
                    ),
                    workflow: None,
                },
            )
            .await?;

            assert_eq!(output.commit.hash, pkg::TOP_HASH);
        }

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_model() -> Result<(), Error> {
        let uri = "quilt+s3://udp-spec#package=spec/quilt-rs@11c5f6dbd1bf1d8675c18aaaa963b2f0dced2f892c7406fa36c9cd17d3d31b73";

        // TODO: commit is not-modified when we commit the same file (timestamp.txt)
        // TODO: commit is modified when we modify a file (README.md)
        // let readme_logical_key = PathBuf::from("READ ME.md");
        let timestamp_logical_key = PathBuf::from("timestamp.txt");

        let (m, installed_package, _temp_dir) = install_package_into_temp_dir(uri).await?;

        let first_input = Input {
            message: "Test message".to_string(),
            namespace: ("spec", "quilt-rs").into(),
            user_meta: None,
            workflow: None,
        };
        let hash_for_initial_test_commit =
            "d6e62c3c43ddd30447d99eede1c7280c017b15cc716037b74af7bb5230fbb61a";

        {
            let local_domain = m.get_local_domain();

            let output = model(local_domain, first_input.clone())
                .await
                .expect("Failed to commit");

            assert_eq!(
                format!("{}", output),
                format!("New commit \"{}\" created", hash_for_initial_test_commit)
            );
        }

        {
            let local_domain = m.get_local_domain();
            let second_commit = model(local_domain, first_input)
                .await
                .expect("Failed to commit second commit which is identical to the first one");

            assert_eq!(second_commit.commit.hash, hash_for_initial_test_commit);
            assert_eq!(
                second_commit.commit.prev_hashes,
                vec![hash_for_initial_test_commit]
            );
        }

        {
            let local_domain = m.get_local_domain();
            let third_commit = model(
                local_domain,
                Input {
                    message: "New commit message".to_string(),
                    namespace: ("spec", "quilt-rs").into(),
                    user_meta: Some(
                        serde_json::json!({"key": "value"})
                            .as_object()
                            .unwrap()
                            .clone(),
                    ),
                    workflow: None,
                },
            )
            .await
            .expect("Failed to commit third commit different from the first one");

            assert_eq!(
                third_commit.commit.hash,
                "e2a86408670c7a33f78758d72166333e4a96b6aadbb3b03d25fd6e209dc6e0b3"
            );
            assert_eq!(
                third_commit.commit.prev_hashes,
                vec![hash_for_initial_test_commit, hash_for_initial_test_commit]
            );
        }

        {
            let local_domain = m.get_local_domain();
            let not_found = model(
                local_domain,
                Input {
                    message: "Anything".to_string(),
                    namespace: ("a", "b").into(),
                    user_meta: None,
                    workflow: None,
                },
            )
            .await;

            assert_eq!(not_found.unwrap_err().to_string(), "Package a/b not found");
        }

        let working_dir = installed_package.working_folder();
        let storage = LocalStorage::new();
        storage
            .write_file(working_dir.join(timestamp_logical_key), b"1697916638")
            .await
            .expect("Failed to write timestamp.txt to the installed package working directory");
        {
            let local_domain = m.get_local_domain();
            let commit_the_same_file = model(
                local_domain,
                Input {
                    message: "Test message".to_string(),
                    namespace: ("spec", "quilt-rs").into(),
                    user_meta: None,
                    workflow: None,
                },
            )
            .await
            .expect("Failed to commit the same file ensuring the commit hash will persist");
            assert_eq!(
                commit_the_same_file.commit.hash,
                hash_for_initial_test_commit
            );
        }

        Ok(())
    }
}
