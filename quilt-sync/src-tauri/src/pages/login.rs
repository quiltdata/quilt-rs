use std::borrow::Cow;

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
    host: Host,
    instructions: Cow<'a, str>,
    layout: Layout<'a>,
    location: Option<String>,
    open_catalog: btn::TmplButton<'a>,
}

impl<'a> TmplPageLogin<'a> {
    pub fn primary_button() -> btn::TmplButton<'a> {
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
}

impl From<ViewLogin> for TmplPageLogin<'_> {
    fn from(view: ViewLogin) -> Self {
        TmplPageLogin {
            instructions: t!("login.code_instruction", s => view.host.to_string()),
            layout: Layout::new(view.globals, Some(Self::primary_button())),
            location: view.location,
            open_catalog: Self::open_catalog(&view.host),
            host: view.host,
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
        // Create a test host
        let host: Host = "test.quilt.dev".parse()?;

        // Create the view
        let view = ViewLogin {
            globals: Globals::default(),
            host: host.clone(),
            location: Some("installed-packages-list.html".to_string()),
        };

        // Render the view to HTML
        let html = view.render()?;

        // Check for input field for code
        assert!(html.contains(r#"<input class="input" id="code" name="code" required />"#));

        // Check for login button with js-login selector
        assert!(html.contains(r#"js-login"#));
        assert!(html.contains(r##"data-form="#form""##));

        // Check for open browser button
        assert!(html.contains(r#"js-open-in-web-browser"#));
        assert!(html.contains(&format!(r#"data-url="https://{host}/code""#)));

        // Check for instructions text
        assert!(html.contains(&format!(
            "Please, visit https://{host}/code to get your code:"
        )));

        Ok(())
    }

    #[test]
    fn test_login_buttons() -> Result {
        // Test primary button (login button)
        let primary_button = TmplPageLogin::primary_button();
        let primary_html = primary_button.to_string();

        assert!(primary_html.contains(r#"js-login"#));
        assert!(primary_html.contains(r##"data-form="#form""##));
        assert!(primary_html.contains(r#"primary"#));
        assert!(primary_html.contains(r#"large"#));

        // Test open catalog button
        let host: Host = "test.quilt.dev".parse()?;
        let open_catalog_button = TmplPageLogin::open_catalog(&host);
        let open_catalog_html = open_catalog_button.to_string();

        assert!(open_catalog_html.contains(r#"js-open-in-web-browser"#));
        assert!(open_catalog_html.contains(r#"data-url="https://test.quilt.dev/code""#));
        assert!(open_catalog_html.contains(r#"open_in_browser"#));

        Ok(())
    }
}
