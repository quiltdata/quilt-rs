use std::borrow::Cow;

use askama::Template;
use rust_i18n::t;

use crate::error::Error;
use crate::quilt;
use crate::quilt::uri::Host;
use crate::ui::btn;
use crate::ui::layout::Layout;
use crate::ui::Icon;

#[derive(Debug)]
pub struct ViewLogin {
    host: Host,
    back: Option<String>,
}

#[derive(Template)]
#[template(path = "./pages/login.html")]
pub struct TmplPageLogin<'a> {
    host: Host,
    instructions: Cow<'a, str>,
    layout: Layout<'a>,
    back: Option<String>,
    login_oauth: btn::TmplButton<'a>,
    open_catalog: btn::TmplButton<'a>,
}

impl<'a> TmplPageLogin<'a> {
    pub fn submit_button() -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_js(btn::JsSelector::Login)
            .set_size(btn::Size::Large)
            .set_color(btn::Color::Primary)
            .set_data("form", "#form")
            .set_label(t!("login.submit"))
            .set_icon(Icon::Done)
    }

    pub fn open_catalog(host: &Host) -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_js(btn::JsSelector::OpenInWebBrowser)
            .set_icon(Icon::OpenInBrowser)
            .set_data("url", format!("https://{host}/code"))
            .set_label(t!("login.open_browser"))
    }

    pub fn login_oauth(host: &Host, back: Option<&str>) -> btn::TmplButton<'a> {
        let btn = btn::TmplButton::builder()
            .set_js(btn::JsSelector::LoginOAuth)
            .set_size(btn::Size::Large)
            .set_color(btn::Color::Primary)
            .set_icon(Icon::OpenInBrowser)
            .set_data("host", host.to_string())
            .set_label(t!("login.login_oauth"));
        match back {
            Some(loc) => btn.set_data("back", loc.to_string()),
            None => btn,
        }
    }
}

impl From<ViewLogin> for TmplPageLogin<'_> {
    fn from(view: ViewLogin) -> Self {
        TmplPageLogin {
            instructions: t!("login.code_instruction", s => view.host.to_string()),
            layout: Layout::new(Some(Self::submit_button())),
            login_oauth: Self::login_oauth(&view.host, view.back.as_deref()),
            open_catalog: Self::open_catalog(&view.host),
            back: view.back,
            host: view.host,
        }
    }
}

impl ViewLogin {
    pub async fn create(
        tracing: &crate::telemetry::Telemetry,
        host: quilt::uri::Host,
        back: Option<String>,
    ) -> Result<ViewLogin, Error> {
        tracing.add_host(&host);
        Ok(ViewLogin { host, back })
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

    use crate::Result;

    #[test]
    fn test_login_page_rendering() -> Result {
        let host: Host = "test.quilt.dev".parse()?;

        let view = ViewLogin {
            host: host.clone(),
            back: Some("installed-packages-list.html".to_string()),
        };

        let html = view.render()?;

        // Check for OAuth login button with back
        assert!(html.contains(r#"js-login-oauth"#));
        assert!(html.contains(&format!(r#"data-host="{host}""#)));
        assert!(html.contains(r#"data-back="installed-packages-list.html"#));

        // Check for code input form
        assert!(html.contains(r#"id="code""#));
        assert!(html.contains(r#"js-login"#));
        assert!(html.contains(r##"data-form="#form""##));

        // Check for open browser button
        assert!(html.contains(r#"js-open-in-web-browser"#));
        assert!(html.contains(&format!(r#"data-url="https://{host}/code""#)));

        // Check for instructions text
        assert!(html.contains(&format!("Or visit https://{host}/code to get your code:")));

        Ok(())
    }

    #[test]
    fn test_login_oauth_button_with_back() -> Result {
        let host: Host = "test.quilt.dev".parse()?;
        let btn = TmplPageLogin::login_oauth(&host, Some("some-page.html"));
        let html = btn.to_string();

        assert!(html.contains(r#"js-login-oauth"#));
        assert!(html.contains(r#"data-host="test.quilt.dev""#));
        assert!(html.contains(r#"data-back="some-page.html""#));
        assert!(html.contains(r#"primary"#));
        assert!(html.contains(r#"large"#));

        Ok(())
    }

    #[test]
    fn test_login_oauth_button_without_back() -> Result {
        let host: Host = "test.quilt.dev".parse()?;
        let btn = TmplPageLogin::login_oauth(&host, None);
        let html = btn.to_string();

        assert!(html.contains(r#"js-login-oauth"#));
        assert!(html.contains(r#"data-host="test.quilt.dev""#));
        assert!(!html.contains(r#"data-back"#));

        Ok(())
    }

    #[test]
    fn test_submit_button() -> Result {
        let btn = TmplPageLogin::submit_button();
        let html = btn.to_string();

        assert!(html.contains(r#"js-login"#));
        assert!(html.contains(r##"data-form="#form""##));
        assert!(html.contains(r#"primary"#));
        assert!(html.contains(r#"large"#));

        Ok(())
    }

    #[test]
    fn test_open_catalog_button() -> Result {
        let host: Host = "test.quilt.dev".parse()?;
        let btn = TmplPageLogin::open_catalog(&host);
        let html = btn.to_string();

        assert!(html.contains(r#"js-open-in-web-browser"#));
        assert!(html.contains(r#"data-url="https://test.quilt.dev/code""#));

        Ok(())
    }
}
