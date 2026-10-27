use std::path::PathBuf;

use quilt_rs::io::remote::HostConfig;
use quilt_rs::lineage::SyncScope;
use quilt_uri::Namespace;

use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Render;
use crate::cli::output::Std;
use crate::cli::text::printable;
use crate::cli::text::printable_line;

#[derive(Debug)]
pub struct Input {
    pub namespace: Namespace,
    pub host_config: Option<HostConfig>,
}

/// `None` when the package was already up to date and nothing was pulled.
#[derive(Debug)]
pub struct Output {
    pub report: Option<quilt_rs::flow::PullReport>,
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
        let Some(report) = &self.report else {
            return write!(f, "Already up to date");
        };
        write!(f, r#"Revision "{}" pulled"#, report.manifest_uri.hash)?;
        // Stdout has no room constraint, so it carries the whole list — it is
        // the one surface that can, and the face an agent loop reads.
        group(f, "new", &report.added)?;
        group(f, "new, not downloaded", &report.added_not_fetched)?;
        group(f, "updated", &report.updated)?;
        group(f, "removed", &report.removed)?;
        if let Some(message) = report.message.as_deref() {
            // Labelled, because a pull advances to `latest` in one step and can
            // span several revisions: this is the newest one's message, not an
            // account of everything above it.
            write!(f, "\nLatest revision: {}", printable(message))?;
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        let paths = |list: &[PathBuf]| {
            list.iter()
                // Lossless only because these are manifest keys, which are
                // UTF-8 by the format's nature. `display()` would silently
                // mangle a path that was not.
                .map(|path| path.display().to_string())
                .collect::<Vec<_>>()
        };

        // The same shape when nothing was pulled, so a consumer reads one
        // schema: no revision, so no `hash`.
        let Some(report) = &self.report else {
            return serde_json::json!({
                "hash": null,
                "manifest_uri": null,
                "added": [],
                "added_not_fetched": [],
                "updated": [],
                "removed": [],
                "message": null,
            });
        };
        serde_json::json!({
            "hash": &report.manifest_uri.hash,
            "manifest_uri": report.manifest_uri.to_string(),
            "added": paths(&report.added),
            "added_not_fetched": paths(&report.added_not_fetched),
            "updated": paths(&report.updated),
            "removed": paths(&report.removed),
            "message": report.message.as_deref(),
        })
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.pull(args).await)
}

async fn pull_package(
    local_domain: &quilt_rs::LocalDomain,
    namespace: Namespace,
    host_config: Option<HostConfig>,
) -> Result<quilt_rs::flow::Pulled, Error> {
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
    let report = match pull_package(local_domain, namespace, host_config).await? {
        quilt_rs::flow::Pulled::UpToDate => None,
        quilt_rs::flow::Pulled::Applied(report) => Some(report),
    };
    Ok(Output { report })
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

            assert_eq!(
                output.report.unwrap().manifest_uri.hash,
                pkg::LATEST_TOP_HASH
            );
        }

        Ok(())
    }

    fn manifest_uri(hash: &str) -> quilt_uri::ManifestUri {
        quilt_uri::ManifestUri {
            origin: None,
            bucket: "bucket".to_string(),
            namespace: ("demo", "sales").into(),
            hash: hash.to_string(),
        }
    }

    /// All four groups are always present, so a consumer indexes without
    /// existence checks — and the URI carries the full hash, never the
    /// abbreviated form `ManifestUri::display` produces.
    #[test]
    fn json_always_lists_four_groups_and_a_full_uri() {
        let hash = "0123456789abcdef";
        let output = Output {
            report: Some(quilt_rs::flow::PullReport {
                manifest_uri: manifest_uri(hash),
                added: vec![std::path::PathBuf::from("new.csv")],
                added_not_fetched: Vec::new(),
                updated: Vec::new(),
                removed: Vec::new(),
                message: None,
            }),
        };

        assert_eq!(
            output.to_json().to_string(),
            r#"{"hash":"0123456789abcdef","manifest_uri":"quilt+s3://bucket#package=demo/sales@0123456789abcdef","added":["new.csv"],"added_not_fetched":[],"updated":[],"removed":[],"message":null}"#
        );

        let up_to_date = Output { report: None };
        assert_eq!(
            up_to_date.to_json().to_string(),
            r#"{"hash":null,"manifest_uri":null,"added":[],"added_not_fetched":[],"updated":[],"removed":[],"message":null}"#
        );
        assert_eq!(up_to_date.to_string(), "Already up to date");
    }
}
