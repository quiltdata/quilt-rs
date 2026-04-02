use std::collections::HashMap;
use std::sync::Arc;

use mixpanel_rs::{Config, Mixpanel};
use serde::Serialize;
use serde_json::Value;

use crate::env;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum LoginFlow {
    OAuth,
    Legacy,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "t", content = "c")]
#[serde(rename_all = "snake_case")]
pub enum MixpanelEvent {
    AppLaunched,
    PageLoaded {
        pathname: String,
        error: Option<String>,
    },
    PackagePulled,
    PackagePushed,
    PackageCommitted,
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
    OriginSet,
    RemoteSet,
    OAuthLoginInitiated {
        host: String,
    },
    UserLoggedIn {
        host: String,
        flow: LoginFlow,
    },
    SetupCompleted,
    CrashReportSent,
    DiagnosticLogsSaved,
    QuiltignorePatternAdded,
    ErrorOccurred {
        error_type: String,
    },
}

impl TryFrom<MixpanelEvent> for (String, Option<HashMap<String, Value>>) {
    type Error = crate::Error;

    fn try_from(event: MixpanelEvent) -> Result<Self, Self::Error> {
        // Use adjacently tagged serde serialization to extract event name and properties
        let serialized = serde_json::to_value(&event)?;

        match serialized {
            Value::Object(map) => {
                // Adjacently tagged format: {"t": "event_name", "c": {...}} or {"t": "event_name"}
                let Some(Value::String(event_name)) = map.get("t") else {
                    return Err(crate::Error::MixpanelSer(
                        "Failed to serialize event name".to_string(),
                    ));
                };

                let properties = map
                    .get("c")
                    .and_then(|v| v.as_object())
                    .map(|obj| obj.clone().into_iter().collect());

                Ok((event_name.clone(), properties))
            }
            _ => {
                // This should not happen with adjacently tagged serialization
                Err(crate::Error::MixpanelSer(
                    "Expected object from adjacently tagged serialization".to_string(),
                ))
            }
        }
    }
}

pub fn mixpanel_config() -> Option<(String, Config)> {
    let token = env::mixpanel_project_token();
    let secret = env::mixpanel_api_secret();
    if token.is_none() {
        eprintln!("No MIXPANEL_PROJECT_TOKEN configured, Mixpanel disabled");
    };
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
    mixpanel: &Option<Arc<Mixpanel>>,
    event: &MixpanelEvent,
) -> crate::Result<()> {
    if let Some(ref mixpanel) = mixpanel {
        let (event_name, properties) = event.clone().try_into()?;
        mixpanel.track(&event_name, properties).await?;
    }
    Ok(())
}

