use crate::telemetry::prelude::*;

pub struct Notify;

impl Notify {
    pub fn new(debug_msg: String) -> Self {
        debug!("{}", debug_msg);
        Notify
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
        let result = notify.map(Ok::<(), &str>(()), "SUCCESS".to_string(), |e| e.to_string());
        assert_eq!(result, Ok("SUCCESS".to_string()));
    }

    #[test]
    fn test_error() {
        let notify = Notify::new("test".to_string());
        let result = notify.map(
            Err::<(), &str>("something broke"),
            "unused".to_string(),
            |e| e.to_string(),
        );
        assert_eq!(result, Err("something broke".to_string()));
    }
}
