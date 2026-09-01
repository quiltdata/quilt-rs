use quilt_rs::lineage::UpstreamState;
use quilt_uri::Namespace;
use tabled::settings::Modify;
use tabled::settings::Span;

use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Std;

/// Rendered in the `bucket` cell of a package with no remote.
const NO_BUCKET: &str = "∅";

/// A listed package as [`model`] resolves it; [`PackageRow`] is its rendering.
pub struct PackageEntry {
    /// `None` for a local-only package — no remote, or a remote with no bucket.
    pub bucket: Option<String>,
    pub namespace: Namespace,
    pub status: UpstreamState,
}

pub struct Output {
    /// Grouped by bucket. Only [`Output::new`] builds this, because
    /// [`Display`](std::fmt::Display) cannot restore the order it needs.
    installed_packages_list: Vec<PackageEntry>,
}

impl Output {
    /// Sorts each bucket's packages together, local-only ones last.
    ///
    /// `Display` spans a bucket's cell over a *run* of adjacent rows, so a
    /// bucket reached in two runs would be named twice, in two spans. Sorting
    /// here rather than trusting the caller is what makes that unreachable.
    fn new(mut installed_packages_list: Vec<PackageEntry>) -> Self {
        installed_packages_list.sort_by(|a, b| {
            (a.bucket.is_none(), &a.bucket, &a.namespace).cmp(&(
                b.bucket.is_none(),
                &b.bucket,
                &b.namespace,
            ))
        });
        Self {
            installed_packages_list,
        }
    }

    fn to_json(&self) -> String {
        let packages: Vec<_> = self
            .installed_packages_list
            .iter()
            .map(|package| {
                serde_json::json!({
                    "bucket": package.bucket.as_deref(),
                    "namespace": package.namespace.to_string(),
                    "status": package.status,
                })
            })
            .collect();
        serde_json::json!({ "packages": packages }).to_string()
    }
}

#[derive(tabled::Tabled)]
struct PackageRow {
    bucket: String,
    namespace: String,
    status: String,
}

impl From<&PackageEntry> for PackageRow {
    fn from(entry: &PackageEntry) -> Self {
        Self {
            bucket: entry.bucket.as_deref().unwrap_or(NO_BUCKET).to_string(),
            namespace: entry.namespace.to_string(),
            status: entry.status.to_string(),
        }
    }
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if self.installed_packages_list.is_empty() {
            return write!(f, "No installed packages");
        }

        let mut table =
            tabled::Table::new(self.installed_packages_list.iter().map(PackageRow::from));

        // Span each bucket's cell over its packages' rows. Not
        // `Merge::vertical()`, the built-in for this: it merges *every*
        // column, so two adjacent packages sharing a status collapse into one
        // cell and the row count stops being readable.
        //
        // `chunk_by` only sees runs, so it depends on `model` having sorted
        // equal buckets adjacent.
        let mut row = 1; // row 0 is the header
        for group in self
            .installed_packages_list
            .chunk_by(|a, b| a.bucket == b.bucket)
        {
            if group.len() > 1 {
                table.with(Modify::new((row, 0)).with(Span::row(group.len().cast_signed())));
            }
            row += group.len();
        }

        write!(f, "{table}")
    }
}

pub async fn command(m: impl Commands, json: bool) -> Std {
    match m.list().await {
        Ok(output) => Std::Out(if json {
            output.to_json()
        } else {
            output.to_string()
        }),
        Err(error) => Std::Err(error),
    }
}

