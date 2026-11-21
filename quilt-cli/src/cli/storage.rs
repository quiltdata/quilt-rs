//! `quilt storage`: how much the dataspace Domain, `bio.quilt.dataspace`,
//! holds, per registry and in total.

use quilt_rs::dataspace::measure::DomainSize;
use quilt_rs::dataspace::measure::measure_storage;
use quilt_rs::domain::DomainRoot;
use quilt_rs::flow::format_bytes;
use quilt_rs::io::storage::LocalStorage;

use crate::cli::Error;
use crate::cli::output::Render;
use crate::cli::output::Std;

pub struct Output {
    size: DomainSize,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{} in all", format_bytes(self.size.total_bytes))?;
        for (bucket, bytes) in &self.size.registries {
            write!(f, "\n  {bucket}: {}", format_bytes(*bytes))?;
        }
        write!(
            f,
            "\n  local registry: {}",
            format_bytes(self.size.local_bytes)
        )
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "total_bytes": self.size.total_bytes,
            "registries": self.size.registries,
            "local_bytes": self.size.local_bytes,
        })
    }
}

/// Measures `domain`, where one exists: none holds nothing.
pub async fn command(domain: Option<DomainRoot>) -> Std {
    Std::from_result(storage_with(domain.as_ref()).await)
}

async fn storage_with(domain: Option<&DomainRoot>) -> Result<Output, Error> {
    let size = match domain {
        Some(domain) => measure_storage(&LocalStorage::new(), domain).await?,
        None => DomainSize::default(),
    };
    Ok(Output { size })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    #[test(tokio::test)]
    async fn storage_says_each_store_and_the_total() -> Result<(), Error> {
        let dir = tempfile::TempDir::new()?;
        let domain = DomainRoot::new(dir.path());
        let objects = domain.registry("b")?.dir().join("objects");
        std::fs::create_dir_all(&objects)?;
        std::fs::write(objects.join("a"), vec![b'x'; 1500])?;
        std::fs::write(domain.queue_db(), vec![b'x'; 500])?;

        let output = storage_with(Some(&domain)).await?;

        assert_eq!(
            output.to_string(),
            "2.0 kB in all\n  b: 1.5 kB\n  local registry: 0 B"
        );
        let json = output.to_json();
        assert_eq!(json["total_bytes"], 2000);
        assert_eq!(json["registries"]["b"], 1500);
        assert_eq!(storage_with(None).await?.to_json()["total_bytes"], 0);
        Ok(())
    }
}
