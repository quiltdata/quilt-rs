use std::path::PathBuf;
use tracing::log;

use crate::cli::model::Commands;
use crate::cli::output::Std;
use crate::cli::Error;

#[derive(Debug)]
pub struct Input {
    pub path: Option<PathBuf>,
    pub migrate: Option<bool>,
}

pub struct Output {
    pub path: PathBuf,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.path.display())
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    match m.home(args).await {
        Ok(output) => Std::Out(output.to_string()),
        Err(err) => Std::Err(err),
    }
}

pub async fn model(
    local_domain: &quilt_rs::LocalDomain,
    Input { path, migrate }: Input,
) -> Result<Output, Error> {
    if let Some(dir_path) = path {
        // Set the working directory
        let dir = local_domain.set_home(&dir_path).await?;

        // Migrate files from legacy working directory if requested
        if migrate.unwrap_or(false) {
            log::info!("Migrating files from legacy working directories to new home");

            // Get all installed packages
            let packages = local_domain.list_installed_packages().await?;

            for package in packages {
                log::info!("Migrating files for package {}", package.namespace);
                local_domain
                    .migrate_from_legacy_working_dir(&package.namespace, &dir_path)
                    .await?;
            }

            log::info!("Migration completed successfully");
        }

        Ok(Output {
            path: dir.get()?.clone(),
        })
    } else {
        // Get the current working directory
        let dir = local_domain.get_home().await?;
        Ok(Output {
            path: dir.get()?.clone(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    use crate::cli::model::create_model_in_temp_dir;

    #[test(tokio::test)]
    async fn test_model_get() -> Result<(), Error> {
        let (m, temp_dir) = create_model_in_temp_dir().await?;

        // Set working directory first
        let working_dir = temp_dir.path().join("working_dir");
        std::fs::create_dir_all(&working_dir)?;

        {
            let local_domain = m.get_local_domain();
            let set_output = model(
                local_domain,
                Input {
                    path: Some(working_dir.clone()),
                    migrate: None,
                },
            )
            .await?;

            assert_eq!(set_output.path, working_dir);
        }

        // Now test getting the working directory
        {
            let local_domain = m.get_local_domain();
            let get_output = model(
                local_domain,
                Input {
                    path: None,
                    migrate: None,
                },
            )
            .await?;

            assert_eq!(get_output.path, working_dir);
        }

        Ok(())
    }

    #[test(tokio::test)]
    async fn test_model_set() -> Result<(), Error> {
        let (m, temp_dir) = create_model_in_temp_dir().await?;

        // Create a new working directory
        let working_dir = temp_dir.path().join("new_working_dir");
        std::fs::create_dir_all(&working_dir)?;

        let local_domain = m.get_local_domain();
        let output = model(
            local_domain,
            Input {
                path: Some(working_dir.clone()),
                migrate: None,
            },
        )
        .await?;

        assert_eq!(output.path, working_dir);

        Ok(())
    }
}
