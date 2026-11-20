//! `quilt local-registry`: the name this machine's local registry keeps,
//! and the names it had; with a name, the registry renamed, its queued work
//! moved, and its old name kept as an alias.

use quilt_rs::dataspace::rename::rename;
use quilt_rs::domain::DomainRoot;
use quilt_rs::domain::LocalNames;
use quilt_rs::io::storage::LocalStorage;
use quilt_rs::queue::Queue;

use crate::cli::Error;
use crate::cli::output::Render;
use crate::cli::output::Std;

#[derive(Debug)]
pub struct Input {
    /// The new name; `None` prints the names.
    pub name: Option<String>,
}

pub struct Output {
    names: LocalNames,
    /// The queue's rows moved to the new name, where it was renamed.
    moved: Option<usize>,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.names.name)?;
        if !self.names.aliases.is_empty() {
            write!(f, "\n  also named {}", self.names.aliases.join(", "))?;
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.names.name,
            "aliases": self.names.aliases,
            "moved": self.moved,
        })
    }
}

/// Names, or renames, the local registry of the Domain at `domain`. A rename
/// makes the Domain where it is not there; a machine with no Domain has no
/// local registry to name.
pub async fn command(domain: DomainRoot, args: Input) -> Std {
    Std::from_result(local_registry_with(&domain, args).await)
}

async fn local_registry_with(domain: &DomainRoot, args: Input) -> Result<Output, Error> {
    let storage = LocalStorage::new();
    let Some(name) = args.name else {
        if !domain.root().is_dir() {
            return Err(Error::NoDomain);
        }
        let names = domain.local().names(&storage).await?;
        return Ok(Output { names, moved: None });
    };
    let queue = Queue::open(&domain.queue_db()).await?;
    let renamed = rename(&storage, domain, &queue, &name).await?;
    Ok(Output {
        names: renamed.names,
        moved: Some(renamed.moved),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    #[test(tokio::test)]
    async fn a_rename_keeps_the_old_name_as_an_alias() -> Result<(), Error> {
        let dir = tempfile::TempDir::new()?;
        let domain = DomainRoot::new(dir.path().join("domain"));

        let first = local_registry_with(
            &domain,
            Input {
                name: Some("lab-mac".to_string()),
            },
        )
        .await?;
        let second = local_registry_with(
            &domain,
            Input {
                name: Some("lab-mac-2".to_string()),
            },
        )
        .await?;
        let shown = local_registry_with(&domain, Input { name: None }).await?;

        assert_eq!(first.to_string(), "lab-mac");
        assert_eq!(second.to_json()["moved"], 0);
        assert_eq!(shown.to_string(), "lab-mac-2\n  also named lab-mac");
        assert_eq!(shown.to_json()["moved"], serde_json::Value::Null);
        Ok(())
    }

    #[test(tokio::test)]
    async fn no_domain_has_no_name_to_show() -> Result<(), Error> {
        let dir = tempfile::TempDir::new()?;
        let domain = DomainRoot::new(dir.path().join("none"));

        let std = Std::from_result(local_registry_with(&domain, Input { name: None }).await);

        let Std::Err(err) = std else {
            panic!("expected an error, got {std:?}");
        };
        assert_eq!(err.kind(), "no_domain");
        assert!(!dir.path().join("none").exists());
        Ok(())
    }

    #[test(tokio::test)]
    async fn a_name_a_registry_cannot_keep_is_an_error() -> Result<(), Error> {
        let dir = tempfile::TempDir::new()?;
        let domain = DomainRoot::new(dir.path());

        let std = Std::from_result(
            local_registry_with(
                &domain,
                Input {
                    name: Some("my mac".to_string()),
                },
            )
            .await,
        );

        let Std::Err(err) = std else {
            panic!("expected an error, got {std:?}");
        };
        assert_eq!(err.kind(), "invalid_name");
        Ok(())
    }
}
