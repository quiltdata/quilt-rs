//! Sending events to Mixpanel. *What* we report lives in
//! [`event`](super::event).

use std::collections::HashMap;
use std::sync::Arc;

use mixpanel_rs::{Config, Mixpanel};
use serde_json::Value;

use crate::env;
use crate::error::TelemetryError;
use crate::telemetry::MixpanelEvent;

/// The event as Mixpanel receives it: its name, and its properties.
///
/// Both come out of the adjacently tagged serialization — `{"t": "event_name"}`
/// for an event with no payload, `{"t": "event_name", "c": {…}}` for one with a
/// payload. Nothing is merged in here: an event that concerns a deployment
/// carries the host in its own payload, so serde has already put it in `c`.
pub fn event_payload(
    event: &MixpanelEvent,
) -> crate::Result<(String, Option<HashMap<String, Value>>)> {
    let Value::Object(map) = serde_json::to_value(event)? else {
        // Unreachable with adjacently tagged serialization.
        return Err(TelemetryError::Serialize(
            "Expected object from adjacently tagged serialization".to_string(),
        )
        .into());
    };

    let Some(Value::String(event_name)) = map.get("t") else {
        return Err(TelemetryError::Serialize("Failed to serialize event name".to_string()).into());
    };

    let properties = map
        .get("c")
        .and_then(|v| v.as_object())
        .map(|obj| obj.clone().into_iter().collect());

    Ok((event_name.clone(), properties))
}

pub fn mixpanel_config() -> Option<(String, Config)> {
    let token = env::mixpanel_project_token();
    let secret = env::mixpanel_api_secret();
    if token.is_none() {
        eprintln!("No MIXPANEL_PROJECT_TOKEN configured, Mixpanel disabled");
    }
    if secret.is_none() {
        eprintln!("No MIXPANEL_API_SECRET configured");
    }
    token.map(|token| {
        let config = Config {
            secret,
            debug: cfg!(debug_assertions),
            ..Default::default()
        };
        (token, config)
    })
}

pub async fn track_event(
    mixpanel: Option<&Arc<Mixpanel>>,
    event: &MixpanelEvent,
) -> crate::Result<()> {
    if let Some(mixpanel) = mixpanel {
        let (event_name, properties) = event_payload(event)?;
        mixpanel.track(&event_name, properties).await?;
    }
    Ok(())
}

