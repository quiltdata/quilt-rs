use askama::Template;
use rust_i18n::t;

use crate::error::Error;
use crate::quilt;
use crate::routes::Paths;
use crate::ui::btn;
use crate::ui::layout::Layout;
use crate::Result;

#[derive(Debug)]
pub struct ViewError {
    err: String,
    title: String,
    login: Option<quilt::uri::Host>,
}

#[derive(Template)]
#[template(path = "./pages/error.html")]
pub struct TmplPageError<'a> {
    dot_quilt: btn::TmplButton<'a>,
    err: String,
    title: String,
    home: btn::TmplButton<'a>,
    login: Option<btn::TmplButton<'a>>,
    reload: btn::TmplButton<'a>,
    layout: Layout<'a>,
}

impl<'a> TmplPageError<'a> {
    pub fn reload_button() -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_js(btn::JsSelector::Refresh)
            .set_label(t!("error.refresh"))
    }

    pub fn dot_quilt_button() -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_js(btn::JsSelector::DotQuilt)
            .set_label(t!("error.dot_quilt"))
    }

    pub fn home_button() -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_href(Paths::InstalledPackagesList)
            .set_color(btn::Color::Primary)
            .set_label(t!("error.home"))
    }

    pub fn login_button(host: quilt::uri::Host) -> btn::TmplButton<'a> {
        btn::TmplButton::builder()
            .set_href(Paths::Login(host))
            .set_label(t!("error.login"))
    }
}

impl From<ViewError> for TmplPageError<'_> {
    fn from(view: ViewError) -> Self {
        TmplPageError {
            dot_quilt: Self::dot_quilt_button(),
            err: view.err,
            title: view.title,
            home: Self::home_button(),
            layout: Layout::new(None),
            login: view.login.map(Self::login_button),
            reload: Self::reload_button(),
        }
    }
}

impl ViewError {
    pub async fn create(err: Error) -> Result<ViewError> {
        let login = match &err {
            Error::Quilt(quilt::Error::Auth(host, _)) => Some(host),
            Error::Quilt(quilt::Error::S3(host, _)) => host.as_ref(),
            _ => None,
        }
        .cloned();
        Ok(ViewError {
            title: t!("error.title").into(),
            err: err.to_string(),
            login,
        })
    }

    pub async fn for_login_error(
        host: quilt::uri::Host,
        title: String,
        error: String,
    ) -> Result<ViewError> {
        Ok(ViewError {
            title,
            err: error,
            login: Some(host),
        })
    }

    pub fn render(self) -> Result<String> {
        let tmpl = TmplPageError::from(self);
        Ok(tmpl.render()?)
    }
}

#[cfg(test)]
pub mod mocks {
    use super::*;

    #[test]
    fn test_view() -> Result {
        let html = TmplPageError::from(ViewError {
            err: "Quilt error: Unimplemented".into(),
            title: "Something went wrong!".into(),
            login: None,
        })
        .render()?;
        let has_error_title = html.contains(&*t!("error.title"));
        let has_error_message =
            html.contains(r#"data-testid="error-msg">Quilt error: Unimplemented"#);
        assert!(has_error_title);
        assert!(has_error_message);
        Ok(())
    }

    #[test]
    fn test_login_error_view() -> Result {
        let host: quilt::uri::Host = "test.quilt.dev".parse().unwrap();
        let html = TmplPageError::from(ViewError {
            err: "Access denied".into(),
            title: "Login failed".into(),
            login: Some(host),
        })
        .render()?;
        assert!(html.contains("Login failed"));
        assert!(html.contains(r#"data-testid="error-msg">Access denied"#));
        Ok(())
    }
}
