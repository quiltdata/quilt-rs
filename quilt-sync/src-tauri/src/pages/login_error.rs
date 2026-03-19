use askama::Template;
use rust_i18n::t;

use crate::app::AppAssets;
use crate::app::Globals;
use crate::quilt;
use crate::routes::Paths;
use crate::ui::btn;
use crate::ui::layout::Layout;
use crate::Result;

#[derive(Debug)]
pub struct ViewLoginError {
    globals: Globals,
    host: quilt::uri::Host,
    error: String,
}

#[derive(Template)]
#[template(path = "./pages/login-error.html")]
pub struct TmplPageLoginError<'a> {
    error: String,
    layout: Layout<'a>,
    retry: btn::TmplButton<'a>,
    home: btn::TmplButton<'a>,
}

impl<'a> TmplPageLoginError<'a> {
    pub fn retry_button(host: &quilt::uri::Host) -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_href(Paths::Login(host.clone()))
            .set_color(btn::Color::Primary)
            .set_label(t!("login_error.retry"))
    }

    pub fn home_button() -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_href(Paths::InstalledPackagesList)
            .set_label(t!("login_error.home"))
    }
}

impl From<ViewLoginError> for TmplPageLoginError<'_> {
    fn from(view: ViewLoginError) -> Self {
        TmplPageLoginError {
            error: view.error,
            retry: Self::retry_button(&view.host),
            home: Self::home_button(),
            layout: Layout::new(view.globals, None),
        }
    }
}

impl ViewLoginError {
    pub async fn create(
        app: &impl AppAssets,
        host: quilt::uri::Host,
        error: String,
    ) -> Result<ViewLoginError> {
        Ok(ViewLoginError {
            globals: app.globals(),
            host,
            error,
        })
    }

    pub fn render(self) -> Result<String> {
        Ok(TmplPageLoginError::from(self).render()?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::app::mocks as app_mocks;

    #[test]
    fn test_view() -> Result<()> {
        let app = app_mocks::create();
        let host: quilt::uri::Host = "test.quilt.dev".parse().unwrap();
        let html = TmplPageLoginError::from(ViewLoginError {
            globals: app.globals(),
            host,
            error: "Access denied".to_string(),
        })
        .render()?;
        assert!(html.contains("Login failed"));
        assert!(html.contains(r#"data-testid="error-msg">Access denied"#));
        assert!(html.contains("Try again"));
        Ok(())
    }
}
