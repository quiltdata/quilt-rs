//! `quilt home --dataspace`: the default dataspace, which replaces the home,
//! printed, or chosen and made a dataspace. The choice is the application's,
//! kept in its folder beside its other settings, not in the Domain.

use std::path::Path;
use std::path::PathBuf;

use quilt_rs::dataspace::home::default_dataspace;
use quilt_rs::dataspace::home::set_home;
use quilt_rs::io::storage::LocalStorage;

use crate::cli::Error;
use crate::cli::output::Render;
use crate::cli::output::Std;
use crate::cli::text::printable_line;

#[derive(Debug)]
pub struct Input {
    /// The folder to choose; `None` prints the one chosen.
    pub dir: Option<PathBuf>,
}

pub struct Output {
    dataspace: PathBuf,
    /// Whether the folder was given an empty `data.toml`.
    made: bool,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            printable_line(&self.dataspace.display().to_string())
        )?;
        if self.made {
            write!(f, "\n  made a dataspace, with an empty data.toml")?;
        }
        Ok(())
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({
            "dataspace": self.dataspace.display().to_string(),
            "made": self.made,
        })
    }
}

/// Prints or chooses the default dataspace of the application whose folder
/// is `app_dir`, the one `--domain` names.
pub async fn command(app_dir: PathBuf, args: Input) -> Std {
    Std::from_result(home_with(&app_dir, args).await)
}

async fn home_with(app_dir: &Path, Input { dir }: Input) -> Result<Output, Error> {
    let storage = LocalStorage::new();
    let Some(dir) = dir else {
        let dataspace = default_dataspace(&storage, app_dir)
            .await?
            .ok_or(Error::NoDefaultDataspace)?;
        return Ok(Output {
            dataspace,
            made: false,
        });
    };
    let home = set_home(&storage, app_dir, &dir).await?;
    Ok(Output {
        dataspace: home.dataspace,
        made: home.made,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    use test_log::test;

    #[test(tokio::test)]
    async fn a_chosen_dataspace_is_printed_again() -> Result<(), Error> {
        let dir = tempfile::TempDir::new()?;
        let app = dir.path().join("app");
        let folder = dir.path().join("work");

        let none = Std::from_result(home_with(&app, Input { dir: None }).await);
        let Std::Err(err) = none else {
            panic!("expected an error, got {none:?}");
        };
        assert_eq!(err.kind(), "no_default_dataspace");

        let chosen = home_with(
            &app,
            Input {
                dir: Some(folder.clone()),
            },
        )
        .await?;
        assert_eq!(chosen.to_json()["made"], true);
        assert!(folder.join("data.toml").exists());

        let shown = home_with(&app, Input { dir: None }).await?;
        assert_eq!(shown.to_string(), folder.display().to_string());
        Ok(())
    }
}
