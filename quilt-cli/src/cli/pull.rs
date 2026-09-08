use std::path::PathBuf;

use quilt_rs::io::remote::HostConfig;
use quilt_rs::lineage::SyncScope;
use quilt_uri::Namespace;

use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::text::printable;
use crate::cli::text::printable_line;

#[derive(Debug)]
pub struct Input {
    pub namespace: Namespace,
    pub host_config: Option<HostConfig>,
}

#[derive(Debug)]
pub struct Output {
    pub hash: String,
    pub report: quilt_rs::flow::PullReport,
}

/// One group of the report: its heading, then its paths one per line.
///
/// Grouped by what happened to this copy rather than by the remote's diff, and
/// worded so the two "new files" cases cannot be confused — under the CLI's
/// sparse scope an added path is listed and not fetched, and saying so is the
/// difference between a true line and a misleading one.
fn group(f: &mut std::fmt::Formatter<'_>, heading: &str, paths: &[PathBuf]) -> std::fmt::Result {
    if paths.is_empty() {
        return Ok(());
    }
    let plural = if paths.len() == 1 { "" } else { "s" };
    write!(f, "\n{} file{plural} {heading}:", paths.len())?;
    for path in paths {
        // One path per line, so a path may not carry a line break of its own:
        // a forged heading in this list is indistinguishable from a real one.
        write!(f, "\n  {}", printable_line(&path.display().to_string()))?;
    }
    Ok(())
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, r#"Revision "{}" pulled"#, self.hash)?;
        // Stdout has no room constraint, so it carries the whole list — it is
        // the one surface that can, and the face an agent loop reads.
        group(f, "new", &self.report.added)?;
        group(f, "new, not downloaded", &self.report.added_not_fetched)?;
        group(f, "updated", &self.report.updated)?;
        group(f, "removed", &self.report.removed)?;
        if let Some(message) = self.report.message.as_deref() {
            // Labelled, because a pull advances to `latest` in one step and can
            // span several revisions: this is the newest one's message, not an
            // account of everything above it.
            write!(f, "\nLatest revision: {}", printable(message))?;
        }
        Ok(())
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.pull(args).await)
}

async fn pull_package(
    local_domain: &quilt_rs::LocalDomain,
    namespace: Namespace,
    host_config: Option<HostConfig>,
) -> Result<quilt_rs::flow::PullReport, Error> {
    match local_domain.get_installed_package(&namespace).await? {
        // Sparse checkout, always. The CLI has no setting for the sync scope
        // and no surface to show one, so it asks for the narrow scope rather
        // than honouring whatever a desktop app may have written to this
        // package's lineage.
        Some(installed_package) => Ok(installed_package
            .pull(host_config, SyncScope::IndividualFiles)
            .await?),
        None => Err(Error::NamespaceNotFound(namespace)),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input {
        namespace,
        host_config,
    }: Input,
) -> Result<Output, Error> {
    let report = pull_package(local_domain, namespace, host_config).await?;
    Ok(Output {
        hash: report.manifest_uri.hash.clone(),
        report,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::cli::fixtures::packages::outdated as pkg;
    use crate::cli::model::install_package_into_temp_dir;

    /// Verifies that pull updates an outdated package to the latest version:
    ///   * installs an outdated package version
    ///   * pulls the latest version
    ///   * verifies the package is up to date
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
                    host_config: None,
                },
            )
            .await?;

            assert_eq!(output.hash, pkg::LATEST_TOP_HASH);
        }

        Ok(())
    }
}
