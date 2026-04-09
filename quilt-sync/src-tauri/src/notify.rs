use crate::telemetry::prelude::*;

fn render_success(message: &str) -> String {
    debug!("{}", message);
    format!(r#"<div class="js-success success">{message}</div>"#)
}

fn render_error(message: &str) -> String {
    error!("{}", message);
    format!(r#"<div class="error">{message}</div>"#)
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
            Ok(_) => Ok(render_success(&success_msg)),
            Err(e) => Ok(render_error(&error_fn(&e))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_success() {
        let snapshot = r#"<div class="js-success success">SUCCESS</div>"#;
        let html = render_success("SUCCESS");
        assert_eq!(html, snapshot);
    }

    #[test]
    fn test_error() {
        let snapshot = r#"<div class="error">ERROR</div>"#;
        let html = render_error("ERROR");
        assert_eq!(html, snapshot);
    }
}