/// Lists installed packages from the local domain — no network, one read of
/// the lineage record for the whole listing.
///
/// `status` is the [`UpstreamState`] cascade over the lineage's four hashes,
/// read against the *last-known* remote tip: `list` never refreshes it.
/// `quilt status <namespace>` does.
pub async fn model(local_domain: &quilt_rs::LocalDomain) -> Result<Output, Error> {
    let domain_lineage = local_domain.get_lineage().await?;
    let mut installed_packages_list = Vec::with_capacity(domain_lineage.packages.len());

    for (namespace, lineage) in domain_lineage.packages {
        // An empty bucket is what the cascade already reads as local-only, so
        // it renders like a package with no remote at all.
        let bucket = lineage
            .remote_uri
            .as_ref()
            .map(|remote| remote.bucket.clone())
            .filter(|bucket| !bucket.is_empty());
        installed_packages_list.push(PackageEntry {
            status: UpstreamState::from(lineage),
            namespace,
            bucket,
        });
    }

    Ok(Output::new(installed_packages_list))
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::cli::fixtures::packages::default as pkg;
    use crate::cli::model::Commands;
    use crate::cli::model::create_model_in_temp_dir;
    use crate::cli::model::install_package_into_temp_dir;

    fn entry(bucket: Option<&str>, name: &str, status: UpstreamState) -> PackageEntry {
        PackageEntry {
            bucket: bucket.map(str::to_owned),
            namespace: ("example", name).into(),
            status,
        }
    }

    #[test(tokio::test)]
    async fn test_empty_list() -> Result<(), Error> {
        let (m, _temp_dir) = create_model_in_temp_dir().await?;
        {
            let local_domain = m.get_local_domain();
            let empty_output = model(local_domain).await?;
            assert!(empty_output.installed_packages_list.is_empty());
            assert_eq!(format!("{empty_output}"), "No installed packages");
        }
        Ok(())
    }

    #[test]
    fn test_json_empty_list() {
        let output = Output::new(Vec::new());

        assert_eq!(output.to_json(), r#"{"packages":[]}"#);
    }

    #[test]
    fn test_json_includes_bucket_namespace_and_status() {
        let output = Output::new(vec![
            entry(Some("acme-research"), "one", UpstreamState::UpToDate),
            entry(None, "scratch", UpstreamState::Local),
        ]);

        assert_eq!(
            output.to_json(),
            r#"{"packages":[{"bucket":"acme-research","namespace":"example/one","status":"up_to_date"},{"bucket":null,"namespace":"example/scratch","status":"local"}]}"#
        );
    }

    /// Packages arrive in whatever order the lineage map yields, so the two
    /// buckets here are interleaved and the local-only package sits in the
    /// middle. Each bucket must still be named exactly once, in one span.
    #[test]
    fn test_display_groups_interleaved_buckets() {
        let output = Output::new(vec![
            entry(Some("zulu-bucket"), "late", UpstreamState::Behind),
            entry(Some("acme-research"), "two", UpstreamState::UpToDate),
            entry(None, "scratch", UpstreamState::Local),
            entry(Some("acme-research"), "one", UpstreamState::UpToDate),
            entry(Some("zulu-bucket"), "early", UpstreamState::Ahead),
        ]);

        let rendered = format!("{output}");
        assert_eq!(rendered.matches("acme-research").count(), 1, "one span");
        assert_eq!(rendered.matches("zulu-bucket").count(), 1, "one span");

        let positions = |name: &str| rendered.find(name).expect("{name} is listed");
        // Buckets sort, each bucket's own packages sort, local-only last.
        assert!(positions("acme-research") < positions("zulu-bucket"));
        assert!(positions("example/one") < positions("example/two"));
        assert!(positions("example/early") < positions("example/late"));
        assert!(positions("zulu-bucket") < positions("example/scratch"));
    }

    /// A bucket shared by several packages is named once, in a spanning cell;
    /// a bucket with one package still gets its own cell; a package with no
    /// remote names that absence.
    #[test]
    fn test_display_spans_repeated_buckets() {
        let output = Output::new(vec![
            entry(Some("acme-research"), "one", UpstreamState::UpToDate),
            entry(Some("acme-research"), "two", UpstreamState::UpToDate),
            entry(Some("zulu-bucket"), "late", UpstreamState::Behind),
            entry(None, "scratch", UpstreamState::Local),
        ]);

        let rendered = format!("{output}");
        assert_eq!(rendered.matches("acme-research").count(), 1, "named once");
        assert_eq!(rendered.matches("zulu-bucket").count(), 1);
        assert_eq!(rendered.matches(NO_BUCKET).count(), 1);
        for name in [
            "example/one",
            "example/two",
            "example/late",
            "example/scratch",
        ] {
            assert!(rendered.contains(name), "{name} is listed");
        }
        // One header, so `status` names the column and nothing else.
        assert_eq!(rendered.matches("status").count(), 1);
        // Statuses are never merged, even when adjacent rows repeat one.
        assert_eq!(rendered.matches("up_to_date").count(), 2);
    }

    #[test(tokio::test)]
    async fn test_model_with_local_package() -> Result<(), Error> {
        let (m, _temp_dir) = create_model_in_temp_dir().await?;
        m.create(crate::cli::create::Input {
            namespace: ("example", "local").into(),
            source: None,
            message: None,
        })
        .await?;

        let output = model(m.get_local_domain()).await?;
        let entry = &output.installed_packages_list[0];
        assert_eq!(entry.bucket, None);
        assert_eq!(entry.namespace, ("example", "local").into());
        assert_eq!(entry.status, UpstreamState::Local);

        let output = format!("{output}");
        assert!(output.contains("example/local"));
        assert!(output.contains(NO_BUCKET));

        Ok(())
    }

    /// Verifies that list model returns correct output for both empty and populated states:
    ///   * empty list shows "No installed packages" message
    ///   * after installing a package, shows its bucket, namespace and status
    ///
    /// This uses the shared S3 fixture and is run by credentialed CI only.
    #[test(tokio::test)]
    async fn live_model() -> Result<(), Error> {
        // Test with one installed package
        let uri = format!("{}&path={}", pkg::URI_LATEST, pkg::README_LK_ESCAPED);
        let (m, _, _temp_dir) = install_package_into_temp_dir(&uri).await?;
        {
            let local_domain = m.get_local_domain();
            let output = model(local_domain).await?;

            let entry = &output.installed_packages_list[0];
            assert_eq!(entry.bucket.as_deref(), Some(pkg::BUCKET));
            assert_eq!(entry.namespace, pkg::NAMESPACE.into());
            assert_eq!(entry.status, UpstreamState::UpToDate);

            let output = format!("{output}");
            assert!(output.contains("bucket"));
            assert!(output.contains("namespace"));
            assert!(output.contains("status"));
            assert!(output.contains(pkg::BUCKET));
            assert!(output.contains("up_to_date"));
        }

        Ok(())
    }

    /// Verifies that list command returns correct output after installing a package:
    ///   * shows the installed package namespace
    ///   * formats output according to display implementation
    // TODO: install and list multiple packages
    ///
    /// This uses the shared S3 fixture and is run by credentialed CI only.
    #[test(tokio::test)]
    async fn live_command_with_package() -> Result<(), Error> {
        let uri = format!("{}&path={}", pkg::URI_LATEST, pkg::README_LK_ESCAPED);
        let (m, _, _temp_dir) = install_package_into_temp_dir(&uri).await?;

        if let Std::Out(output) = command(m, false).await {
            assert!(output.contains(pkg::BUCKET));
            assert!(output.contains(pkg::NAMESPACE_STR));
            assert!(output.contains("up_to_date"));
        } else {
            return Err(Error::Test("Failed to list packages".to_string()));
        }

        Ok(())
    }
}
