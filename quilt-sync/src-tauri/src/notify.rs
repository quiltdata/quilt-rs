use crate::telemetry::prelude::*;

pub struct Notify;

impl Notify {
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(debug_msg: String) -> Self {
        debug!("{}", debug_msg);
        Notify
    }

    #[allow(
        clippy::unused_self,
        reason = "fluent logging helper; `Notify` is a ZST and `self` threads the call chain"
    )]
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
            Ok(_) => {
                debug!("{}", success_msg);
                Ok(success_msg)
            }
            Err(e) => {
                let msg = error_fn(&e);
                error!("{}", msg);
                Err(msg)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_success() {
        let notify = Notify::new("test".to_string());
        let result = notify.map(
            Ok::<(), &str>(()),
            "SUCCESS".to_string(),
            std::string::ToString::to_string,
        );
        assert_eq!(result, Ok("SUCCESS".to_string()));
    }

    #[test]
    fn test_error() {
        let notify = Notify::new("test".to_string());
        let result = notify.map(
            Err::<(), &str>("something broke"),
            "unused".to_string(),
            std::string::ToString::to_string,
        );
        assert_eq!(result, Err("something broke".to_string()));
    }
}
