use crate::telemetry::prelude::*;
use crate::telemetry::{MixpanelEvent, Telemetry};

// TODO: replace this fluent helper with an extension trait on `Result` —
// `trait NotifyResult<E> { fn notify(self, ok_msg: String, err_fn: impl FnOnce(&E) -> String) -> Result<String, String>; }`
// implemented for `Result<T, E>`, logging the init line explicitly at the call
// site (`debug!("{msg_init}"); op().await.notify(ok, err)`). This drops the
// fluent receiver, at the cost of splitting the ~25 one-liner call sites into an
// explicit `debug!` + `.notify(...)`. The receiver exists only to log init before
// the op runs (receiver evaluated before args); the trait moves that ordering
// into an explicit line. Whatever replaces it has to keep [`Notify::on_success`]'s
// property: an event registered before the operation can only fire after it.
pub struct Notify<'a> {
    /// The event to report, and where — attached before the operation runs and
    /// emitted only if it succeeded.
    ///
    /// This is the whole reason telemetry belongs here rather than at the call
    /// site. The helper is the one place that already knows whether the operation
    /// worked, so registering an event with it makes "an event means the thing
    /// happened" **structural**: a site cannot emit early by mistake, because the
    /// only path to emission runs through the success arm. Before, timing was a
    /// per-site accident and most sites got it wrong.
    on_success: Option<(&'a Telemetry, MixpanelEvent)>,
}

impl<'a> Notify<'a> {
    #[allow(clippy::needless_pass_by_value)]
    pub fn new(debug_msg: String) -> Self {
        debug!("{}", debug_msg);
        Self { on_success: None }
    }

    /// Report `event` if — and only if — the operation succeeds.
    ///
    /// Registered *before* the operation runs, which is what keeps the call site
    /// reading in the order things happen, while emission still waits for the
    /// outcome. Emitting is a queue push, so the success arm pays nothing for it.
    #[must_use]
    pub fn on_success(mut self, telemetry: &'a Telemetry, event: MixpanelEvent) -> Self {
        self.on_success = Some((telemetry, event));
        self
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
                if let Some((telemetry, event)) = self.on_success {
                    telemetry.track(event);
                }
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

    /// The property the whole unit exists for: a registered event reaches the sink
    /// when the operation succeeded.
    #[test]
    fn a_registered_event_is_reported_on_success() {
        let telemetry = Telemetry::default();

        let _ = Notify::new("doing it".to_string())
            .on_success(&telemetry, MixpanelEvent::SetupCompleted)
            .map(
                Ok::<(), &str>(()),
                "done".to_string(),
                std::string::ToString::to_string,
            );

        assert_eq!(
            telemetry.queued_events(),
            vec!["setup_completed".to_owned()]
        );
    }

    /// And the half that was the actual defect: it does **not** reach the sink when
    /// the operation failed. Most sites used to emit before even attempting, so
    /// every count conflated attempts with successes.
    #[test]
    fn a_registered_event_is_not_reported_on_failure() {
        let telemetry = Telemetry::default();

        let _ = Notify::new("doing it".to_string())
            .on_success(&telemetry, MixpanelEvent::SetupCompleted)
            .map(
                Err::<(), &str>("refused"),
                "unused".to_string(),
                std::string::ToString::to_string,
            );

        assert!(
            telemetry.queued_events().is_empty(),
            "a failed operation reported an event: {:?}",
            telemetry.queued_events()
        );
    }
}