pub fn init(mixpanel: &Option<Arc<Mixpanel>>) {
    if let Some(ref mixpanel) = mixpanel {
        let mixpanel_clone = mixpanel.clone();
        tauri::async_runtime::spawn(async move {
            match MixpanelEvent::AppLaunched.try_into() {
                Ok((event_name, properties)) => {
                    if let Err(err) = mixpanel_clone.track(&event_name, properties).await {
                        eprintln!("Failed to track app launch: {}", err);
                    }
                }
                Err(err) => {
                    eprintln!("Failed to serialize app launch event: {}", err);
                }
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Error;
    use crate::Result;

    #[test]
    fn test_all_primitive_events() -> Result {
        let (name, p) = MixpanelEvent::AppLaunched.try_into()?;
        assert_eq!(name, "app_launched");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::PackagePulled.try_into()?;
        assert_eq!(name, "package_pulled");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::PackagePushed.try_into()?;
        assert_eq!(name, "package_pushed");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::PackageCommitted.try_into()?;
        assert_eq!(name, "package_committed");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::PackageUninstalled.try_into()?;
        assert_eq!(name, "package_uninstalled");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::PackageInstalled.try_into()?;
        assert_eq!(name, "package_installed");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::PackageCreated.try_into()?;
        assert_eq!(name, "package_created");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::DirectoryPickerOpened.try_into()?;
        assert_eq!(name, "directory_picker_opened");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::AuthErased.try_into()?;
        assert_eq!(name, "auth_erased");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::DebugDotQuiltOpened.try_into()?;
        assert_eq!(name, "debug_dot_quilt_opened");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::DebugLogsOpened.try_into()?;
        assert_eq!(name, "debug_logs_opened");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::FileRevealed.try_into()?;
        assert_eq!(name, "file_revealed");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::FileBrowserOpened.try_into()?;
        assert_eq!(name, "file_browser_opened");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::DefaultApplicationOpened.try_into()?;
        assert_eq!(name, "default_application_opened");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::WebBrowserOpened.try_into()?;
        assert_eq!(name, "web_browser_opened");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::LatestCertified.try_into()?;
        assert_eq!(name, "latest_certified");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::LocalReset.try_into()?;
        assert_eq!(name, "local_reset");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::OriginSet.try_into()?;
        assert_eq!(name, "origin_set");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::RemoteSet.try_into()?;
        assert_eq!(name, "remote_set");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::SetupCompleted.try_into()?;
        assert_eq!(name, "setup_completed");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::CrashReportSent.try_into()?;
        assert_eq!(name, "crash_report_sent");
        assert!(p.is_none());

        let (name, p) = MixpanelEvent::DiagnosticLogsSaved.try_into()?;
        assert_eq!(name, "diagnostic_logs_saved");
        assert!(p.is_none());

        Ok(())
    }

    #[test]
    fn test_page_loaded_event() -> Result {
        // Test PageLoaded with no error
        let event = MixpanelEvent::PageLoaded {
            pathname: "commit".to_string(),
            error: None,
        };
        match event.try_into()? {
            (name, Some(props)) => {
                assert_eq!(name, "page_loaded");
                assert_eq!(
                    props.get("pathname"),
                    Some(&Value::String("commit".to_string()))
                );
                assert_eq!(props.get("error"), Some(&Value::Null));
            }
            _ => return Err(Error::MixpanelSer("PageLoaded with no error".to_string())),
        }

        // Test PageLoaded with error
        let event = MixpanelEvent::PageLoaded {
            pathname: "login".to_string(),
            error: Some("Authentication failed".to_string()),
        };
        match event.try_into()? {
            (name, Some(props)) => {
                assert_eq!(name, "page_loaded");
                assert_eq!(
                    props.get("pathname"),
                    Some(&Value::String("login".to_string()))
                );
                assert_eq!(
                    props.get("error"),
                    Some(&Value::String("Authentication failed".to_string()))
                );
                Ok(())
            }
            _ => Err(Error::MixpanelSer("PageLoaded with error".to_string())),
        }
    }

    #[test]
    fn test_user_logged_in_event() -> Result {
        let event = MixpanelEvent::UserLoggedIn {
            host: "example.quilt.dev".to_string(),
            flow: LoginFlow::OAuth,
        };
        match event.try_into()? {
            (name, Some(props)) => {
                assert_eq!(name, "user_logged_in");
                assert_eq!(
                    props.get("host"),
                    Some(&Value::String("example.quilt.dev".to_string()))
                );
                assert_eq!(props.get("flow"), Some(&Value::String("oauth".to_string())));
                Ok(())
            }
            _ => Err(Error::MixpanelSer("UserLoggedIn".to_string())),
        }
    }

    #[test]
    fn test_error_occurred_event() -> Result {
        let event = MixpanelEvent::ErrorOccurred {
            error_type: "network_timeout".to_string(),
        };
        match event.try_into()? {
            (name, Some(props)) => {
                assert_eq!(name, "error_occurred");
                assert_eq!(
                    props.get("error_type"),
                    Some(&Value::String("network_timeout".to_string()))
                );
                Ok(())
            }
            _ => Err(Error::MixpanelSer("ErrorOccurred".to_string())),
        }
    }
}
