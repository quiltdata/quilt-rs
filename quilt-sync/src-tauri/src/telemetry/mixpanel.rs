use std::collections::HashMap;
use std::sync::Arc;

use mixpanel_rs::{Config, Mixpanel};
use serde::Serialize;
use serde_json::Value;

use crate::env;
use crate::error::TelemetryError;
use crate::telemetry::EventContext;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LoginFlow {
    OAuth,
    Legacy,
}

/// The analytics vocabulary — deliberately low-detail: no package names, no
/// paths, no user identity.
///
/// The **host** an event concerns is not a field here. It travels as an
/// argument of the tracking call ([`EventContext`]), so the property is
/// spelled one way for every event, no event can disagree with itself, and a
/// new variant cannot be added without its emitter deciding what its host is.
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "t", content = "c")]
#[serde(rename_all = "snake_case")]
pub enum MixpanelEvent {
    AppLaunched,
    PackagePulled,
    PackagePushed,
    PackageCommitted,
    PackagePublished,
    PackageUninstalled,
    PackageInstalled,
    PackageCreated,
    DirectoryPickerOpened,
    AuthErased,
    DebugDotQuiltOpened,
    DebugLogsOpened,
    FileRevealed,
    FileBrowserOpened,
    DefaultApplicationOpened,
    WebBrowserOpened,
    LatestCertified,
    LocalReset,
    RemoteSet,
    OAuthLoginInitiated,
    RoleSwitched,
    UserLoggedIn { flow: LoginFlow },
    SetupCompleted,
    CrashReportSent,
    DiagnosticLogsSaved,
    QuiltignorePatternAdded,
}

/// The event name and its own properties, read out of the adjacently tagged
/// serialization: `{"t": "event_name"}` or `{"t": "event_name", "c": {…}}`.
fn split_event(event: &MixpanelEvent) -> crate::Result<(String, Option<HashMap<String, Value>>)> {
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

/// The event as Mixpanel receives it: its name, and its own properties with the
/// [`EventContext`] merged in.
pub fn event_payload(
    event: &MixpanelEvent,
    context: &EventContext,
) -> crate::Result<(String, Option<HashMap<String, Value>>)> {
    let (event_name, mut properties) = split_event(event)?;

    if let Some(host) = &context.host {
        properties
            .get_or_insert_with(HashMap::new)
            .insert("host".to_string(), Value::String(host.to_string()));
    }

    Ok((event_name, properties))
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
    context: &EventContext,
) -> crate::Result<()> {
    if let Some(mixpanel) = mixpanel {
        let (event_name, properties) = event_payload(event, context)?;
        mixpanel.track(&event_name, properties).await?;
    }
    Ok(())
}

pub fn init(mixpanel: Option<&Arc<Mixpanel>>) {
    if let Some(mixpanel) = mixpanel {
        let mixpanel_clone = mixpanel.clone();
        tauri::async_runtime::spawn(async move {
            // App launch precedes any host, so it concerns no deployment.
            match event_payload(&MixpanelEvent::AppLaunched, &EventContext::default()) {
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

    fn context() -> EventContext {
        let host = Host::from_str("example.quilt.dev").expect("valid test host");
        EventContext::for_host(Some(&host))
    }

    #[test]
    fn test_all_primitive_events() -> Result {
        for (event, expected) in [
            (MixpanelEvent::AppLaunched, "app_launched"),
            (MixpanelEvent::PackagePulled, "package_pulled"),
            (MixpanelEvent::PackagePushed, "package_pushed"),
            (MixpanelEvent::PackageCommitted, "package_committed"),
            (MixpanelEvent::PackagePublished, "package_published"),
            (MixpanelEvent::PackageUninstalled, "package_uninstalled"),
            (MixpanelEvent::PackageInstalled, "package_installed"),
            (MixpanelEvent::PackageCreated, "package_created"),
            (
                MixpanelEvent::DirectoryPickerOpened,
                "directory_picker_opened",
            ),
            (MixpanelEvent::AuthErased, "auth_erased"),
            (MixpanelEvent::DebugDotQuiltOpened, "debug_dot_quilt_opened"),
            (MixpanelEvent::DebugLogsOpened, "debug_logs_opened"),
            (MixpanelEvent::FileRevealed, "file_revealed"),
            (MixpanelEvent::FileBrowserOpened, "file_browser_opened"),
            (
                MixpanelEvent::DefaultApplicationOpened,
                "default_application_opened",
            ),
            (MixpanelEvent::WebBrowserOpened, "web_browser_opened"),
            (MixpanelEvent::LatestCertified, "latest_certified"),
            (MixpanelEvent::LocalReset, "local_reset"),
            (MixpanelEvent::RemoteSet, "remote_set"),
            // `snake_case` splits the leading acronym, so this event has always
            // been reported under this slightly odd name. Renaming it would
            // break the continuity of its history in the analytics, so the name
            // stays and the test records it.
            (MixpanelEvent::OAuthLoginInitiated, "o_auth_login_initiated"),
            (MixpanelEvent::RoleSwitched, "role_switched"),
            (MixpanelEvent::SetupCompleted, "setup_completed"),
            (MixpanelEvent::CrashReportSent, "crash_report_sent"),
            (MixpanelEvent::DiagnosticLogsSaved, "diagnostic_logs_saved"),
            (
                MixpanelEvent::QuiltignorePatternAdded,
                "quiltignore_pattern_added",
            ),
        ] {
            let (name, props) = event_payload(&event, &EventContext::default())?;
            assert_eq!(name, expected);
            assert!(props.is_none(), "{expected} must carry no properties");
        }

        Ok(())
    }

    /// An event with no properties of its own gains `host` alone.
    #[test]
    fn test_host_is_merged_into_a_bare_event() -> Result {
        let (name, props) = event_payload(&MixpanelEvent::PackagePulled, &context())?;

        assert_eq!(name, "package_pulled");
        let props = props.expect("host must create the property map");
        assert_eq!(
            props.get("host"),
            Some(&Value::String("example.quilt.dev".to_string()))
        );
        assert_eq!(props.len(), 1);

        Ok(())
    }

    /// No host named means no host property — never an inherited one.
    #[test]
    fn test_hostless_event_carries_no_host() -> Result {
        let (name, props) = event_payload(
            &MixpanelEvent::DirectoryPickerOpened,
            &EventContext::default(),
        )?;

        assert_eq!(name, "directory_picker_opened");
        assert!(props.is_none());

        Ok(())
    }

    /// The wire shape a login event sent while `host` was its own field:
    /// `host` beside `flow`, same name, same string value.
    #[test]
    fn test_user_logged_in_keeps_its_wire_shape() -> Result {
        let event = MixpanelEvent::UserLoggedIn {
            flow: LoginFlow::OAuth,
        };

        let (name, props) = event_payload(&event, &context())?;

        assert_eq!(name, "user_logged_in");
        let props = props.expect("flow and host");
        assert_eq!(
            props.get("host"),
            Some(&Value::String("example.quilt.dev".to_string()))
        );
        assert_eq!(props.get("flow"), Some(&Value::String("oauth".to_string())));

        Ok(())
    }

    #[test]
    fn test_role_switched_carries_the_host_it_switched_on() -> Result {
        let (name, props) = event_payload(&MixpanelEvent::RoleSwitched, &context())?;

        assert_eq!(name, "role_switched");
        assert_eq!(
            props.expect("host").get("host"),
            Some(&Value::String("example.quilt.dev".to_string()))
        );

        Ok(())
    }
}
