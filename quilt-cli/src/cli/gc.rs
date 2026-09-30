use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Render;
use crate::cli::output::Std;

pub struct Output {
    report: quilt_rs::flow::GcReport,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.report)
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let quilt_rs::flow::GcReport {
            objects,
            cached_manifests,
            staging,
            bytes,
        } = self.report;
        serde_json::json!({
            "objects": objects,
            "cached_manifests": cached_manifests,
            "staging": staging,
            "bytes": bytes,
        })
    }
}

pub async fn command(m: impl Commands) -> Std {
    Std::from_result(m.gc().await)
}

pub async fn model(local_domain: &quilt_rs::LocalDomain) -> Result<Output, Error> {
    match local_domain.gc().await {
        Ok(report) => Ok(Output { report }),
        Err(quilt_rs::Error::PackageBusy(namespace)) => Err(Error::PackageBusy(namespace)),
        Err(err) => Err(err.into()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::cli::model::create_model_in_temp_dir;

    /// Uninstalling a package leaves its objects; `gc` frees them and says so.
    #[test(tokio::test)]
    async fn frees_what_an_uninstall_left() -> Result<(), Error> {
        let (m, _temp_dir) = create_model_in_temp_dir().await?;
        let source = tempfile::tempdir()?;
        std::fs::write(source.path().join("a.txt"), "abc")?;
        let namespace: quilt_uri::Namespace = ("test", "gone").into();
        m.create(crate::cli::create::Input {
            namespace: namespace.clone(),
            source: Some(source.path().to_path_buf()),
            message: None,
        })
        .await?;
        m.uninstall(crate::cli::uninstall::Input { namespace })
            .await?;

        let output = m.gc().await?;
        assert_eq!(output.to_string(), "Freed 3 B: 1 object");
        assert_eq!(
            output.to_json(),
            serde_json::json!({"objects": 1, "cached_manifests": 0, "staging": 0, "bytes": 3})
        );

        assert_eq!(m.gc().await?.to_string(), "Nothing to free");
        Ok(())
    }

    /// A package another process holds stops `gc`, naming it, with its own
    /// error kind.
    #[test(tokio::test)]
    async fn a_busy_package_is_named() -> Result<(), Error> {
        let (m, _temp_dir) = create_model_in_temp_dir().await?;
        let namespace: quilt_uri::Namespace = ("test", "busy").into();
        m.create(crate::cli::create::Input {
            namespace: namespace.clone(),
            source: None,
            message: None,
        })
        .await?;
        let package = m
            .get_local_domain()
            .get_installed_package(&namespace)
            .await?
            .expect("installed");
        let _held = package.lock().await?;

        let Err(err) = m.gc().await else {
            return Err(Error::Test("expected a busy error".to_string()));
        };
        assert_eq!(err.kind(), "package_busy");
        assert_eq!(
            err.to_string(),
            "Cannot free space: test/busy is busy in another quilt process. \
             Try again once it finishes"
        );
        Ok(())
    }
}
