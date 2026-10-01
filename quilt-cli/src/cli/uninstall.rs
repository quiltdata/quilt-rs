use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Render;
use crate::cli::output::Std;

use quilt_rs::flow::Pruned;
use quilt_uri::Namespace;

#[derive(Debug)]
pub struct Input {
    pub namespace: Namespace,
    /// Also delete the package's objects no other installed package uses.
    pub prune: bool,
}

pub struct Output {
    namespace: Namespace,
    /// What `--prune` freed, or the busy package that kept it.
    pruned: Option<Pruned>,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Package {} successfully uninstalled", self.namespace)?;
        match &self.pruned {
            Some(pruned) => write!(f, ". {pruned}"),
            None => Ok(()),
        }
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let mut json = serde_json::json!({ "namespace": self.namespace.to_string() });
        let pruned = match &self.pruned {
            None => return json,
            Some(Pruned::Freed(report)) => {
                serde_json::json!({ "objects": report.objects, "bytes": report.bytes })
            }
            Some(Pruned::Busy(namespace)) => serde_json::json!({ "busy": namespace.to_string() }),
        };
        json["pruned"] = pruned;
        json
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.uninstall(args).await)
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { namespace, prune }: Input,
) -> Result<Output, Error> {
    let pruned = if prune {
        Some(local_domain.uninstall_package_pruning(namespace.clone()).await?)
    } else {
        local_domain.uninstall_package(namespace.clone()).await?;
        None
    };
    Ok(Output { namespace, pruned })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::cli::fixtures::packages::default as pkg;
    use crate::cli::model::install_package_into_temp_dir;
    use crate::cli::output::Render;

    /// Verifies that uninstall removes an installed package:
    ///   * installs a package
    ///   * uninstalls it
    ///   * verifies it's no longer present
    #[test(tokio::test)]
    async fn live_model() -> Result<(), Error> {
        let uri = pkg::URI;
        let (m, _, _temp_dir) = install_package_into_temp_dir(uri).await?;

        {
            let local_domain = m.get_local_domain();
            let output = model(
                local_domain,
                Input {
                    namespace: pkg::NAMESPACE.into(),
                    prune: false,
                },
            )
            .await?;

            assert_eq!(output.namespace, ("reference", "quilt-rs").into());
        }

        {
            let local_domain = m.get_local_domain();
            // Try to uninstall again - should fail
            if let Err(error_str) = model(
                local_domain,
                Input {
                    namespace: pkg::NAMESPACE.into(),
                    prune: false,
                },
            )
            .await
            {
                assert_eq!(
                    error_str.to_string(),
                    "quilt_rs error: The given package is not installed: reference/quilt-rs"
                );
            } else {
                return Err(Error::Test("Expected package not found error".to_string()));
            }
        }

        Ok(())
    }

    /// Creates a package holding one 3-byte file.
    async fn create_one(m: &crate::cli::model::Model, namespace: &Namespace) -> Result<(), Error> {
        let source = tempfile::tempdir()?;
        std::fs::write(source.path().join("a.txt"), "abc")?;
        m.create(crate::cli::create::Input {
            namespace: namespace.clone(),
            source: Some(source.path().to_path_buf()),
            message: None,
        })
        .await?;
        Ok(())
    }

    /// `--prune` deletes the package's objects and says what it freed.
    #[test(tokio::test)]
    async fn prune_frees_the_packages_objects() -> Result<(), Error> {
        let (m, _temp_dir) = crate::cli::model::create_model_in_temp_dir().await?;
        let namespace: Namespace = ("test", "gone").into();
        create_one(&m, &namespace).await?;

        let output = m
            .uninstall(Input {
                namespace,
                prune: true,
            })
            .await?;

        assert_eq!(
            output.to_string(),
            "Package test/gone successfully uninstalled. Freed 3 B: 1 object"
        );
        assert_eq!(
            output.to_json(),
            serde_json::json!({"namespace": "test/gone", "pruned": {"objects": 1, "bytes": 3}})
        );
        assert_eq!(m.gc().await?.to_string(), "Nothing to free");
        Ok(())
    }

    /// Another package busy keeps the objects; the uninstall still succeeds.
    #[test(tokio::test)]
    async fn prune_with_a_busy_package_keeps_the_objects() -> Result<(), Error> {
        let (m, _temp_dir) = crate::cli::model::create_model_in_temp_dir().await?;
        let busy: Namespace = ("test", "busy").into();
        create_one(&m, &busy).await?;
        let gone: Namespace = ("test", "gone").into();
        create_one(&m, &gone).await?;
        let package = m
            .get_local_domain()
            .get_installed_package(&busy)
            .await?
            .expect("installed");
        let _held = package.lock().await?;

        let output = m
            .uninstall(Input {
                namespace: gone,
                prune: true,
            })
            .await?;

        assert_eq!(
            output.to_string(),
            "Package test/gone successfully uninstalled. \
             Kept downloaded files: test/busy is busy"
        );
        assert_eq!(
            output.to_json(),
            serde_json::json!({"namespace": "test/gone", "pruned": {"busy": "test/busy"}})
        );
        Ok(())
    }

    #[test]
    fn json_carries_the_namespace() {
        let output = Output {
            namespace: ("test", "pkg").into(),
            pruned: None,
        };

        assert_eq!(output.to_json().to_string(), r#"{"namespace":"test/pkg"}"#);
    }
}
