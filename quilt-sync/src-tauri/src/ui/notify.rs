use askama::Template;

use crate::telemetry::prelude::*;
use crate::Result;

#[derive(Template)]
#[template(path = "./components/notify-success.html")]
struct TmplNotifySuccess<'a> {
    pub message: &'a str,
}

impl TmplNotifySuccess<'_> {
    fn render(message: &str) -> Result<String> {
        debug!("{}", message);
        Ok(TmplNotifySuccess { message }.render()?)
    }
}

#[derive(Template)]
#[template(path = "./components/notify-error.html")]
struct TmplNotifyError<'a> {
    pub message: &'a str,
}

impl TmplNotifyError<'_> {
    fn render(message: &str) -> Result<String> {
        error!("{}", message);
        Ok(TmplNotifyError { message }.render()?)
    }
}

pub struct TmplNotify;

impl TmplNotify {
    pub fn new(debug_msg: String) -> Self {
        debug!("{}", debug_msg);
        TmplNotify
    }

    pub fn map<T, E: std::fmt::Display, F>(
        self,
        result: std::result::Result<T, E>,
        success_msg: String,
        error_fn: F,
    ) -> std::result::Result<String, String>
    where
        F: FnOnce(&E) -> String,
    {
        match result {
            Ok(_) => TmplNotifySuccess::render(&success_msg).map_err(|err| err.to_string()),
            Err(e) => TmplNotifyError::render(&error_fn(&e)).map_err(|err| err.to_string()),
        }
    }
}

#[cfg(test)]
#[derive(Template)]
#[template(path = "./components/notify.html", whitespace = "suppress")]
pub struct TmplNotifyWrapper {}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::ui::strip_whitespace;

    #[test]
    fn test_success() -> Result {
        let snapshot = String::from(r#"<div class="js-success success">SUCCESS</div>"#);
        let html = TmplNotifySuccess::render("SUCCESS")?;
        assert_eq!(strip_whitespace(html), snapshot);
        Ok(())
    }

    #[test]
    fn test_error() -> Result {
        let snapshot = String::from(r#"<div class="error">ERROR</div>"#);
        let html = TmplNotifyError::render("ERROR")?;
        assert_eq!(strip_whitespace(html), snapshot);
        Ok(())
    }

    #[test]
    fn test_view() -> Result {
        let snapshot = String::from(
            r#"<div class="qui-notify"> <div id="notify" class="root" onclick="var el = this; el.innerHTML = '';"></div> </div>"#,
        );
        let html = (TmplNotifyWrapper {}).render()?;

        assert_eq!(strip_whitespace(html), snapshot);

        Ok(())
    }
}
