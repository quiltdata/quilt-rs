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
    /// Sorted by bucket, then namespace, so each bucket's packages are adjacent.
    installed_packages_list: Vec<PackageEntry>,
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

pub async fn command(m: impl Commands) -> Std {
    Std::from_result(m.list().await)
}

/// Lists installed packages from the local domain — no network, one lineage
/// read per package.
///
/// `status` is the [`UpstreamState`] cascade over the lineage's four hashes,
/// read against the *last-known* remote tip: `list` never refreshes it.
/// `quilt status <namespace>` does.
pub async fn model(local_domain: &quilt_rs::LocalDomain) -> Result<Output, Error> {
    let installed_packages = local_domain.list_installed_packages().await?;
    let mut installed_packages_list = Vec::with_capacity(installed_packages.len());

    for installed_package in installed_packages {
        let lineage = installed_package.lineage().await?;
        installed_packages_list.push(PackageEntry {
            status: UpstreamState::from(lineage.clone()),
            namespace: installed_package.namespace,
            // An empty bucket is what the cascade already reads as local-only,
            // so it renders like a package with no remote at all.
            bucket: lineage
                .remote_uri
                .map(|remote| remote.bucket)
                .filter(|bucket| !bucket.is_empty()),
        });
    }

    // The leading `is_none()` puts the local-only packages last; the rest
    // makes each bucket's packages adjacent, which `Display`'s span needs.
    installed_packages_list.sort_by(|a, b| {
        (a.bucket.is_none(), &a.bucket, &a.namespace).cmp(&(
            b.bucket.is_none(),
            &b.bucket,
            &b.namespace,
        ))
    });

    Ok(Output {
        installed_packages_list,
    })
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

    /// A bucket shared by several packages is named once, in a spanning cell;
    /// a bucket with one package still gets its own cell; a package with no
    /// remote names that absence.
    #[test]
    fn test_display_spans_repeated_buckets() {
        let output = Output {
            installed_packages_list: vec![
                entry(Some("acme-research"), "one", UpstreamState::UpToDate),
                entry(Some("acme-research"), "two", UpstreamState::UpToDate),
                entry(Some("zulu-bucket"), "late", UpstreamState::Behind),
                entry(None, "scratch", UpstreamState::Local),
            ],
        };

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
    #[test(tokio::test)]
    async fn test_model() -> Result<(), Error> {
        // Test with one installed package
        let uri = format!("{}&path={}", pkg::URI, pkg::README_LK_ESCAPED);
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
    #[test(tokio::test)]
    async fn test_command_with_package() -> Result<(), Error> {
        let uri = format!("{}&path={}", pkg::URI, pkg::README_LK_ESCAPED);
        let (m, _, _temp_dir) = install_package_into_temp_dir(&uri).await?;

        if let Std::Out(output) = command(m).await {
            assert!(output.contains(pkg::BUCKET));
            assert!(output.contains(pkg::NAMESPACE_STR));
            assert!(output.contains("up_to_date"));
        } else {
            return Err(Error::Test("Failed to list packages".to_string()));
        }

        Ok(())
    }
}
