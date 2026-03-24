use std::path::Path;
use std::path::PathBuf;

use askama::Template;
use rust_i18n::t;

use semver::Version;

use crate::app::App;
use crate::error::Error;
use crate::telemetry::LogsDir;
use crate::ui::btn;
use crate::ui::crumbs;
use crate::ui::layout::Layout;
use crate::ui::Icon;

/// A single authenticated host row for the settings page.
pub struct AuthHost<'a> {
    pub name: String,
    pub relogin_button: btn::TmplButton<'a>,
}

pub struct ViewSettings<'a> {
    version: Version,
    logs_dir: &'a LogsDir,
    home_dir: Option<PathBuf>,
    data_dir: PathBuf,
    auth_hosts: Vec<String>,
    log_level: String,
}

#[derive(Template)]
#[template(path = "./pages/settings.html")]
pub struct TmplSettings<'a> {
    layout: Layout<'a>,
    version: String,
    release_notes: btn::TmplButton<'a>,
    home_dir: String,
    open_home_dir: btn::TmplButton<'a>,
    data_dir: String,
    open_data_dir: btn::TmplButton<'a>,
    auth_hosts: Vec<AuthHost<'a>>,
    log_level: String,
    logs_dir: String,
    open_logs_dir: btn::TmplButton<'a>,
    crash_report: btn::TmplButton<'a>,
    email_support: btn::TmplButton<'a>,
}

impl<'a> TmplSettings<'a> {
    fn breadcrumbs() -> crumbs::TmplBreadcrumbs<'a> {
        crumbs::TmplBreadcrumbs {
            list: vec![
                crumbs::Link::home(),
                crumbs::Current::create(t!("settings.title")),
            ],
        }
    }

    fn release_notes_button(version: &Version) -> btn::TmplButton<'static> {
        let release_url = format!(
            "https://github.com/quiltdata/quilt-rs/releases/tag/QuiltSync/v{}",
            version
        );
        btn::TmplButton::builder()
            .set_label(t!("settings.release_notes"))
            .set_modificator(btn::Modificator::Link)
            .set_js(btn::JsSelector::OpenInWebBrowser)
            .set_data("url", release_url.clone())
            .set_title(release_url)
    }

    fn open_home_dir_button() -> btn::TmplButton<'static> {
        btn::TmplButton::builder()
            .set_icon(Icon::FolderOpen)
            .set_label(t!("settings.open"))
            .set_modificator(btn::Modificator::Link)
            .set_size(btn::Size::Small)
            .set_js(btn::JsSelector::OpenHomeDir)
    }

    fn open_data_dir_button() -> btn::TmplButton<'static> {
        btn::TmplButton::builder()
            .set_icon(Icon::FolderOpen)
            .set_label(t!("settings.open"))
            .set_modificator(btn::Modificator::Link)
            .set_size(btn::Size::Small)
            .set_js(btn::JsSelector::OpenDataDir)
    }

    fn open_logs_dir_button(logs_dir: &LogsDir) -> btn::TmplButton<'static> {
        let button = btn::TmplButton::builder()
            .set_icon(Icon::FolderOpen)
            .set_label(t!("settings.open"))
            .set_modificator(btn::Modificator::Link)
            .set_size(btn::Size::Small)
            .set_js(btn::JsSelector::DebugLogs)
            .set_title(logs_dir.path().display().to_string());

        if matches!(logs_dir, LogsDir::Temporary(_)) {
            return button.set_icon(Icon::Warning);
        }

        button
    }

    fn crash_report_button() -> btn::TmplButton<'static> {
        btn::TmplButton::builder()
            .set_label(t!("settings.send_crash_report"))
            .set_js(btn::JsSelector::CrashReport)
    }

    fn relogin_button(host: &str) -> btn::TmplButton<'static> {
        btn::TmplButton::builder()
            .set_icon(Icon::Warning)
            .set_label(t!("settings.relogin"))
            .set_size(btn::Size::Small)
            .set_js(btn::JsSelector::EraseAuth)
            .set_data("host", host.to_string())
    }

    fn email_support_button(version: &Version) -> btn::TmplButton<'static> {
        btn::TmplButton::builder()
            .set_label(t!("settings.email_support"))
            .set_js(btn::JsSelector::DiagnosticLogs)
            .set_data("version", version.to_string())
            .set_data("os", std::env::consts::OS.to_string())
            .set_data("collecting", t!("settings.email_support_collecting"))
            .set_data("logs-saved", t!("settings.email_support_logs_saved"))
            .set_data("open-email", t!("settings.email_support_open"))
            .set_data("show-file", t!("settings.email_support_show_file"))
    }
}

impl From<ViewSettings<'_>> for TmplSettings<'_> {
    fn from(view: ViewSettings<'_>) -> Self {
        let auth_hosts: Vec<AuthHost> = view
            .auth_hosts
            .iter()
            .map(|host| AuthHost {
                relogin_button: TmplSettings::relogin_button(host),
                name: host.clone(),
            })
            .collect();

        TmplSettings {
            version: view.version.to_string(),
            release_notes: TmplSettings::release_notes_button(&view.version),
            home_dir: view
                .home_dir
                .as_ref()
                .map(|h| h.display().to_string())
                .unwrap_or_else(|| t!("settings.not_set").into()),
            open_home_dir: TmplSettings::open_home_dir_button(),
            data_dir: view.data_dir.display().to_string(),
            open_data_dir: TmplSettings::open_data_dir_button(),
            auth_hosts,
            log_level: view.log_level.clone(),
            logs_dir: view.logs_dir.path().display().to_string(),
            open_logs_dir: TmplSettings::open_logs_dir_button(view.logs_dir),
            crash_report: TmplSettings::crash_report_button(),
            email_support: TmplSettings::email_support_button(&view.version),
            layout: Layout::builder().set_breadcrumbs(TmplSettings::breadcrumbs()),
        }
    }
}

impl<'a> ViewSettings<'a> {
    pub async fn create(
        app: &'a App,
        data_dir: &Path,
        home_dir: Option<PathBuf>,
        log_level: String,
        auth_hosts: Vec<String>,
    ) -> Result<ViewSettings<'a>, Error> {
        Ok(ViewSettings {
            version: app.version.clone(),
            logs_dir: &app.logs_dir,
            home_dir,
            data_dir: data_dir.to_path_buf(),
            auth_hosts,
            log_level,
        })
    }

    pub fn render(self) -> Result<String, Error> {
        Ok(TmplSettings::from(self)
            .render()?
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::App;
    use crate::Result;

    #[tokio::test]
    async fn test_settings_page_rendering() -> Result<()> {
        let app = App::create()?;
        let data_dir = PathBuf::from("/tmp/quiltsync/data");
        let home_dir = Some(PathBuf::from("/home/user/QuiltSync"));

        let view = ViewSettings::create(
            &app,
            &data_dir,
            home_dir,
            "Info".to_string(),
            vec!["open.quilt.bio".to_string()],
        )
        .await?;

        let html = view.render()?;

        assert!(html.contains("Settings"));
        assert!(html.contains("open.quilt.bio"));
        assert!(html.contains("/home/user/QuiltSync"));

        Ok(())
    }
}
