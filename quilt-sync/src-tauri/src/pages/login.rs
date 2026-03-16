use askama::Template;
use rust_i18n::t;

use crate::app::AppAssets;
use crate::app::Globals;
use crate::error::Error;
use crate::quilt;
use crate::quilt::uri::Host;
use crate::ui::btn;
use crate::ui::layout::Layout;
use crate::ui::Icon;

#[derive(Debug)]
pub struct ViewLogin {
    globals: Globals,
    host: Host,
    location: Option<String>,
}

#[derive(Template)]
#[template(path = "./pages/login.html")]
pub struct TmplPageLogin<'a> {
    login_oauth: btn::TmplButton<'a>,
    layout: Layout<'a>,
}

impl<'a> TmplPageLogin<'a> {
    pub fn login_oauth(host: &Host, location: Option<&str>) -> btn::TmplButton<'a> {
        let btn = btn::TmplButton::builder()
            .set_js(btn::JsSelector::LoginOAuth)
            .set_size(btn::Size::Large)
            .set_color(btn::Color::Primary)
            .set_icon(Icon::OpenInBrowser)
            .set_data("host", host.to_string())
            .set_label(t!("login.login_oauth"));
        match location {
            Some(loc) => btn.set_data("location", loc.to_string()),
            None => btn,
        }
    }
}

impl From<ViewLogin> for TmplPageLogin<'_> {
    fn from(view: ViewLogin) -> Self {
        TmplPageLogin {
            login_oauth: Self::login_oauth(&view.host, view.location.as_deref()),
            layout: Layout::new(view.globals, None),
        }
    }
}

impl ViewLogin {
    pub async fn create(
        app: &impl AppAssets,
        tracing: &crate::telemetry::Telemetry,
        host: quilt::uri::Host,
        location: Option<String>,
    ) -> Result<ViewLogin, Error> {
        tracing.add_host(&host);
        Ok(ViewLogin {
            globals: app.globals(),
            host,
            location,
        })
    }

    pub fn render(self) -> Result<String, Error> {
        Ok(TmplPageLogin::from(self)
            .render()?
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" "))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use quilt::uri::Host;

    use crate::app::Globals;
    use crate::Result;

    #[test]
    fn test_login_page_rendering() -> Result {
        let host: Host = "test.quilt.dev".parse()?;

        let view = ViewLogin {
            globals: Globals::default(),
            host: host.clone(),
            location: Some("installed-packages-list.html".to_string()),
        };

        let html = view.render()?;

        // Check for OAuth login button
        assert!(html.contains(r#"js-login-oauth"#));
        assert!(html.contains(&format!(r#"data-host="{host}""#)));
        assert!(html.contains(r#"data-location="installed-packages-list.html""#));

        // Ensure paste-code form is gone
        assert!(!html.contains(r#"id="code""#));
        assert!(!html.contains(r#"class="qui-button js-login""#));

        Ok(())
    }

    #[test]
    fn test_login_oauth_button_with_location() -> Result {
        let host: Host = "test.quilt.dev".parse()?;
        let btn = TmplPageLogin::login_oauth(&host, Some("some-page.html"));
        let html = btn.to_string();

        assert!(html.contains(r#"js-login-oauth"#));
        assert!(html.contains(r#"data-host="test.quilt.dev""#));
        assert!(html.contains(r#"data-location="some-page.html""#));
        assert!(html.contains(r#"primary"#));
        assert!(html.contains(r#"large"#));

        Ok(())
    }

    #[test]
    fn test_login_oauth_button_without_location() -> Result {
        let host: Host = "test.quilt.dev".parse()?;
        let btn = TmplPageLogin::login_oauth(&host, None);
        let html = btn.to_string();

        assert!(html.contains(r#"js-login-oauth"#));
        assert!(html.contains(r#"data-host="test.quilt.dev""#));
        assert!(!html.contains(r#"data-location"#));

        Ok(())
    }
}
