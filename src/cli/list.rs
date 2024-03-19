use std::fmt;

use quilt_rs::quilt::lineage;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

pub struct Output {
    installed_packages_list: Vec<quilt_rs::InstalledPackage>,
}

impl fmt::Display for Output {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.installed_packages_list.is_empty() {
            return write!(f, "No installed packages");
        }
        let mut output: Vec<String> = Vec::new();
        for installed_package in &self.installed_packages_list {
            output.push(installed_package.to_string());
        }
        write!(f, "{}", output.join("\n"))
    }
}

pub async fn command(m: impl Commands) -> Std {
    match m.list().await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    lineage_io: &impl lineage::ReadableLineage,
) -> Result<Output, Error> {
    Ok(Output {
        installed_packages_list: local_domain.list_installed_packages(lineage_io).await?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[tokio::test]
    async fn list() -> Result<(), Error> {
        let lineage_io = lineage::mocks::create(1);
        let local_domain = quilt_rs::LocalDomain::new(PathBuf::new());
        let output = model(&local_domain, &lineage_io).await?;
        assert_eq!(output.installed_packages_list[0].namespace, "foo/bar_0");
        Ok(())
    }
}
