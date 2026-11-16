use std::path::PathBuf;

use quilt_rs::io::storage::LocalStorage;
use quilt_rs::io::storage::Storage;
use quilt_rs::settings;
use quilt_rs::settings::PackageOrigin;
use quilt_rs::settings::SharedSettings;
use quilt_uri::Host;

use crate::cli::Error;
use crate::cli::model::Commands;
use crate::cli::output::Render;
use crate::cli::output::Std;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Action {
    Show,
    Set(Host),
    Clear,
}

#[derive(Debug)]
pub struct Input {
    pub action: Action,
}

pub struct Output {
    catalog: Option<Host>,
}

impl std::fmt::Display for Output {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.catalog {
            Some(host) => write!(f, "{host}"),
            None => write!(f, "No default catalog"),
        }
    }
}

impl Render for Output {
    fn to_json(&self) -> serde_json::Value {
        serde_json::json!({ "default_catalog": self.catalog.as_ref().map(ToString::to_string) })
    }
}

pub async fn command(m: impl Commands, args: Input) -> Std {
    Std::from_result(m.catalog(args).await)
}

pub async fn model(Input { action }: Input) -> Result<Output, Error> {
    run(&open()?, &settings::quilt3_config_files(), action).await
}

/// The default catalog, or the `--host` a command was given, for a command
/// that is not about a package. Naming one always wins.
pub async fn host_or_default(explicit: Option<Host>) -> Result<Host, Error> {
    if let Some(host) = explicit {
        return Ok(host);
    }
    resolve(&open()?, &settings::quilt3_config_files(), None).await
}

fn open() -> Result<SharedSettings<LocalStorage>, Error> {
    let dir = settings::shared_dir().ok_or(Error::SettingsDir)?;
    Ok(SharedSettings::new(LocalStorage::new(), dir))
}

async fn resolve<S: Storage + Sync>(
    settings: &SharedSettings<S>,
    quilt3_config_files: &[PathBuf],
    explicit: Option<Host>,
) -> Result<Host, Error> {
    let default = settings
        .default_catalog_or_adopt(quilt3_config_files)
        .await?;
    settings::effective_catalog(explicit, PackageOrigin::None, default).ok_or(Error::NoCatalog)
}

async fn run<S: Storage + Sync>(
    settings: &SharedSettings<S>,
    quilt3_config_files: &[PathBuf],
    action: Action,
) -> Result<Output, Error> {
    let catalog = match action {
        Action::Show => {
            settings
                .default_catalog_or_adopt(quilt3_config_files)
                .await?
        }
        Action::Set(host) => {
            settings.set_default_catalog(&host).await?;
            Some(host)
        }
        Action::Clear => {
            settings.clear_default_catalog().await?;
            None
        }
    };
    Ok(Output { catalog })
}

#[cfg(test)]
mod tests {
    use super::*;

    use quilt_uri::fixtures;
    use tempfile::TempDir;
    use test_log::test;

    fn settings() -> (SharedSettings<LocalStorage>, TempDir) {
        let dir = TempDir::new().unwrap();
        let settings = SharedSettings::new(LocalStorage::new(), dir.path().join("auth"));
        (settings, dir)
    }

    async fn shown(settings: &SharedSettings<LocalStorage>, action: Action) -> String {
        let out = run(settings, &[], action).await.unwrap();
        format!("{out}")
    }

    #[test(tokio::test)]
    async fn set_show_and_clear_round_trip() {
        let (settings, _dir) = settings();
        assert_eq!(shown(&settings, Action::Show).await, "No default catalog");
        assert_eq!(
            shown(&settings, Action::Set(fixtures::host())).await,
            "quilt.test"
        );
        assert_eq!(shown(&settings, Action::Show).await, "quilt.test");
        assert_eq!(shown(&settings, Action::Clear).await, "No default catalog");
        assert_eq!(shown(&settings, Action::Show).await, "No default catalog");
    }

    #[test]
    fn json_carries_the_host_or_null() {
        let some = Output {
            catalog: Some(fixtures::host()),
        };
        let none = Output { catalog: None };

        assert_eq!(
            some.to_json().to_string(),
            r#"{"default_catalog":"quilt.test"}"#
        );
        assert_eq!(none.to_json().to_string(), r#"{"default_catalog":null}"#);
    }

    #[test(tokio::test)]
    async fn show_adopts_quilt3s_catalog() {
        let (settings, dir) = settings();
        let config = dir.path().join("config.yml");
        std::fs::write(&config, "navigator_url: https://quilt3.example.com\n").unwrap();

        let shown = shown_with(&settings, &[config]).await;

        assert_eq!(shown, "quilt3.example.com");
        assert_eq!(
            settings
                .default_catalog()
                .await
                .unwrap()
                .map(|host| host.to_string()),
            Some("quilt3.example.com".to_string())
        );
    }

    async fn shown_with(settings: &SharedSettings<LocalStorage>, files: &[PathBuf]) -> String {
        let out = run(settings, files, Action::Show).await.unwrap();
        format!("{out}")
    }

    #[test(tokio::test)]
    async fn a_host_that_is_named_beats_the_default() {
        let (settings, _dir) = settings();
        settings
            .set_default_catalog(&fixtures::one_host())
            .await
            .unwrap();

        let chosen = resolve(&settings, &[], Some(fixtures::another_host()))
            .await
            .unwrap();

        assert_eq!(chosen, fixtures::another_host());
    }

    #[test(tokio::test)]
    async fn the_default_is_used_when_no_host_is_named() {
        let (settings, _dir) = settings();
        settings
            .set_default_catalog(&fixtures::host())
            .await
            .unwrap();

        let chosen = resolve(&settings, &[], None).await.unwrap();

        assert_eq!(chosen, fixtures::host());
    }

    #[test(tokio::test)]
    async fn no_host_and_no_default_is_an_error_that_says_what_to_do() {
        let (settings, _dir) = settings();

        let err = resolve(&settings, &[], None).await.unwrap_err();

        assert!(matches!(err, Error::NoCatalog));
        let message = err.to_string();
        assert!(message.contains("--host"), "{message}");
        assert!(message.contains("quilt catalog set"), "{message}");
    }
}