pub fn init(mixpanel: Option<&Arc<Mixpanel>>) {
    if let Some(mixpanel) = mixpanel {
        let mixpanel_clone = mixpanel.clone();
        tauri::async_runtime::spawn(async move {
            // App launch precedes any host, so it carries no payload at all.
            match event_payload(&MixpanelEvent::AppLaunched) {
                Ok((event_name, properties)) => {
                    if let Err(err) = mixpanel_clone.track(&event_name, properties).await {
                        eprintln!("Failed to track app launch: {err}");
                    }
                }
                Err(err) => {
                    eprintln!("Failed to serialize app launch event: {err}");
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use std::str::FromStr;

    use quilt_uri::Host;

    use super::*;
    use crate::Result;
    use crate::telemetry::event::{
        AuthEvent, LoginEvent, LoginFlow, PackageEvent, PackageFileEvent, RemotePackageEvent,
    };

    fn host() -> Host {
        Host::from_str("example.quilt.dev").expect("valid test host")
    }

    fn host_value() -> Value {
        Value::String("example.quilt.dev".to_string())
    }

    /// The events that carry no payload: name only, no properties object.
    #[test]
    fn test_events_without_a_payload() -> Result {
        for (event, expected) in [
            (MixpanelEvent::AppLaunched, "app_launched"),
            (MixpanelEvent::SetupCompleted, "setup_completed"),
            (
                MixpanelEvent::DirectoryPickerOpened,
                "directory_picker_opened",
            ),
            (MixpanelEvent::HomeDirOpened, "home_dir_opened"),
            (MixpanelEvent::WebBrowserOpened, "web_browser_opened"),
            (MixpanelEvent::DebugDotQuiltOpened, "debug_dot_quilt_opened"),
            (MixpanelEvent::DebugLogsOpened, "debug_logs_opened"),
            (MixpanelEvent::DataDirOpened, "data_dir_opened"),
            (MixpanelEvent::DiagnosticLogsSaved, "diagnostic_logs_saved"),
            (MixpanelEvent::CrashReportSent, "crash_report_sent"),
        ] {
            let (name, props) = event_payload(&event)?;
            assert_eq!(name, expected);
            assert!(props.is_none(), "{expected} must carry no properties");
        }

        Ok(())
    }

    /// A remote operation reports the deployment it ran against.
    #[test]
    fn test_remote_package_event_reports_its_host() -> Result {
        let event = MixpanelEvent::PackagePulled(RemotePackageEvent::for_host(Some(host())));

        let (name, props) = event_payload(&event)?;

        assert_eq!(name, "package_pulled");
        let props = props.expect("host");
        assert_eq!(props.get("host"), Some(&host_value()));
        assert_eq!(props.len(), 1);

        Ok(())
    }

    /// Every operation whose host the caller supplies must still emit when the
    /// caller supplied none — an unattributed event, never a dropped one.
    #[test]
    fn test_caller_supplied_events_emit_without_a_host() -> Result {
        for (event, expected) in [
            (
                MixpanelEvent::PackagePushed(RemotePackageEvent::for_uri(None)),
                "package_pushed",
            ),
            (
                MixpanelEvent::PackageCommitted(PackageEvent::for_uri(None)),
                "package_committed",
            ),
            (
                MixpanelEvent::PackageCreated(PackageEvent::hostless()),
                "package_created",
            ),
            (
                MixpanelEvent::FileRevealed(PackageFileEvent::for_uri(None)),
                "file_revealed",
            ),
        ] {
            let (name, props) = event_payload(&event)?;
            assert_eq!(name, expected);
            assert_eq!(
                props.expect("payload").get("host"),
                Some(&Value::Null),
                "{expected} must report a null host, not omit the property"
            );
        }

        Ok(())
    }

    /// The wire form a login event sent when `host` was a field of the event
    /// itself: `host` beside `flow`, same names, same string value.
    #[test]
    fn test_user_logged_in_keeps_its_wire_shape() -> Result {
        let event = MixpanelEvent::UserLoggedIn(LoginEvent {
            host: host(),
            flow: LoginFlow::OAuth,
        });

        let (name, props) = event_payload(&event)?;

        assert_eq!(name, "user_logged_in");
        let props = props.expect("host and flow");
        assert_eq!(props.get("host"), Some(&host_value()));
        assert_eq!(props.get("flow"), Some(&Value::String("oauth".to_string())));

        Ok(())
    }

    #[test]
    fn test_auth_events_report_a_guaranteed_host() -> Result {
        for (event, expected) in [
            (
                MixpanelEvent::RoleSwitched(AuthEvent { host: host() }),
                "role_switched",
            ),
            // `snake_case` splits the leading acronym, so this event has always
            // been reported under this slightly odd name. Renaming it would break
            // the continuity of its history, so the name stays and this records it.
            (
                MixpanelEvent::OAuthLoginInitiated(AuthEvent { host: host() }),
                "o_auth_login_initiated",
            ),
            (
                MixpanelEvent::AuthErased(AuthEvent { host: host() }),
                "auth_erased",
            ),
        ] {
            let (name, props) = event_payload(&event)?;
            assert_eq!(name, expected);
            assert_eq!(props.expect("host").get("host"), Some(&host_value()));
        }

        Ok(())
    }

    /// The accessor the crash-report cell reads. Exhaustive by construction —
    /// the compiler rejects a new variant that does not answer this.
    #[test]
    fn test_host_accessor_matches_the_payload() {
        assert_eq!(
            MixpanelEvent::PackagePulled(RemotePackageEvent::for_host(Some(host()))).host(),
            Some(&host())
        );
        assert_eq!(
            MixpanelEvent::AuthErased(AuthEvent { host: host() }).host(),
            Some(&host())
        );
        assert_eq!(MixpanelEvent::AppLaunched.host(), None);
        assert_eq!(
            MixpanelEvent::PackageCommitted(PackageEvent::for_uri(None)).host(),
            None
        );
    }
}
